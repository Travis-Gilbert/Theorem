//! RustyRed code-workspace import seam.
//!
//! W0 turns a git checkout into files inside the embedded `DocTree`
//! workspace. The importer walks the real checkout with gitignore awareness,
//! filters build artifacts, batches the source files through `Engine`, and
//! leaves later units to maintain the code graph and execution bridge.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::mpsc::Sender;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use notify_debouncer_full::notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use rustyred_embedded::{
    DirectoryRemoveDisposition, EmbeddedConfig, Engine, EngineError, FileWrite, FileWriteReceipt,
};
use rustyred_thg_code::{
    index_source_file_write_in_store, stage_repo_for_ingest_with_credential, FetchedRepo,
    GitCredential, IngestCodebaseInput, RepoFetchCaps, SourceFileWriteIndexInput,
};
use rustyred_thg_core::{NodeQuery, DOC_TREE_CONTENT_HASH_PROPERTY, DOC_TREE_PATH_PROPERTY};
use theorem_receiver::{
    build_spawn_plan, ProofPlan, ProofReceipt, SandboxCancelToken, SandboxFile,
    SandboxProvisionRequest, SandboxRuntime, SandboxStreamEvent, SpawnPlan,
};

const DEFAULT_MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
pub const DEFAULT_MIRROR_DEBOUNCE: Duration = Duration::from_millis(200);
pub const DEFAULT_CONVERGENCE_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_MIRROR_AUDIT_INTERVAL: Duration = Duration::from_secs(30);
const LOCAL_WORKSPACE_TENANT: &str = "local";

/// Import controls for a checkout-to-DocTree batch.
#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// Optional destination prefix inside the engine workspace.
    pub prefix: String,
    /// Maximum size of one imported file.
    pub max_file_bytes: u64,
    /// Maximum total imported source bytes.
    pub max_total_bytes: u64,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            prefix: String::new(),
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
        }
    }
}

/// Summary of one workspace import.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportReceipt {
    pub root: PathBuf,
    pub files_imported: usize,
    pub files_skipped: usize,
    pub bytes_imported: u64,
    pub paths: Vec<String>,
    pub clone_ms: u64,
}

/// Summary of projecting DocTree files into a real OS directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializeReceipt {
    pub root: PathBuf,
    pub files_written: usize,
    pub bytes_written: u64,
    pub paths: Vec<String>,
}

/// Summary of syncing real OS source files back into the DocTree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncBackReceipt {
    pub root: PathBuf,
    pub files_synced: usize,
    pub files_skipped: usize,
    pub bytes_synced: u64,
    pub paths: Vec<String>,
    pub code_symbols_indexed: u64,
    pub code_edges_indexed: u64,
    pub code_edges_retired: u64,
    pub code_bucket_lookups: u64,
}

/// Import controls for a real filesystem mirror.
#[derive(Clone, Debug)]
pub struct MirrorImportOptions {
    /// Optional destination prefix inside the engine DocTree/index namespace.
    pub prefix: String,
    /// Optional workspace root. When absent, a local checkout is used in place
    /// and a URL import keeps the staged clone as the mirror root.
    pub workspace_root: Option<PathBuf>,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for MirrorImportOptions {
    fn default() -> Self {
        Self {
            prefix: String::new(),
            workspace_root: None,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MirrorImportReceipt {
    pub root: PathBuf,
    pub files_mirrored: usize,
    pub files_skipped: usize,
    pub bytes_mirrored: u64,
    pub paths: Vec<String>,
    pub index: IndexReceipt,
    pub clone_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexReceipt {
    pub root: PathBuf,
    pub files_indexed: usize,
    pub files_removed: usize,
    pub files_skipped: usize,
    pub bytes_indexed: u64,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditReceipt {
    pub root: PathBuf,
    pub divergence_count: usize,
    pub divergence: Divergence,
}

/// Live-filesystem-vs-index divergence returned by the consistency oracle.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Divergence {
    pub missing_from_index: Vec<String>,
    pub indexed_but_missing_on_disk: Vec<String>,
    pub hash_mismatches: Vec<String>,
    pub excluded_index_paths: Vec<String>,
    pub doctree_without_real_file: Vec<String>,
}

impl Divergence {
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    pub fn count(&self) -> usize {
        self.missing_from_index.len()
            + self.indexed_but_missing_on_disk.len()
            + self.hash_mismatches.len()
            + self.excluded_index_paths.len()
            + self.doctree_without_real_file.len()
    }
}

/// File type exposed by the W6 DocTree mount core.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountEntryKind {
    File,
    Directory,
}

/// Minimal metadata a FUSE binding needs before translating into platform attrs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountMetadata {
    pub kind: MountEntryKind,
    pub len: u64,
}

/// One immediate child in a mounted DocTree directory listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountDirEntry {
    pub name: String,
    pub kind: MountEntryKind,
}

/// Where a mount write landed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MountWriteDisposition {
    /// Source-like content went into the DocTree through `Engine::fs_write`.
    Stored,
    /// Artifact-like content was accepted by the mount boundary but must be
    /// routed to throwaway disk by the platform host, not indexed in DocTree.
    Throwaway,
}

/// Receipt for a filesystem-style write through the mount core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountWriteReceipt {
    pub path: String,
    pub bytes_written: u64,
    pub disposition: MountWriteDisposition,
}

/// Where a mount unlink landed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MountUnlinkDisposition {
    /// A source-like file was removed from the DocTree through
    /// `Engine::fs_unlink`.
    Removed,
    /// No source-like DocTree file existed at that path.
    Missing,
    /// Artifact-like deletion belongs to the platform host's throwaway disk.
    Throwaway,
}

/// Where a mount directory removal landed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MountRemoveDirDisposition {
    /// An explicit empty directory marker was removed from the DocTree.
    Removed,
    /// DocTree directories are synthetic; a directory with source-backed
    /// children cannot be removed as an empty directory.
    NotEmpty,
    /// No source-like DocTree directory existed at that path.
    Missing,
    /// The requested path is a source-like file, not a directory.
    NotDirectory,
    /// Artifact-like directory deletion belongs to the platform host's
    /// throwaway disk.
    Throwaway,
}

/// Where a mount directory creation landed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MountMakeDirDisposition {
    /// An explicit empty directory marker was created in the DocTree.
    Created,
    /// The directory already exists, either explicitly or through descendants.
    AlreadyExists,
    /// A source-like file exists at that path.
    FileExists,
    /// Artifact-like directory creation belongs to the platform host's
    /// throwaway disk.
    Throwaway,
}

/// Receipt for a filesystem-style unlink through the mount core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountUnlinkReceipt {
    pub path: String,
    pub disposition: MountUnlinkDisposition,
}

/// Receipt for a filesystem-style directory removal through the mount core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountRemoveDirReceipt {
    pub path: String,
    pub disposition: MountRemoveDirDisposition,
}

/// Receipt for a filesystem-style directory creation through the mount core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountMakeDirReceipt {
    pub path: String,
    pub disposition: MountMakeDirDisposition,
}

/// Where a mount rename landed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MountRenameDisposition {
    /// A source-like file was renamed inside the DocTree.
    Renamed,
    /// No source-like DocTree file existed at the source path.
    Missing,
    /// Artifact-like rename belongs to the platform host's throwaway disk.
    Throwaway,
}

/// Receipt for a filesystem-style rename through the mount core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountRenameReceipt {
    pub from: String,
    pub to: String,
    pub disposition: MountRenameDisposition,
}

/// W6 mount core over an embedded DocTree subtree.
///
/// This is deliberately not a FUSE binding yet. It is the reusable, tested
/// translation layer a macFUSE/fuse3 host will call: path normalization,
/// single-copy reads/writes through the W0 Engine seam, directory projection,
/// and artifact routing decisions.
pub struct DocTreeMount<'a> {
    engine: &'a Engine,
    prefix: String,
}

impl<'a> DocTreeMount<'a> {
    pub fn new(engine: &'a Engine, prefix: &str) -> Self {
        Self {
            engine,
            prefix: prefix.trim_matches('/').to_string(),
        }
    }

    pub fn read_file(&self, path: &str) -> Result<Option<Vec<u8>>, WorkspaceError> {
        let relative = safe_mount_path(path)?;
        if relative.as_os_str().is_empty() || should_skip(&relative) {
            return Ok(None);
        }
        self.engine
            .fs_read(&mount_engine_path(&self.prefix, &relative)?)
            .map_err(WorkspaceError::Engine)
    }

    pub fn write_file(
        &self,
        path: &str,
        content: &[u8],
    ) -> Result<MountWriteReceipt, WorkspaceError> {
        let relative = safe_mount_path(path)?;
        if relative.as_os_str().is_empty() {
            return Err(WorkspaceError::Path(
                "cannot write the mount root as a file".to_string(),
            ));
        }
        let path = relative_to_string(&relative)?;
        if should_skip(&relative) {
            return Ok(MountWriteReceipt {
                path,
                bytes_written: content.len() as u64,
                disposition: MountWriteDisposition::Throwaway,
            });
        }

        self.engine
            .fs_write(&mount_engine_path(&self.prefix, &relative)?, content)
            .map_err(WorkspaceError::Engine)?;
        Ok(MountWriteReceipt {
            path,
            bytes_written: content.len() as u64,
            disposition: MountWriteDisposition::Stored,
        })
    }

    pub fn unlink(&self, path: &str) -> Result<MountUnlinkReceipt, WorkspaceError> {
        let relative = safe_mount_path(path)?;
        if relative.as_os_str().is_empty() {
            return Err(WorkspaceError::Path(
                "cannot unlink the mount root".to_string(),
            ));
        }
        let path = relative_to_string(&relative)?;
        if should_skip(&relative) {
            return Ok(MountUnlinkReceipt {
                path,
                disposition: MountUnlinkDisposition::Throwaway,
            });
        }

        let removed = self
            .engine
            .fs_unlink(&mount_engine_path(&self.prefix, &relative)?)
            .map_err(WorkspaceError::Engine)?;
        Ok(MountUnlinkReceipt {
            path,
            disposition: if removed {
                MountUnlinkDisposition::Removed
            } else {
                MountUnlinkDisposition::Missing
            },
        })
    }

    pub fn rmdir(&self, path: &str) -> Result<MountRemoveDirReceipt, WorkspaceError> {
        let relative = safe_mount_path(path)?;
        if relative.as_os_str().is_empty() {
            return Err(WorkspaceError::Path(
                "cannot remove the mount root".to_string(),
            ));
        }
        let path = relative_to_string(&relative)?;
        if should_skip(&relative) {
            return Ok(MountRemoveDirReceipt {
                path,
                disposition: MountRemoveDirDisposition::Throwaway,
            });
        }
        if self
            .engine
            .fs_read(&mount_engine_path(&self.prefix, &relative)?)
            .map_err(WorkspaceError::Engine)?
            .is_some()
        {
            return Ok(MountRemoveDirReceipt {
                path,
                disposition: MountRemoveDirDisposition::NotDirectory,
            });
        }
        let outcome = self
            .engine
            .fs_rmdir(&mount_engine_path(&self.prefix, &relative)?)
            .map_err(WorkspaceError::Engine)?;
        Ok(MountRemoveDirReceipt {
            path,
            disposition: match outcome {
                DirectoryRemoveDisposition::Removed => MountRemoveDirDisposition::Removed,
                DirectoryRemoveDisposition::Missing => MountRemoveDirDisposition::Missing,
                DirectoryRemoveDisposition::NotEmpty => MountRemoveDirDisposition::NotEmpty,
                DirectoryRemoveDisposition::NotDirectory => MountRemoveDirDisposition::NotDirectory,
            },
        })
    }

    pub fn mkdir(&self, path: &str) -> Result<MountMakeDirReceipt, WorkspaceError> {
        let relative = safe_mount_path(path)?;
        if relative.as_os_str().is_empty() {
            return Err(WorkspaceError::Path(
                "cannot create the mount root".to_string(),
            ));
        }
        let path = relative_to_string(&relative)?;
        if should_skip(&relative) {
            return Ok(MountMakeDirReceipt {
                path,
                disposition: MountMakeDirDisposition::Throwaway,
            });
        }
        if self
            .engine
            .fs_read(&mount_engine_path(&self.prefix, &relative)?)
            .map_err(WorkspaceError::Engine)?
            .is_some()
        {
            return Ok(MountMakeDirReceipt {
                path,
                disposition: MountMakeDirDisposition::FileExists,
            });
        }
        let created = self
            .engine
            .fs_mkdir(&mount_engine_path(&self.prefix, &relative)?)
            .map_err(WorkspaceError::Engine)?;
        Ok(MountMakeDirReceipt {
            path,
            disposition: if created {
                MountMakeDirDisposition::Created
            } else {
                MountMakeDirDisposition::AlreadyExists
            },
        })
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<MountRenameReceipt, WorkspaceError> {
        let from_relative = safe_mount_path(from)?;
        let to_relative = safe_mount_path(to)?;
        if from_relative.as_os_str().is_empty() || to_relative.as_os_str().is_empty() {
            return Err(WorkspaceError::Path(
                "cannot rename the mount root".to_string(),
            ));
        }
        let from = relative_to_string(&from_relative)?;
        let to = relative_to_string(&to_relative)?;
        if should_skip(&from_relative) || should_skip(&to_relative) {
            return Ok(MountRenameReceipt {
                from,
                to,
                disposition: MountRenameDisposition::Throwaway,
            });
        }
        if from_relative != to_relative && to_relative.starts_with(&from_relative) {
            return Err(WorkspaceError::Path(
                "cannot rename a directory into its own subtree".to_string(),
            ));
        }

        let renamed = self
            .engine
            .fs_rename(
                &mount_engine_path(&self.prefix, &from_relative)?,
                &mount_engine_path(&self.prefix, &to_relative)?,
            )
            .map_err(WorkspaceError::Engine)?;
        if renamed {
            return Ok(MountRenameReceipt {
                from,
                to,
                disposition: MountRenameDisposition::Renamed,
            });
        }

        let from_prefix = mount_engine_prefix(&self.prefix, &from_relative)?;
        let mut engine_paths = self.engine.list_paths(&from_prefix)?;
        let mut directory_paths = self.engine.list_directories(&from_prefix)?;
        if engine_paths.is_empty() && directory_paths.is_empty() {
            if self
                .engine
                .fs_is_dir(&mount_engine_path(&self.prefix, &from_relative)?)
                .map_err(WorkspaceError::Engine)?
            {
                self.engine
                    .fs_mkdir(&mount_engine_path(&self.prefix, &to_relative)?)
                    .map_err(WorkspaceError::Engine)?;
                let _ = self
                    .engine
                    .fs_rmdir(&mount_engine_path(&self.prefix, &from_relative)?)
                    .map_err(WorkspaceError::Engine)?;
                return Ok(MountRenameReceipt {
                    from,
                    to,
                    disposition: MountRenameDisposition::Renamed,
                });
            }
            return Ok(MountRenameReceipt {
                from,
                to,
                disposition: MountRenameDisposition::Missing,
            });
        }
        engine_paths.sort();
        directory_paths.sort();
        self.engine
            .fs_mkdir(&mount_engine_path(&self.prefix, &to_relative)?)
            .map_err(WorkspaceError::Engine)?;
        for directory_path in &directory_paths {
            let full_relative = materialized_relative_path(&self.prefix, directory_path)?;
            let suffix = full_relative.strip_prefix(&from_relative).map_err(|_| {
                WorkspaceError::Path(format!(
                    "listed directory {:?} is outside renamed directory {:?}",
                    full_relative, from_relative
                ))
            })?;
            if suffix.as_os_str().is_empty() {
                continue;
            }
            let destination = to_relative.join(suffix);
            self.engine
                .fs_mkdir(&mount_engine_path(&self.prefix, &destination)?)
                .map_err(WorkspaceError::Engine)?;
        }
        for engine_path in engine_paths {
            let full_relative = materialized_relative_path(&self.prefix, &engine_path)?;
            let suffix = full_relative.strip_prefix(&from_relative).map_err(|_| {
                WorkspaceError::Path(format!(
                    "listed path {:?} is outside renamed directory {:?}",
                    full_relative, from_relative
                ))
            })?;
            if suffix.as_os_str().is_empty() {
                continue;
            }
            let destination = to_relative.join(suffix);
            self.engine
                .fs_rename(
                    &engine_path,
                    &mount_engine_path(&self.prefix, &destination)?,
                )
                .map_err(WorkspaceError::Engine)?;
        }
        directory_paths.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
        for directory_path in directory_paths {
            let _ = self
                .engine
                .fs_rmdir(&directory_path)
                .map_err(WorkspaceError::Engine)?;
        }
        let _ = self
            .engine
            .fs_rmdir(&mount_engine_path(&self.prefix, &from_relative)?)
            .map_err(WorkspaceError::Engine)?;
        Ok(MountRenameReceipt {
            from,
            to,
            disposition: MountRenameDisposition::Renamed,
        })
    }

    pub fn getattr(&self, path: &str) -> Result<Option<MountMetadata>, WorkspaceError> {
        let relative = safe_mount_path(path)?;
        if relative.as_os_str().is_empty() {
            return Ok(Some(MountMetadata {
                kind: MountEntryKind::Directory,
                len: 0,
            }));
        }
        if should_skip(&relative) {
            return Ok(None);
        }
        if let Some(body) = self
            .engine
            .fs_read(&mount_engine_path(&self.prefix, &relative)?)
            .map_err(WorkspaceError::Engine)?
        {
            return Ok(Some(MountMetadata {
                kind: MountEntryKind::File,
                len: body.len() as u64,
            }));
        }
        let prefix = mount_engine_prefix(&self.prefix, &relative)?;
        if self.engine.list_paths(&prefix)?.is_empty()
            && self.engine.list_directories(&prefix)?.is_empty()
            && !self
                .engine
                .fs_is_dir(&mount_engine_path(&self.prefix, &relative)?)
                .map_err(WorkspaceError::Engine)?
        {
            Ok(None)
        } else {
            Ok(Some(MountMetadata {
                kind: MountEntryKind::Directory,
                len: 0,
            }))
        }
    }

    pub fn read_dir(&self, path: &str) -> Result<Vec<MountDirEntry>, WorkspaceError> {
        let relative = safe_mount_path(path)?;
        if should_skip(&relative) {
            return Ok(Vec::new());
        }
        let prefix = mount_engine_prefix(&self.prefix, &relative)?;
        let mut entries = BTreeMap::<String, MountEntryKind>::new();
        for engine_path in self
            .engine
            .list_paths(&prefix)?
            .into_iter()
            .chain(self.engine.list_directories(&prefix)?)
        {
            let full_relative = materialized_relative_path(&self.prefix, &engine_path)?;
            if should_skip(&full_relative) {
                continue;
            }
            let child = if relative.as_os_str().is_empty() {
                full_relative.as_path()
            } else {
                match full_relative.strip_prefix(&relative) {
                    Ok(rest) => rest,
                    Err(_) => continue,
                }
            };
            let mut components = child.components().filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_string_lossy().to_string()),
                _ => None,
            });
            let Some(name) = components.next() else {
                continue;
            };
            let kind = if components.next().is_some() {
                MountEntryKind::Directory
            } else {
                MountEntryKind::File
            };
            entries
                .entry(name)
                .and_modify(|existing| {
                    if kind == MountEntryKind::Directory {
                        *existing = MountEntryKind::Directory;
                    }
                })
                .or_insert(kind);
        }
        Ok(entries
            .into_iter()
            .map(|(name, kind)| MountDirEntry { name, kind })
            .collect())
    }
}

#[cfg(feature = "fuse-host")]
pub mod fuse_host {
    use std::collections::{BTreeMap, HashMap};
    use std::ffi::OsStr;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::{mpsc, Mutex};
    use std::thread;
    use std::time::{Duration, SystemTime};

    use fuser::{
        Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo,
        MountOption, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory,
        ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request, WriteFlags,
    };
    use rustyred_embedded::{EmbeddedConfig, Engine};

    use crate::{
        DocTreeMount, MountDirEntry, MountEntryKind, MountMakeDirDisposition, MountMetadata,
        MountRemoveDirDisposition, MountRenameDisposition, MountUnlinkDisposition,
        MountWriteDisposition, WorkspaceError,
    };

    const ROOT_INO: u64 = 1;
    const TTL: Duration = Duration::from_secs(1);

    /// POSIX metadata policy for the FUSE host.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct FuseAttrPolicy {
        pub file_perm: u16,
        pub dir_perm: u16,
        pub uid: u32,
        pub gid: u32,
        pub block_size: u32,
    }

    impl Default for FuseAttrPolicy {
        fn default() -> Self {
            Self {
                file_perm: 0o644,
                dir_perm: 0o755,
                uid: current_uid(),
                gid: current_gid(),
                block_size: 4096,
            }
        }
    }

    /// Thread-safe backend contract for a platform FUSE host.
    ///
    /// `DocTreeMount` remains the semantic core for the embedded engine, but the
    /// current `Engine` is intentionally single-threaded (`Rc<RefCell<_>>` in the
    /// MCP store). A fuser filesystem must be `Send + Sync`, so this trait is the
    /// platform-host boundary until the engine handle itself is thread-safe.
    pub trait DocTreeFuseBackend: Send + Sync + 'static {
        fn read_file(&self, path: &str) -> Result<Option<Vec<u8>>, WorkspaceError>;
        fn write_file(
            &self,
            path: &str,
            content: &[u8],
        ) -> Result<MountWriteDisposition, WorkspaceError>;
        fn mkdir(&self, path: &str) -> Result<MountMakeDirDisposition, WorkspaceError>;
        fn unlink(&self, path: &str) -> Result<MountUnlinkDisposition, WorkspaceError>;
        fn rmdir(&self, path: &str) -> Result<MountRemoveDirDisposition, WorkspaceError>;
        fn rename(&self, from: &str, to: &str) -> Result<MountRenameDisposition, WorkspaceError>;
        fn getattr(&self, path: &str) -> Result<Option<MountMetadata>, WorkspaceError>;
        fn read_dir(&self, path: &str) -> Result<Vec<MountDirEntry>, WorkspaceError>;
    }

    /// Thread-safe FUSE backend over the real embedded `Engine`.
    ///
    /// The embedded engine intentionally stays single-threaded. This backend
    /// owns an actor thread that opens the engine, runs any setup hooks there,
    /// and services FUSE requests through `DocTreeMount`.
    pub struct EngineFuseBackend {
        sender: Mutex<mpsc::Sender<EngineFuseRequest>>,
    }

    impl EngineFuseBackend {
        pub fn open(
            data_dir: impl Into<PathBuf>,
            config: EmbeddedConfig,
            prefix: impl Into<String>,
        ) -> Result<Self, WorkspaceError> {
            Self::open_with_setup(data_dir, config, prefix, |_| Ok(()))
        }

        pub fn open_with_setup<F>(
            data_dir: impl Into<PathBuf>,
            config: EmbeddedConfig,
            prefix: impl Into<String>,
            setup: F,
        ) -> Result<Self, WorkspaceError>
        where
            F: FnOnce(&Engine) -> Result<(), WorkspaceError> + Send + 'static,
        {
            let data_dir = data_dir.into();
            let prefix = prefix.into();
            let (sender, receiver) = mpsc::channel();
            let (startup_sender, startup_receiver) = mpsc::channel();
            thread::Builder::new()
                .name("rustyred-fuse-engine".to_string())
                .spawn(move || {
                    let startup = match Engine::open(data_dir, config) {
                        Ok(engine) => match setup(&engine) {
                            Ok(()) => {
                                let _ = startup_sender.send(Ok(()));
                                serve_engine_fuse_backend(engine, prefix, receiver);
                                return;
                            }
                            Err(error) => Err(error),
                        },
                        Err(error) => Err(WorkspaceError::Engine(error)),
                    };
                    let _ = startup_sender.send(startup);
                })
                .map_err(|error| {
                    WorkspaceError::Run(format!("starting FUSE engine actor: {error}"))
                })?;

            startup_receiver.recv().map_err(|error| {
                WorkspaceError::Run(format!("waiting for FUSE engine actor startup: {error}"))
            })??;

            Ok(Self {
                sender: Mutex::new(sender),
            })
        }

        fn request<T>(
            &self,
            build: impl FnOnce(mpsc::Sender<Result<T, WorkspaceError>>) -> EngineFuseRequest,
        ) -> Result<T, WorkspaceError>
        where
            T: Send + 'static,
        {
            let (reply_sender, reply_receiver) = mpsc::channel();
            let request = build(reply_sender);
            let sender = self
                .sender
                .lock()
                .map_err(|_| WorkspaceError::Run("FUSE engine actor sender lock poisoned".into()))?
                .clone();
            sender
                .send(request)
                .map_err(|error| WorkspaceError::Run(format!("sending FUSE request: {error}")))?;
            reply_receiver.recv().map_err(|error| {
                WorkspaceError::Run(format!("receiving FUSE engine actor reply: {error}"))
            })?
        }
    }

    impl DocTreeFuseBackend for EngineFuseBackend {
        fn read_file(&self, path: &str) -> Result<Option<Vec<u8>>, WorkspaceError> {
            self.request(|reply| EngineFuseRequest::ReadFile {
                path: path.to_string(),
                reply,
            })
        }

        fn write_file(
            &self,
            path: &str,
            content: &[u8],
        ) -> Result<MountWriteDisposition, WorkspaceError> {
            self.request(|reply| EngineFuseRequest::WriteFile {
                path: path.to_string(),
                content: content.to_vec(),
                reply,
            })
        }

        fn mkdir(&self, path: &str) -> Result<MountMakeDirDisposition, WorkspaceError> {
            self.request(|reply| EngineFuseRequest::MakeDir {
                path: path.to_string(),
                reply,
            })
        }

        fn unlink(&self, path: &str) -> Result<MountUnlinkDisposition, WorkspaceError> {
            self.request(|reply| EngineFuseRequest::Unlink {
                path: path.to_string(),
                reply,
            })
        }

        fn rmdir(&self, path: &str) -> Result<MountRemoveDirDisposition, WorkspaceError> {
            self.request(|reply| EngineFuseRequest::RemoveDir {
                path: path.to_string(),
                reply,
            })
        }

        fn rename(&self, from: &str, to: &str) -> Result<MountRenameDisposition, WorkspaceError> {
            self.request(|reply| EngineFuseRequest::Rename {
                from: from.to_string(),
                to: to.to_string(),
                reply,
            })
        }

        fn getattr(&self, path: &str) -> Result<Option<MountMetadata>, WorkspaceError> {
            self.request(|reply| EngineFuseRequest::GetAttr {
                path: path.to_string(),
                reply,
            })
        }

        fn read_dir(&self, path: &str) -> Result<Vec<MountDirEntry>, WorkspaceError> {
            self.request(|reply| EngineFuseRequest::ReadDir {
                path: path.to_string(),
                reply,
            })
        }
    }

    enum EngineFuseRequest {
        ReadFile {
            path: String,
            reply: mpsc::Sender<Result<Option<Vec<u8>>, WorkspaceError>>,
        },
        WriteFile {
            path: String,
            content: Vec<u8>,
            reply: mpsc::Sender<Result<MountWriteDisposition, WorkspaceError>>,
        },
        MakeDir {
            path: String,
            reply: mpsc::Sender<Result<MountMakeDirDisposition, WorkspaceError>>,
        },
        Unlink {
            path: String,
            reply: mpsc::Sender<Result<MountUnlinkDisposition, WorkspaceError>>,
        },
        RemoveDir {
            path: String,
            reply: mpsc::Sender<Result<MountRemoveDirDisposition, WorkspaceError>>,
        },
        Rename {
            from: String,
            to: String,
            reply: mpsc::Sender<Result<MountRenameDisposition, WorkspaceError>>,
        },
        GetAttr {
            path: String,
            reply: mpsc::Sender<Result<Option<MountMetadata>, WorkspaceError>>,
        },
        ReadDir {
            path: String,
            reply: mpsc::Sender<Result<Vec<MountDirEntry>, WorkspaceError>>,
        },
    }

    fn serve_engine_fuse_backend(
        engine: Engine,
        prefix: String,
        receiver: mpsc::Receiver<EngineFuseRequest>,
    ) {
        let mount = DocTreeMount::new(&engine, &prefix);
        while let Ok(request) = receiver.recv() {
            match request {
                EngineFuseRequest::ReadFile { path, reply } => {
                    let _ = reply.send(mount.read_file(&path));
                }
                EngineFuseRequest::WriteFile {
                    path,
                    content,
                    reply,
                } => {
                    let _ = reply.send(
                        mount
                            .write_file(&path, &content)
                            .map(|receipt| receipt.disposition),
                    );
                }
                EngineFuseRequest::MakeDir { path, reply } => {
                    let _ = reply.send(mount.mkdir(&path).map(|receipt| receipt.disposition));
                }
                EngineFuseRequest::Unlink { path, reply } => {
                    let _ = reply.send(mount.unlink(&path).map(|receipt| receipt.disposition));
                }
                EngineFuseRequest::RemoveDir { path, reply } => {
                    let _ = reply.send(mount.rmdir(&path).map(|receipt| receipt.disposition));
                }
                EngineFuseRequest::Rename { from, to, reply } => {
                    let _ = reply.send(mount.rename(&from, &to).map(|receipt| receipt.disposition));
                }
                EngineFuseRequest::GetAttr { path, reply } => {
                    let _ = reply.send(mount.getattr(&path));
                }
                EngineFuseRequest::ReadDir { path, reply } => {
                    let _ = reply.send(mount.read_dir(&path));
                }
            }
        }
    }

    /// A read/write FUSE filesystem adapter for DocTree-like backends.
    ///
    /// This is W6's platform host shell: path/inode translation, fuser replies,
    /// write buffering for offset writes, and forwarding of file operations into
    /// the same source/artifact boundary exposed by the mount core.
    pub struct DocTreeFuseHost<B> {
        backend: B,
        attr_policy: FuseAttrPolicy,
        inodes: Mutex<InodeTable>,
        write_buffers: Mutex<HashMap<u64, Vec<u8>>>,
        throwaway: Mutex<ThrowawayDisk>,
    }

    impl<B: DocTreeFuseBackend> DocTreeFuseHost<B> {
        pub fn new(backend: B) -> Self {
            Self::with_attr_policy(backend, FuseAttrPolicy::default())
        }

        pub fn with_attr_policy(backend: B, attr_policy: FuseAttrPolicy) -> Self {
            Self {
                backend,
                attr_policy,
                inodes: Mutex::new(InodeTable::new()),
                write_buffers: Mutex::new(HashMap::new()),
                throwaway: Mutex::new(ThrowawayDisk::new()),
            }
        }

        fn path_for(&self, ino: INodeNo) -> Result<PathBuf, Errno> {
            self.inodes
                .lock()
                .map_err(|_| Errno::EIO)?
                .path_for(u64::from(ino))
                .ok_or(Errno::ENOENT)
        }

        fn record_path(&self, path: PathBuf) -> Result<u64, Errno> {
            self.inodes.lock().map_err(|_| Errno::EIO)?.record(path)
        }

        fn forget_path(&self, path: &Path) -> Result<(), Errno> {
            let forgotten = self.inodes.lock().map_err(|_| Errno::EIO)?.forget(path);
            if !forgotten.is_empty() {
                let mut write_buffers = self.write_buffers.lock().map_err(|_| Errno::EIO)?;
                for ino in forgotten {
                    write_buffers.remove(&ino);
                }
            }
            Ok(())
        }

        fn forget_inode(&self, ino: INodeNo) -> Result<(), Errno> {
            if u64::from(ino) == ROOT_INO {
                return Ok(());
            }
            let Some(path) = self
                .inodes
                .lock()
                .map_err(|_| Errno::EIO)?
                .path_for(u64::from(ino))
            else {
                return Ok(());
            };
            self.forget_path(&path)
        }

        #[cfg(test)]
        pub(crate) fn test_record_path(&self, path: &Path) -> Result<u64, Errno> {
            self.record_path(path.to_path_buf())
        }

        #[cfg(test)]
        pub(crate) fn test_forget_path(&self, path: &Path) -> Result<(), Errno> {
            self.forget_path(path)
        }

        #[cfg(test)]
        pub(crate) fn test_seed_write_buffer(&self, ino: u64, content: Vec<u8>) {
            self.write_buffers
                .lock()
                .expect("write buffer lock")
                .insert(ino, content);
        }

        #[cfg(test)]
        pub(crate) fn test_has_write_buffer(&self, ino: u64) -> bool {
            self.write_buffers
                .lock()
                .expect("write buffer lock")
                .contains_key(&ino)
        }

        #[cfg(test)]
        pub(crate) fn test_path_for(&self, ino: u64) -> Option<PathBuf> {
            self.inodes.lock().expect("inode lock").path_for(ino)
        }

        fn rename_path(&self, from: &Path, to: PathBuf) -> Result<(), Errno> {
            self.inodes.lock().map_err(|_| Errno::EIO)?.rename(from, to);
            Ok(())
        }

        fn child_path(&self, parent: INodeNo, name: &OsStr) -> Result<PathBuf, Errno> {
            let mut path = self.path_for(parent)?;
            let Some(name) = name.to_str() else {
                return Err(Errno::EINVAL);
            };
            if name.is_empty() || name.contains('/') {
                return Err(Errno::EINVAL);
            }
            path.push(name);
            Ok(path)
        }

        fn path_string(path: &Path) -> Result<String, Errno> {
            let parts = path
                .components()
                .map(|component| match component {
                    std::path::Component::Normal(value) => value.to_str().ok_or(Errno::EINVAL),
                    _ => Err(Errno::EINVAL),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(parts.join("/"))
        }

        fn metadata_for_path(&self, path: &Path) -> Result<Option<(u64, FileAttr)>, Errno> {
            let path_string = Self::path_string(path)?;
            let metadata = match self.backend.getattr(&path_string).map_err(|_| Errno::EIO)? {
                Some(metadata) => Some(metadata),
                None => self
                    .throwaway
                    .lock()
                    .map_err(|_| Errno::EIO)?
                    .metadata(path)
                    .map_err(errno_from_io)?,
            };
            let Some(metadata) = metadata else {
                return Ok(None);
            };
            let ino = self.record_path(path.to_path_buf())?;
            Ok(Some((ino, file_attr(ino, metadata, self.attr_policy))))
        }

        fn read_file_for_path(&self, path: &Path) -> Result<Option<Vec<u8>>, Errno> {
            let path_string = Self::path_string(path)?;
            if let Some(body) = self
                .backend
                .read_file(&path_string)
                .map_err(|_| Errno::EIO)?
            {
                return Ok(Some(body));
            }
            self.throwaway
                .lock()
                .map_err(|_| Errno::EIO)?
                .read_file(path)
                .map_err(errno_from_io)
        }

        fn write_file_for_path(
            &self,
            path: &Path,
            content: &[u8],
        ) -> Result<MountWriteDisposition, Errno> {
            let path_string = Self::path_string(path)?;
            match self
                .backend
                .write_file(&path_string, content)
                .map_err(|_| Errno::EIO)?
            {
                MountWriteDisposition::Stored => Ok(MountWriteDisposition::Stored),
                MountWriteDisposition::Throwaway => {
                    self.throwaway
                        .lock()
                        .map_err(|_| Errno::EIO)?
                        .write_file(path, content)
                        .map_err(errno_from_io)?;
                    Ok(MountWriteDisposition::Throwaway)
                }
            }
        }

        fn mkdir_path(&self, path: &Path) -> Result<(), Errno> {
            let path_string = Self::path_string(path)?;
            match self.backend.mkdir(&path_string).map_err(|_| Errno::EIO)? {
                MountMakeDirDisposition::Created => Ok(()),
                MountMakeDirDisposition::AlreadyExists | MountMakeDirDisposition::FileExists => {
                    Err(Errno::EEXIST)
                }
                MountMakeDirDisposition::Throwaway => match self
                    .throwaway
                    .lock()
                    .map_err(|_| Errno::EIO)?
                    .mkdir(path)
                    .map_err(errno_from_io)?
                {
                    MountMakeDirDisposition::Created => Ok(()),
                    MountMakeDirDisposition::AlreadyExists
                    | MountMakeDirDisposition::FileExists => Err(Errno::EEXIST),
                    MountMakeDirDisposition::Throwaway => unreachable!("throwaway disk resolved"),
                },
            }
        }

        fn unlink_path(&self, path: &Path) -> Result<(), Errno> {
            let path_string = Self::path_string(path)?;
            match self.backend.unlink(&path_string).map_err(|_| Errno::EIO)? {
                MountUnlinkDisposition::Removed => Ok(()),
                MountUnlinkDisposition::Missing | MountUnlinkDisposition::Throwaway => {
                    if self
                        .throwaway
                        .lock()
                        .map_err(|_| Errno::EIO)?
                        .unlink(path)
                        .map_err(errno_from_io)?
                    {
                        Ok(())
                    } else {
                        Err(Errno::ENOENT)
                    }
                }
            }
        }

        fn rmdir_path(&self, path: &Path) -> Result<(), Errno> {
            let path_string = Self::path_string(path)?;
            match self.backend.rmdir(&path_string).map_err(|_| Errno::EIO)? {
                MountRemoveDirDisposition::Removed => Ok(()),
                MountRemoveDirDisposition::NotDirectory => Err(Errno::ENOTDIR),
                MountRemoveDirDisposition::NotEmpty => Err(Errno::ENOTEMPTY),
                MountRemoveDirDisposition::Missing | MountRemoveDirDisposition::Throwaway => {
                    match self
                        .throwaway
                        .lock()
                        .map_err(|_| Errno::EIO)?
                        .rmdir(path)
                        .map_err(errno_from_io)?
                    {
                        MountRemoveDirDisposition::Removed => Ok(()),
                        MountRemoveDirDisposition::Missing => Err(Errno::ENOENT),
                        MountRemoveDirDisposition::NotDirectory => Err(Errno::ENOTDIR),
                        MountRemoveDirDisposition::NotEmpty => Err(Errno::ENOTEMPTY),
                        MountRemoveDirDisposition::Throwaway => {
                            unreachable!("throwaway disk resolved")
                        }
                    }
                }
            }
        }

        fn rename_entry(&self, from: &Path, to: &Path) -> Result<(), Errno> {
            let from_is_throwaway = crate::should_skip(from);
            let to_is_throwaway = crate::should_skip(to);
            if from_is_throwaway || to_is_throwaway {
                return self.rename_across_throwaway_boundary(
                    from,
                    to,
                    from_is_throwaway,
                    to_is_throwaway,
                );
            }

            let from_string = Self::path_string(from)?;
            let to_string = Self::path_string(to)?;
            match self
                .backend
                .rename(&from_string, &to_string)
                .map_err(|_| Errno::EIO)?
            {
                MountRenameDisposition::Renamed => Ok(()),
                MountRenameDisposition::Missing | MountRenameDisposition::Throwaway => {
                    if self
                        .throwaway
                        .lock()
                        .map_err(|_| Errno::EIO)?
                        .rename(from, to)
                        .map_err(errno_from_io)?
                    {
                        Ok(())
                    } else {
                        Err(Errno::ENOENT)
                    }
                }
            }
        }

        fn rename_across_throwaway_boundary(
            &self,
            from: &Path,
            to: &Path,
            from_is_throwaway: bool,
            to_is_throwaway: bool,
        ) -> Result<(), Errno> {
            match (from_is_throwaway, to_is_throwaway) {
                (true, true) => {
                    if self
                        .throwaway
                        .lock()
                        .map_err(|_| Errno::EIO)?
                        .rename(from, to)
                        .map_err(errno_from_io)?
                    {
                        Ok(())
                    } else {
                        Err(Errno::ENOENT)
                    }
                }
                (true, false) => {
                    self.copy_throwaway_to_backend(from, to)?;
                    self.throwaway
                        .lock()
                        .map_err(|_| Errno::EIO)?
                        .remove_tree(from)
                        .map_err(errno_from_io)?;
                    Ok(())
                }
                (false, true) => {
                    self.copy_backend_to_throwaway(from, to)?;
                    self.remove_backend_tree(from)
                }
                (false, false) => unreachable!("non-throwaway rename handled by backend"),
            }
        }

        fn copy_throwaway_to_backend(&self, from: &Path, to: &Path) -> Result<(), Errno> {
            let metadata = self
                .throwaway
                .lock()
                .map_err(|_| Errno::EIO)?
                .metadata(from)
                .map_err(errno_from_io)?
                .ok_or(Errno::ENOENT)?;
            match metadata.kind {
                MountEntryKind::File => {
                    let body = self
                        .throwaway
                        .lock()
                        .map_err(|_| Errno::EIO)?
                        .read_file(from)
                        .map_err(errno_from_io)?
                        .ok_or(Errno::ENOENT)?;
                    self.write_backend_file(to, &body)
                }
                MountEntryKind::Directory => {
                    self.mkdir_backend_dir(to)?;
                    let entries = self
                        .throwaway
                        .lock()
                        .map_err(|_| Errno::EIO)?
                        .read_dir(from)
                        .map_err(errno_from_io)?;
                    for entry in entries {
                        self.copy_throwaway_to_backend(
                            &from.join(&entry.name),
                            &to.join(&entry.name),
                        )?;
                    }
                    Ok(())
                }
            }
        }

        fn copy_backend_to_throwaway(&self, from: &Path, to: &Path) -> Result<(), Errno> {
            let from_string = Self::path_string(from)?;
            if let Some(body) = self
                .backend
                .read_file(&from_string)
                .map_err(|_| Errno::EIO)?
            {
                self.throwaway
                    .lock()
                    .map_err(|_| Errno::EIO)?
                    .write_file(to, &body)
                    .map_err(errno_from_io)?;
                return Ok(());
            }
            let metadata = self
                .backend
                .getattr(&from_string)
                .map_err(|_| Errno::EIO)?
                .ok_or(Errno::ENOENT)?;
            if metadata.kind != MountEntryKind::Directory {
                return Err(Errno::ENOENT);
            }
            self.throwaway
                .lock()
                .map_err(|_| Errno::EIO)?
                .mkdir(to)
                .map_err(errno_from_io)?;
            for entry in self
                .backend
                .read_dir(&from_string)
                .map_err(|_| Errno::EIO)?
            {
                self.copy_backend_to_throwaway(&from.join(&entry.name), &to.join(&entry.name))?;
            }
            Ok(())
        }

        fn write_backend_file(&self, path: &Path, body: &[u8]) -> Result<(), Errno> {
            let path_string = Self::path_string(path)?;
            match self
                .backend
                .write_file(&path_string, body)
                .map_err(|_| Errno::EIO)?
            {
                MountWriteDisposition::Stored => Ok(()),
                MountWriteDisposition::Throwaway => Err(Errno::EIO),
            }
        }

        fn mkdir_backend_dir(&self, path: &Path) -> Result<(), Errno> {
            let path_string = Self::path_string(path)?;
            match self.backend.mkdir(&path_string).map_err(|_| Errno::EIO)? {
                MountMakeDirDisposition::Created | MountMakeDirDisposition::AlreadyExists => Ok(()),
                MountMakeDirDisposition::FileExists => Err(Errno::EEXIST),
                MountMakeDirDisposition::Throwaway => Err(Errno::EIO),
            }
        }

        fn remove_backend_tree(&self, path: &Path) -> Result<(), Errno> {
            let path_string = Self::path_string(path)?;
            if self
                .backend
                .read_file(&path_string)
                .map_err(|_| Errno::EIO)?
                .is_some()
            {
                return match self.backend.unlink(&path_string).map_err(|_| Errno::EIO)? {
                    MountUnlinkDisposition::Removed => Ok(()),
                    MountUnlinkDisposition::Missing => Err(Errno::ENOENT),
                    MountUnlinkDisposition::Throwaway => Err(Errno::EIO),
                };
            }
            let metadata = self
                .backend
                .getattr(&path_string)
                .map_err(|_| Errno::EIO)?
                .ok_or(Errno::ENOENT)?;
            if metadata.kind != MountEntryKind::Directory {
                return Err(Errno::ENOTDIR);
            }
            for entry in self
                .backend
                .read_dir(&path_string)
                .map_err(|_| Errno::EIO)?
            {
                self.remove_backend_tree(&path.join(&entry.name))?;
            }
            match self.backend.rmdir(&path_string).map_err(|_| Errno::EIO)? {
                MountRemoveDirDisposition::Removed | MountRemoveDirDisposition::Missing => Ok(()),
                MountRemoveDirDisposition::NotDirectory => Err(Errno::ENOTDIR),
                MountRemoveDirDisposition::NotEmpty => Err(Errno::ENOTEMPTY),
                MountRemoveDirDisposition::Throwaway => Err(Errno::EIO),
            }
        }

        fn read_dir_entries(&self, path: &Path) -> Result<Vec<MountDirEntry>, Errno> {
            let path_string = Self::path_string(path)?;
            let mut entries = BTreeMap::<String, MountEntryKind>::new();
            for entry in self
                .backend
                .read_dir(&path_string)
                .map_err(|_| Errno::EIO)?
                .into_iter()
                .chain(
                    self.throwaway
                        .lock()
                        .map_err(|_| Errno::EIO)?
                        .read_dir(path)
                        .map_err(errno_from_io)?,
                )
            {
                entries
                    .entry(entry.name)
                    .and_modify(|existing| {
                        if entry.kind == MountEntryKind::Directory {
                            *existing = MountEntryKind::Directory;
                        }
                    })
                    .or_insert(entry.kind);
            }
            Ok(entries
                .into_iter()
                .map(|(name, kind)| MountDirEntry { name, kind })
                .collect())
        }

        #[cfg(test)]
        pub(crate) fn test_metadata_for_path(
            &self,
            path: &Path,
        ) -> Result<Option<(u64, FileAttr)>, Errno> {
            self.metadata_for_path(path)
        }

        #[cfg(test)]
        pub(crate) fn test_read_file(&self, path: &Path) -> Result<Option<Vec<u8>>, Errno> {
            self.read_file_for_path(path)
        }

        #[cfg(test)]
        pub(crate) fn test_write_file(
            &self,
            path: &Path,
            content: &[u8],
        ) -> Result<MountWriteDisposition, Errno> {
            self.write_file_for_path(path, content)
        }

        #[cfg(test)]
        pub(crate) fn test_mkdir(&self, path: &Path) -> Result<(), Errno> {
            self.mkdir_path(path)
        }

        #[cfg(test)]
        pub(crate) fn test_unlink(&self, path: &Path) -> Result<(), Errno> {
            self.unlink_path(path)
        }

        #[cfg(test)]
        pub(crate) fn test_rmdir(&self, path: &Path) -> Result<(), Errno> {
            self.rmdir_path(path)
        }

        #[cfg(test)]
        pub(crate) fn test_rename(&self, from: &Path, to: &Path) -> Result<(), Errno> {
            self.rename_entry(from, to)
        }

        #[cfg(test)]
        pub(crate) fn test_read_dir(&self, path: &Path) -> Result<Vec<MountDirEntry>, Errno> {
            self.read_dir_entries(path)
        }
    }

    impl<B: DocTreeFuseBackend> Filesystem for DocTreeFuseHost<B> {
        fn forget(&self, _req: &Request, ino: INodeNo, _nlookup: u64) {
            let _ = self.forget_inode(ino);
        }

        fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
            let result = self
                .child_path(parent, name)
                .and_then(|path| self.metadata_for_path(&path));
            match result {
                Ok(Some((_ino, attr))) => reply.entry(&TTL, &attr, Generation(0)),
                Ok(None) => reply.error(Errno::ENOENT),
                Err(error) => reply.error(error),
            }
        }

        fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
            let result = self
                .path_for(ino)
                .and_then(|path| self.metadata_for_path(&path));
            match result {
                Ok(Some((_ino, attr))) => reply.attr(&TTL, &attr),
                Ok(None) => reply.error(Errno::ENOENT),
                Err(error) => reply.error(error),
            }
        }

        fn open(&self, _req: &Request, _ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
            reply.opened(FileHandle(0), FopenFlags::empty());
        }

        fn read(
            &self,
            _req: &Request,
            ino: INodeNo,
            _fh: FileHandle,
            offset: u64,
            size: u32,
            _flags: OpenFlags,
            _lock_owner: Option<fuser::LockOwner>,
            reply: ReplyData,
        ) {
            let result = self
                .path_for(ino)
                .and_then(|path| self.read_file_for_path(&path)?.ok_or(Errno::ENOENT));
            match result {
                Ok(body) => {
                    let start = (offset as usize).min(body.len());
                    let end = (start + size as usize).min(body.len());
                    reply.data(&body[start..end]);
                }
                Err(error) => reply.error(error),
            }
        }

        fn write(
            &self,
            _req: &Request,
            ino: INodeNo,
            _fh: FileHandle,
            offset: u64,
            data: &[u8],
            _write_flags: WriteFlags,
            _flags: OpenFlags,
            _lock_owner: Option<fuser::LockOwner>,
            reply: ReplyWrite,
        ) {
            let result = self.path_for(ino).and_then(|path| {
                let mut buffers = self.write_buffers.lock().map_err(|_| Errno::EIO)?;
                let buffer = buffers.entry(u64::from(ino)).or_insert_with(|| {
                    self.read_file_for_path(&path)
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                });
                let start = offset as usize;
                let end = start.saturating_add(data.len());
                if buffer.len() < end {
                    buffer.resize(end, 0);
                }
                buffer[start..end].copy_from_slice(data);
                self.write_file_for_path(&path, buffer)?;
                Ok(data.len() as u32)
            });
            match result {
                Ok(written) => reply.written(written),
                Err(error) => reply.error(error),
            }
        }

        fn create(
            &self,
            _req: &Request,
            parent: INodeNo,
            name: &OsStr,
            _mode: u32,
            _umask: u32,
            _flags: i32,
            reply: ReplyCreate,
        ) {
            let result = self.child_path(parent, name).and_then(|path| {
                self.write_file_for_path(&path, &[])?;
                let ino = self.record_path(path)?;
                Ok(file_attr(
                    ino,
                    MountMetadata {
                        kind: MountEntryKind::File,
                        len: 0,
                    },
                    self.attr_policy,
                ))
            });
            match result {
                Ok(attr) => reply.created(
                    &TTL,
                    &attr,
                    Generation(0),
                    FileHandle(0),
                    FopenFlags::empty(),
                ),
                Err(error) => reply.error(error),
            }
        }

        fn mkdir(
            &self,
            _req: &Request,
            parent: INodeNo,
            name: &OsStr,
            _mode: u32,
            _umask: u32,
            reply: ReplyEntry,
        ) {
            let result = self.child_path(parent, name).and_then(|path| {
                self.mkdir_path(&path)?;
                let ino = self.record_path(path)?;
                Ok(file_attr(
                    ino,
                    MountMetadata {
                        kind: MountEntryKind::Directory,
                        len: 0,
                    },
                    self.attr_policy,
                ))
            });
            match result {
                Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
                Err(error) => reply.error(error),
            }
        }

        fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
            let result = self.child_path(parent, name).and_then(|path| {
                self.unlink_path(&path)?;
                self.forget_path(&path)?;
                Ok(())
            });
            match result {
                Ok(()) => reply.ok(),
                Err(error) => reply.error(error),
            }
        }

        fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
            let result = self.child_path(parent, name).and_then(|path| {
                self.rmdir_path(&path)?;
                self.forget_path(&path)?;
                Ok(())
            });
            match result {
                Ok(()) => reply.ok(),
                Err(error) => reply.error(error),
            }
        }

        fn rename(
            &self,
            _req: &Request,
            parent: INodeNo,
            name: &OsStr,
            newparent: INodeNo,
            newname: &OsStr,
            flags: RenameFlags,
            reply: ReplyEmpty,
        ) {
            if !flags.is_empty() {
                reply.error(Errno::EINVAL);
                return;
            }
            let result = self.child_path(parent, name).and_then(|from| {
                let to = self.child_path(newparent, newname)?;
                self.rename_entry(&from, &to)?;
                self.rename_path(&from, to)?;
                Ok(())
            });
            match result {
                Ok(()) => reply.ok(),
                Err(error) => reply.error(error),
            }
        }

        fn opendir(&self, _req: &Request, _ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
            reply.opened(FileHandle(0), FopenFlags::empty());
        }

        fn readdir(
            &self,
            _req: &Request,
            ino: INodeNo,
            _fh: FileHandle,
            offset: u64,
            mut reply: ReplyDirectory,
        ) {
            let result = self.path_for(ino).and_then(|path| {
                let mut entries = vec![
                    (u64::from(ino), FileType::Directory, ".".to_string()),
                    (
                        parent_ino(&self.inodes, &path)?,
                        FileType::Directory,
                        "..".to_string(),
                    ),
                ];
                for entry in self.read_dir_entries(&path)? {
                    let mut child = path.clone();
                    child.push(&entry.name);
                    let child_ino = self.record_path(child)?;
                    entries.push((child_ino, file_type(entry.kind), entry.name));
                }
                Ok(entries)
            });
            match result {
                Ok(entries) => {
                    for (idx, (ino, kind, name)) in
                        entries.into_iter().enumerate().skip(offset as usize)
                    {
                        if reply.add(INodeNo(ino), (idx + 1) as u64, kind, name) {
                            break;
                        }
                    }
                    reply.ok();
                }
                Err(error) => reply.error(error),
            }
        }
    }

    pub fn default_fuse_config() -> Config {
        let mut config = Config::default();
        config.mount_options = vec![
            MountOption::FSName("rustyred-doctree".to_string()),
            MountOption::RW,
            MountOption::NoDev,
            MountOption::NoSuid,
        ];
        // Keep the first host conservative until the embedded engine has a
        // thread-safe handle and POSIX race semantics are specified.
        config.n_threads = Some(1);
        config
    }

    pub fn mount_doctree_fuse<B: DocTreeFuseBackend>(
        backend: B,
        mountpoint: impl AsRef<Path>,
    ) -> io::Result<()> {
        fuser::mount2(
            DocTreeFuseHost::new(backend),
            mountpoint,
            &default_fuse_config(),
        )
    }

    struct ThrowawayDisk {
        root: PathBuf,
    }

    impl ThrowawayDisk {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "rustyred-fuse-throwaway-{}-{nonce}",
                std::process::id()
            ));
            Self { root }
        }

        fn metadata(&self, path: &Path) -> io::Result<Option<MountMetadata>> {
            let disk_path = self.disk_path(path)?;
            let metadata = match fs::metadata(disk_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
            let kind = if metadata.is_dir() {
                MountEntryKind::Directory
            } else if metadata.is_file() {
                MountEntryKind::File
            } else {
                return Ok(None);
            };
            Ok(Some(MountMetadata {
                kind,
                len: metadata.len(),
            }))
        }

        fn read_file(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
            let disk_path = self.disk_path(path)?;
            match fs::metadata(&disk_path) {
                Ok(metadata) if metadata.is_file() => fs::read(disk_path).map(Some),
                Ok(_) => Ok(None),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }

        fn write_file(&self, path: &Path, content: &[u8]) -> io::Result<()> {
            let disk_path = self.disk_path(path)?;
            if let Some(parent) = disk_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(disk_path, content)
        }

        fn mkdir(&self, path: &Path) -> io::Result<MountMakeDirDisposition> {
            let disk_path = self.disk_path(path)?;
            match fs::metadata(&disk_path) {
                Ok(metadata) if metadata.is_dir() => {
                    return Ok(MountMakeDirDisposition::AlreadyExists)
                }
                Ok(_) => return Ok(MountMakeDirDisposition::FileExists),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            fs::create_dir_all(&disk_path)?;
            Ok(MountMakeDirDisposition::Created)
        }

        fn unlink(&self, path: &Path) -> io::Result<bool> {
            let disk_path = self.disk_path(path)?;
            match fs::metadata(&disk_path) {
                Ok(metadata) if metadata.is_file() => {
                    fs::remove_file(disk_path)?;
                    Ok(true)
                }
                Ok(metadata) if metadata.is_dir() => Err(io::Error::new(
                    io::ErrorKind::IsADirectory,
                    "throwaway path is a directory",
                )),
                Ok(_) => Ok(false),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error),
            }
        }

        fn rmdir(&self, path: &Path) -> io::Result<MountRemoveDirDisposition> {
            let disk_path = self.disk_path(path)?;
            match fs::metadata(&disk_path) {
                Ok(metadata) if metadata.is_file() => {
                    return Ok(MountRemoveDirDisposition::NotDirectory)
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Ok(MountRemoveDirDisposition::NotDirectory),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok(MountRemoveDirDisposition::Missing)
                }
                Err(error) => return Err(error),
            }
            if fs::read_dir(&disk_path)?.next().is_some() {
                return Ok(MountRemoveDirDisposition::NotEmpty);
            }
            fs::remove_dir(disk_path)?;
            Ok(MountRemoveDirDisposition::Removed)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<bool> {
            let from_disk_path = self.disk_path(from)?;
            if fs::metadata(&from_disk_path)
                .map(|metadata| !(metadata.is_file() || metadata.is_dir()))
                .unwrap_or(false)
            {
                return Ok(false);
            }
            if !from_disk_path.exists() {
                return Ok(false);
            }
            let to_disk_path = self.disk_path(to)?;
            if let Some(parent) = to_disk_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(from_disk_path, to_disk_path)?;
            Ok(true)
        }

        fn remove_tree(&self, path: &Path) -> io::Result<bool> {
            let disk_path = self.disk_path(path)?;
            match fs::metadata(&disk_path) {
                Ok(metadata) if metadata.is_dir() => {
                    fs::remove_dir_all(disk_path)?;
                    Ok(true)
                }
                Ok(metadata) if metadata.is_file() => {
                    fs::remove_file(disk_path)?;
                    Ok(true)
                }
                Ok(_) => Ok(false),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error),
            }
        }

        fn read_dir(&self, path: &Path) -> io::Result<Vec<MountDirEntry>> {
            let disk_path = self.disk_path(path)?;
            let entries = match fs::read_dir(disk_path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(error) => return Err(error),
            };
            let mut children = Vec::new();
            for entry in entries {
                let entry = entry?;
                let metadata = entry.metadata()?;
                let kind = if metadata.is_dir() {
                    MountEntryKind::Directory
                } else if metadata.is_file() {
                    MountEntryKind::File
                } else {
                    continue;
                };
                children.push(MountDirEntry {
                    name: entry.file_name().to_string_lossy().to_string(),
                    kind,
                });
            }
            children.sort_by(|left, right| left.name.cmp(&right.name));
            Ok(children)
        }

        fn disk_path(&self, path: &Path) -> io::Result<PathBuf> {
            let mut disk_path = self.root.clone();
            for component in path.components() {
                let std::path::Component::Normal(value) = component else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "invalid throwaway path",
                    ));
                };
                disk_path.push(value);
            }
            Ok(disk_path)
        }
    }

    impl Drop for ThrowawayDisk {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn errno_from_io(error: io::Error) -> Errno {
        match error.kind() {
            io::ErrorKind::NotFound => Errno::ENOENT,
            io::ErrorKind::AlreadyExists => Errno::EEXIST,
            io::ErrorKind::PermissionDenied => Errno::EACCES,
            io::ErrorKind::InvalidInput => Errno::EINVAL,
            io::ErrorKind::DirectoryNotEmpty => Errno::ENOTEMPTY,
            io::ErrorKind::IsADirectory => Errno::EISDIR,
            _ => Errno::EIO,
        }
    }

    fn file_attr(ino: u64, metadata: MountMetadata, policy: FuseAttrPolicy) -> FileAttr {
        let now = SystemTime::UNIX_EPOCH;
        FileAttr {
            ino: INodeNo(ino),
            size: metadata.len,
            blocks: metadata.len.div_ceil(512),
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: file_type(metadata.kind),
            perm: match metadata.kind {
                MountEntryKind::Directory => policy.dir_perm,
                MountEntryKind::File => policy.file_perm,
            },
            nlink: match metadata.kind {
                MountEntryKind::Directory => 2,
                MountEntryKind::File => 1,
            },
            uid: policy.uid,
            gid: policy.gid,
            rdev: 0,
            blksize: policy.block_size,
            flags: 0,
        }
    }

    fn file_type(kind: MountEntryKind) -> FileType {
        match kind {
            MountEntryKind::File => FileType::RegularFile,
            MountEntryKind::Directory => FileType::Directory,
        }
    }

    #[cfg(unix)]
    fn current_uid() -> u32 {
        unsafe extern "C" {
            fn getuid() -> u32;
        }
        // SAFETY: `getuid` has no arguments, no Rust aliasing interaction, and
        // returns the current process user id.
        unsafe { getuid() }
    }

    #[cfg(not(unix))]
    fn current_uid() -> u32 {
        0
    }

    #[cfg(unix)]
    fn current_gid() -> u32 {
        unsafe extern "C" {
            fn getgid() -> u32;
        }
        // SAFETY: `getgid` has no arguments, no Rust aliasing interaction, and
        // returns the current process group id.
        unsafe { getgid() }
    }

    #[cfg(not(unix))]
    fn current_gid() -> u32 {
        0
    }

    fn parent_ino(inodes: &Mutex<InodeTable>, path: &Path) -> Result<u64, Errno> {
        if path.as_os_str().is_empty() {
            return Ok(ROOT_INO);
        }
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        inodes
            .lock()
            .map_err(|_| Errno::EIO)?
            .ino_for(parent)
            .ok_or(Errno::ENOENT)
    }

    struct InodeTable {
        by_path: HashMap<PathBuf, u64>,
        by_ino: HashMap<u64, PathBuf>,
        next_ino: u64,
    }

    impl InodeTable {
        fn new() -> Self {
            let mut by_path = HashMap::new();
            let mut by_ino = HashMap::new();
            by_path.insert(PathBuf::new(), ROOT_INO);
            by_ino.insert(ROOT_INO, PathBuf::new());
            Self {
                by_path,
                by_ino,
                next_ino: ROOT_INO + 1,
            }
        }

        fn record(&mut self, path: PathBuf) -> Result<u64, Errno> {
            if let Some(ino) = self.by_path.get(&path) {
                return Ok(*ino);
            }
            let ino = self.next_ino;
            self.next_ino = self.next_ino.checked_add(1).ok_or(Errno::EOVERFLOW)?;
            self.by_path.insert(path.clone(), ino);
            self.by_ino.insert(ino, path);
            Ok(ino)
        }

        fn ino_for(&self, path: &Path) -> Option<u64> {
            self.by_path.get(path).copied()
        }

        fn path_for(&self, ino: u64) -> Option<PathBuf> {
            self.by_ino.get(&ino).cloned()
        }

        fn forget(&mut self, path: &Path) -> Vec<u64> {
            let forgotten = self
                .by_path
                .iter()
                .filter_map(|(candidate, ino)| {
                    if candidate == path || candidate.starts_with(path) {
                        Some((candidate.clone(), *ino))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            let mut forgotten_inodes = Vec::with_capacity(forgotten.len());
            for (path, ino) in forgotten {
                self.by_path.remove(&path);
                self.by_ino.remove(&ino);
                forgotten_inodes.push(ino);
            }
            forgotten_inodes
        }

        fn rename(&mut self, from: &Path, to: PathBuf) {
            let renames = self
                .by_path
                .iter()
                .filter_map(|(path, ino)| {
                    if path == from || path.starts_with(from) {
                        let suffix = path.strip_prefix(from).ok()?;
                        Some((path.clone(), *ino, to.join(suffix)))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            for (old_path, ino, new_path) in renames {
                self.by_path.remove(&old_path);
                self.by_ino.insert(ino, new_path.clone());
                self.by_path.insert(new_path, ino);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn inode_forget_removes_cached_descendants() {
            let mut table = InodeTable::new();
            let parent = table.record(PathBuf::from("src")).expect("record src");
            let child = table
                .record(PathBuf::from("src/generated"))
                .expect("record child");
            let grandchild = table
                .record(PathBuf::from("src/generated/file.rs"))
                .expect("record grandchild");
            let sibling = table
                .record(PathBuf::from("tests"))
                .expect("record sibling");

            table.forget(Path::new("src"));
            assert!(table.path_for(parent).is_none());
            assert!(table.path_for(child).is_none());
            assert!(table.path_for(grandchild).is_none());
            assert_eq!(table.path_for(sibling), Some(PathBuf::from("tests")));
        }
    }
}

/// Optional W1 code-indexing controls for sync-back.
#[derive(Clone, Debug)]
pub struct CodeIndexOptions {
    pub repo_id: String,
    pub repo_root_display: String,
    pub materialize_symbol_name_index: bool,
    pub actor: String,
}

impl CodeIndexOptions {
    pub fn new(repo_id: impl Into<String>, repo_root_display: impl Into<String>) -> Self {
        Self {
            repo_id: repo_id.into(),
            repo_root_display: repo_root_display.into(),
            materialize_symbol_name_index: true,
            actor: "rustyred-workspace".to_string(),
        }
    }
}

/// Install the generic embedded post-`File`-write hook that keeps the W1
/// CodeCrawler overlay fresh for source files written directly through
/// `Engine::fs_write` / `fs_write_batch`.
pub fn install_code_index_file_write_hook(engine: &Engine, options: CodeIndexOptions) {
    engine.register_file_write_hook(
        move |engine: &Engine, writes: &[FileWrite], _receipts: &[FileWriteReceipt]| {
            for write in writes {
                if !is_code_index_candidate(&write.path) {
                    continue;
                }
                engine
                    .with_store(|store| {
                        index_source_file_write_in_store(
                            store,
                            SourceFileWriteIndexInput {
                                tenant_id: LOCAL_WORKSPACE_TENANT.to_string(),
                                repo_id: options.repo_id.clone(),
                                repo_root_display: options.repo_root_display.clone(),
                                file_path: write.path.clone(),
                                content: write.content.clone(),
                                actor: options.actor.clone(),
                                generation: 0,
                                materialize_symbol_name_index: options
                                    .materialize_symbol_name_index,
                            },
                        )
                    })
                    .map_err(|error| {
                        EngineError::Hook(format!("indexing {}: {error}", write.path))
                    })?;
            }
            Ok(())
        },
    );
}

/// A real-process toolchain command over a materialized workspace.
#[derive(Clone, Debug)]
pub struct RunPlan {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
    pub env: Vec<(String, String)>,
    pub inherit_env: Vec<String>,
}

impl RunPlan {
    pub fn new(command: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            timeout: Duration::from_secs(60),
            env: Vec::new(),
            inherit_env: default_inherited_env(),
        }
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

/// Captured result of a real-process run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunReceipt {
    pub status_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl RunReceipt {
    pub fn success(&self) -> bool {
        self.status_code == Some(0) && !self.timed_out
    }
}

/// A sandbox-backed toolchain command over a DocTree workspace.
#[derive(Clone, Debug)]
pub struct SandboxRunPlan {
    pub command: String,
    pub args: Vec<String>,
    /// Relative directory under the sandbox worktree where the command runs.
    pub workdir: PathBuf,
    pub timeout: Duration,
    /// Relative source paths to fetch after the run. Empty means fetch the files
    /// uploaded from the DocTree, which covers source rewrites.
    pub sync_paths: Vec<String>,
}

impl SandboxRunPlan {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            workdir: PathBuf::new(),
            timeout: Duration::from_secs(60),
            sync_paths: Vec::new(),
        }
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn workdir(mut self, workdir: impl Into<PathBuf>) -> Self {
        self.workdir = workdir.into();
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn sync_paths(mut self, paths: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.sync_paths = paths.into_iter().map(Into::into).collect();
        self
    }
}

/// Summary of one sandbox-backed workspace run.
#[derive(Clone, Debug, PartialEq)]
pub struct SandboxRunReceipt {
    pub files_uploaded: usize,
    pub files_synced: usize,
    pub uploaded_paths: Vec<String>,
    pub synced_paths: Vec<String>,
    pub proof: ProofReceipt,
}

/// Errors surfaced by workspace import.
#[derive(Debug)]
pub enum WorkspaceError {
    Io { path: PathBuf, message: String },
    Walk(String),
    Engine(EngineError),
    Code(String),
    Limit(String),
    Path(String),
    Run(String),
    Sandbox(String),
    Watch(String),
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceError::Io { path, message } => write!(f, "io {:?}: {message}", path),
            WorkspaceError::Walk(message) => write!(f, "walk: {message}"),
            WorkspaceError::Engine(error) => write!(f, "engine: {error}"),
            WorkspaceError::Code(message) => write!(f, "code staging: {message}"),
            WorkspaceError::Limit(message) => write!(f, "limit: {message}"),
            WorkspaceError::Path(message) => write!(f, "path: {message}"),
            WorkspaceError::Run(message) => write!(f, "run: {message}"),
            WorkspaceError::Sandbox(message) => write!(f, "sandbox: {message}"),
            WorkspaceError::Watch(message) => write!(f, "watch: {message}"),
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl From<EngineError> for WorkspaceError {
    fn from(error: EngineError) -> Self {
        WorkspaceError::Engine(error)
    }
}

/// Import an already-materialized checkout into the embedded workspace.
pub fn import_checkout(
    engine: &Engine,
    repo: impl AsRef<Path>,
    options: ImportOptions,
) -> Result<ImportReceipt, WorkspaceError> {
    import_checkout_with_clone_ms(engine, repo.as_ref(), options, 0)
}

/// Clone/stage a remote repository through CodeCrawler's existing fetch seam,
/// then import the staged checkout into the embedded workspace.
pub fn import_repo_url(
    engine: &Engine,
    url: &str,
    options: ImportOptions,
    credential: Option<&GitCredential>,
) -> Result<ImportReceipt, WorkspaceError> {
    let caps = RepoFetchCaps::from_requested(options.max_total_bytes);
    let input = IngestCodebaseInput {
        tenant_id: LOCAL_WORKSPACE_TENANT.to_string(),
        max_total_bytes: options.max_total_bytes,
        actor: "rustyred-workspace".to_string(),
        ..IngestCodebaseInput::default()
    };
    let (staged, clone_ms, fetched) =
        stage_repo_for_ingest_with_credential(input, Some((url, &caps)), credential)
            .map_err(|error| WorkspaceError::Code(error.to_string()))?;
    let _fetched: Option<FetchedRepo> = fetched;
    import_checkout_with_clone_ms(engine, Path::new(&staged.repo_path), options, clone_ms)
}

/// Use an existing checkout as the canonical real filesystem mirror and rebuild
/// the downstream DocTree/File index from it.
pub fn import_checkout_mirror(
    engine: &Engine,
    repo: impl AsRef<Path>,
    options: MirrorImportOptions,
) -> Result<MirrorImportReceipt, WorkspaceError> {
    import_checkout_mirror_with_clone_ms(engine, repo.as_ref(), options, 0)
}

/// Clone a remote repository, keep or copy the real files as the workspace
/// mirror, and rebuild the downstream DocTree/File index from that directory.
pub fn import_repo_url_mirror(
    engine: &Engine,
    url: &str,
    options: MirrorImportOptions,
    credential: Option<&GitCredential>,
) -> Result<MirrorImportReceipt, WorkspaceError> {
    let caps = RepoFetchCaps::from_requested(options.max_total_bytes);
    let input = IngestCodebaseInput {
        tenant_id: LOCAL_WORKSPACE_TENANT.to_string(),
        max_total_bytes: options.max_total_bytes,
        actor: "rustyred-workspace".to_string(),
        ..IngestCodebaseInput::default()
    };
    let (staged, clone_ms, fetched) =
        stage_repo_for_ingest_with_credential(input, Some((url, &caps)), credential)
            .map_err(|error| WorkspaceError::Code(error.to_string()))?;
    let repo = if options.workspace_root.is_none() {
        fetched
            .map(FetchedRepo::keep)
            .unwrap_or_else(|| PathBuf::from(&staged.repo_path))
    } else {
        PathBuf::from(&staged.repo_path)
    };
    import_checkout_mirror_with_clone_ms(engine, &repo, options, clone_ms)
}

/// CommonPlace repo-connect front door: one call lands real files in the
/// workspace mirror and creates the downstream File index.
pub fn connect_commonplace_repo_mirror(
    engine: &Engine,
    repo: impl AsRef<Path>,
    options: MirrorImportOptions,
) -> Result<MirrorImportReceipt, WorkspaceError> {
    import_checkout_mirror(engine, repo, options)
}

/// Build an agent session entry that works directly in the real mirror root.
/// There is no materialize step: the spawned CLI's cwd is the canonical
/// workspace directory and notify/reconcile can keep the index fresh.
pub fn agent_session_plan(
    root: impl AsRef<Path>,
    lane: &str,
    intent: &str,
) -> Result<SpawnPlan, WorkspaceError> {
    let root = canonical_root(root.as_ref())?;
    build_spawn_plan(lane, intent, &root)
        .ok_or_else(|| WorkspaceError::Run(format!("unknown agent lane {lane:?}")))
}

pub fn index_path(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
    path: impl AsRef<Path>,
) -> Result<Option<FileWriteReceipt>, WorkspaceError> {
    index_path_with_max_file_bytes(engine, root, prefix, path, DEFAULT_MAX_FILE_BYTES)
}

fn index_path_with_max_file_bytes(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
    path: impl AsRef<Path>,
    max_file_bytes: u64,
) -> Result<Option<FileWriteReceipt>, WorkspaceError> {
    let root = canonical_root(root.as_ref())?;
    let input_path = path.as_ref();
    let (path, relative) = if input_path.is_absolute() {
        let relative = input_path
            .strip_prefix(&root)
            .map_err(|error| WorkspaceError::Path(error.to_string()))?
            .to_path_buf();
        (input_path.to_path_buf(), relative)
    } else {
        let relative = safe_relative_path(input_path.to_string_lossy().as_ref())?;
        (root.join(&relative), relative)
    };
    if relative.as_os_str().is_empty() {
        return Ok(None);
    }
    let engine_path = import_path(prefix, &relative)?;
    if should_skip(&relative) || !path.exists() {
        engine.fs_remove_file_index(&engine_path)?;
        return Ok(None);
    }
    let canonical = fs::canonicalize(&path).map_err(|error| WorkspaceError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if !canonical.starts_with(&root) {
        engine.fs_remove_file_index(&engine_path)?;
        return Ok(None);
    }
    let metadata = fs::metadata(&canonical).map_err(|error| WorkspaceError::Io {
        path: canonical.clone(),
        message: error.to_string(),
    })?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let Some(body) = read_stable_file(&canonical, max_file_bytes) else {
        engine.fs_remove_file_index(&engine_path)?;
        return Ok(None);
    };
    engine
        .fs_link_real_file(&engine_path, &canonical, &body)
        .map(Some)
        .map_err(WorkspaceError::Engine)
}

pub fn index_tree(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
) -> Result<IndexReceipt, WorkspaceError> {
    reconcile(engine, root, prefix)
}

/// Runtime reconcile: rescan the captured region, index fresh/changed files,
/// and remove stale downstream File nodes and DocTree entries.
pub fn reconcile(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
) -> Result<IndexReceipt, WorkspaceError> {
    reconcile_with_limits(
        engine,
        root,
        prefix,
        DEFAULT_MAX_FILE_BYTES,
        DEFAULT_MAX_TOTAL_BYTES,
    )
}

pub fn reconcile_with_options(
    engine: &Engine,
    root: impl AsRef<Path>,
    options: &MirrorImportOptions,
) -> Result<IndexReceipt, WorkspaceError> {
    reconcile_with_limits(
        engine,
        root,
        &options.prefix,
        options.max_file_bytes,
        options.max_total_bytes,
    )
}

fn reconcile_with_max_file_bytes(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
    max_file_bytes: u64,
) -> Result<IndexReceipt, WorkspaceError> {
    reconcile_with_limits(
        engine,
        root,
        prefix,
        max_file_bytes,
        DEFAULT_MAX_TOTAL_BYTES,
    )
}

fn reconcile_with_limits(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
    max_file_bytes: u64,
    max_total_bytes: u64,
) -> Result<IndexReceipt, WorkspaceError> {
    let root = canonical_root(root.as_ref())?;
    let snapshot = filesystem_hashes(&root, prefix, max_file_bytes, max_total_bytes)?;
    let disk = snapshot.files;
    let doc = doctree_paths(engine, prefix)?;
    let files = file_node_paths(engine, prefix)?;
    let mut indexed = BTreeSet::new();
    indexed.extend(doc.keys().cloned());
    indexed.extend(files.keys().cloned());
    let mut files_removed = 0usize;
    for path in &indexed {
        if !disk.contains_key(path) {
            if engine.fs_remove_file_index(path)? {
                files_removed += 1;
            }
        }
    }

    let mut files_indexed = 0usize;
    let mut bytes_indexed = 0u64;
    let mut paths = Vec::new();
    for (engine_path, file) in disk {
        let doc_matches = doc
            .get(&engine_path)
            .is_some_and(|entry| entry.content_hash == file.content_hash);
        let file_node_matches = files
            .get(&engine_path)
            .is_some_and(|hash| hash == &file.content_hash);
        if doc_matches && file_node_matches {
            continue;
        }
        let receipt = engine.fs_link_real_file(&engine_path, &file.real_path, &file.body)?;
        bytes_indexed = bytes_indexed.saturating_add(file.body.len() as u64);
        files_indexed += 1;
        paths.push(receipt.path);
    }
    paths.sort();
    Ok(IndexReceipt {
        root,
        files_indexed,
        files_removed,
        files_skipped: snapshot.files_skipped,
        bytes_indexed,
        paths,
    })
}

/// Drop the downstream code projection under `prefix`.
///
/// This removes mirrored DocTree/File entries while leaving unrelated graph
/// content intact. Use `rebuild_code_index` when the desired end state is a
/// fresh projection of the live checkout.
pub fn drop_code_index(engine: &Engine, prefix: &str) -> Result<IndexReceipt, WorkspaceError> {
    let doc = doctree_paths(engine, prefix)?;
    let files = file_node_paths(engine, prefix)?;
    let mut paths = BTreeSet::new();
    paths.extend(doc.keys().cloned());
    paths.extend(files.keys().cloned());

    let mut files_removed = 0usize;
    let mut removed_paths = Vec::new();
    for path in paths {
        if engine.fs_remove_file_index(&path)? {
            files_removed += 1;
            removed_paths.push(path);
        }
    }
    Ok(IndexReceipt {
        root: PathBuf::new(),
        files_indexed: 0,
        files_removed,
        files_skipped: 0,
        bytes_indexed: 0,
        paths: removed_paths,
    })
}

/// Rebuild the downstream code projection from the real mirror root.
pub fn rebuild_code_index(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
) -> Result<IndexReceipt, WorkspaceError> {
    rebuild_code_index_with_max_file_bytes(engine, root, prefix, DEFAULT_MAX_FILE_BYTES)
}

pub fn rebuild_code_index_with_options(
    engine: &Engine,
    root: impl AsRef<Path>,
    options: &MirrorImportOptions,
) -> Result<IndexReceipt, WorkspaceError> {
    rebuild_code_index_with_limits(
        engine,
        root,
        &options.prefix,
        options.max_file_bytes,
        options.max_total_bytes,
    )
}

fn rebuild_code_index_with_max_file_bytes(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
    max_file_bytes: u64,
) -> Result<IndexReceipt, WorkspaceError> {
    let dropped = drop_code_index(engine, prefix)?;
    let mut rebuilt = reconcile_with_max_file_bytes(engine, root, prefix, max_file_bytes)?;
    rebuilt.files_removed = rebuilt.files_removed.saturating_add(dropped.files_removed);
    rebuilt.paths.extend(dropped.paths);
    rebuilt.paths.sort();
    rebuilt.paths.dedup();
    Ok(rebuilt)
}

fn rebuild_code_index_with_limits(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
    max_file_bytes: u64,
    max_total_bytes: u64,
) -> Result<IndexReceipt, WorkspaceError> {
    let dropped = drop_code_index(engine, prefix)?;
    let mut rebuilt = reconcile_with_limits(engine, root, prefix, max_file_bytes, max_total_bytes)?;
    rebuilt.files_removed = rebuilt.files_removed.saturating_add(dropped.files_removed);
    rebuilt.paths.extend(dropped.paths);
    rebuilt.paths.sort();
    rebuilt.paths.dedup();
    Ok(rebuilt)
}

/// Consistency oracle. It reads the live filesystem fresh on every call and
/// compares it to the downstream File nodes plus DocTree entries.
pub fn audit_consistency(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
) -> Result<Divergence, WorkspaceError> {
    audit_consistency_with_limits(
        engine,
        root,
        prefix,
        DEFAULT_MAX_FILE_BYTES,
        DEFAULT_MAX_TOTAL_BYTES,
    )
}

pub fn audit_consistency_with_options(
    engine: &Engine,
    root: impl AsRef<Path>,
    options: &MirrorImportOptions,
) -> Result<Divergence, WorkspaceError> {
    audit_consistency_with_limits(
        engine,
        root,
        &options.prefix,
        options.max_file_bytes,
        options.max_total_bytes,
    )
}

fn audit_consistency_with_limits(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
    max_file_bytes: u64,
    max_total_bytes: u64,
) -> Result<Divergence, WorkspaceError> {
    let root = canonical_root(root.as_ref())?;
    let disk = filesystem_hashes(&root, prefix, max_file_bytes, max_total_bytes)?.files;
    let doc = doctree_paths(engine, prefix)?;
    let files = file_node_paths(engine, prefix)?;
    let mut all_indexed = BTreeSet::new();
    all_indexed.extend(doc.keys().cloned());
    all_indexed.extend(files.keys().cloned());

    let mut divergence = Divergence::default();
    for (path, disk_file) in &disk {
        let doc_hash = doc.get(path).map(|entry| entry.content_hash.as_str());
        let file_hash = files.get(path).map(String::as_str);
        if doc_hash.is_none() || file_hash.is_none() {
            divergence.missing_from_index.push(path.clone());
            continue;
        }
        if doc_hash != Some(disk_file.content_hash.as_str())
            || file_hash != Some(disk_file.content_hash.as_str())
        {
            divergence.hash_mismatches.push(path.clone());
        }
    }
    for path in &all_indexed {
        let relative = materialized_relative_path(prefix, path)?;
        if should_skip(&relative) {
            divergence.excluded_index_paths.push(path.clone());
        }
        if !disk.contains_key(path) {
            divergence.indexed_but_missing_on_disk.push(path.clone());
        }
    }
    for (path, entry) in &doc {
        match entry.real_path.as_ref() {
            Some(real_path) if Path::new(real_path).is_file() => {}
            _ => divergence.doctree_without_real_file.push(path.clone()),
        }
    }
    divergence.missing_from_index.sort();
    divergence.indexed_but_missing_on_disk.sort();
    divergence.hash_mismatches.sort();
    divergence.excluded_index_paths.sort();
    divergence.doctree_without_real_file.sort();
    Ok(divergence)
}

/// Runtime audit: report divergence without changing state.
pub fn audit(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
) -> Result<AuditReceipt, WorkspaceError> {
    audit_with_limits(
        engine,
        root,
        prefix,
        DEFAULT_MAX_FILE_BYTES,
        DEFAULT_MAX_TOTAL_BYTES,
    )
}

pub fn audit_with_options(
    engine: &Engine,
    root: impl AsRef<Path>,
    options: &MirrorImportOptions,
) -> Result<AuditReceipt, WorkspaceError> {
    audit_with_limits(
        engine,
        root,
        &options.prefix,
        options.max_file_bytes,
        options.max_total_bytes,
    )
}

fn audit_with_limits(
    engine: &Engine,
    root: impl AsRef<Path>,
    prefix: &str,
    max_file_bytes: u64,
    max_total_bytes: u64,
) -> Result<AuditReceipt, WorkspaceError> {
    let root = canonical_root(root.as_ref())?;
    let divergence =
        audit_consistency_with_limits(engine, &root, prefix, max_file_bytes, max_total_bytes)?;
    Ok(AuditReceipt {
        root,
        divergence_count: divergence.count(),
        divergence,
    })
}

/// Render a small Prometheus-compatible metric block for periodic mirror audits.
pub fn render_audit_prometheus(receipt: &AuditReceipt, prefix: &str) -> String {
    let root = prometheus_label_value(&receipt.root.to_string_lossy());
    let prefix = prometheus_label_value(prefix.trim_matches('/'));
    let mut out = String::new();
    out.push_str("# HELP rustyred_workspace_mirror_divergence_count Live workspace mirror divergence count.\n");
    out.push_str("# TYPE rustyred_workspace_mirror_divergence_count gauge\n");
    out.push_str(&format!(
        "rustyred_workspace_mirror_divergence_count{{root=\"{root}\",prefix=\"{prefix}\"}} {}\n",
        receipt.divergence_count
    ));
    out.push_str("# HELP rustyred_workspace_mirror_divergence_paths Live workspace mirror divergence by class.\n");
    out.push_str("# TYPE rustyred_workspace_mirror_divergence_paths gauge\n");
    for (class, count) in [
        (
            "missing_from_index",
            receipt.divergence.missing_from_index.len(),
        ),
        (
            "indexed_but_missing_on_disk",
            receipt.divergence.indexed_but_missing_on_disk.len(),
        ),
        ("hash_mismatches", receipt.divergence.hash_mismatches.len()),
        (
            "excluded_index_paths",
            receipt.divergence.excluded_index_paths.len(),
        ),
        (
            "doctree_without_real_file",
            receipt.divergence.doctree_without_real_file.len(),
        ),
    ] {
        out.push_str(&format!(
            "rustyred_workspace_mirror_divergence_paths{{root=\"{root}\",prefix=\"{prefix}\",class=\"{class}\"}} {count}\n"
        ));
    }
    out
}

#[derive(Clone, Debug)]
pub struct MirrorAuditTarget {
    pub root: PathBuf,
    pub options: MirrorImportOptions,
}

impl MirrorAuditTarget {
    pub fn new(root: impl Into<PathBuf>, options: MirrorImportOptions) -> Self {
        Self {
            root: root.into(),
            options,
        }
    }

    fn key(&self) -> String {
        format!(
            "{}\0{}",
            self.root.display(),
            self.options.prefix.trim_matches('/')
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MirrorAuditSnapshot {
    pub prometheus: String,
    pub receipts: Vec<AuditReceipt>,
    pub errors: Vec<String>,
    pub runs: u64,
}

struct MirrorAuditState {
    targets: BTreeMap<String, MirrorAuditTarget>,
    latest: MirrorAuditSnapshot,
}

impl MirrorAuditState {
    fn new(targets: Vec<MirrorAuditTarget>) -> Self {
        let targets = targets
            .into_iter()
            .map(|target| (target.key(), target))
            .collect();
        Self {
            targets,
            latest: MirrorAuditSnapshot::default(),
        }
    }
}

/// Periodic runtime self-audit for real filesystem mirrors.
///
/// The monitor opens the embedded engine on its worker thread, runs the same
/// filesystem oracle used by tests, and caches Prometheus text for whichever
/// HTTP/service surface already owns observability.
pub struct WorkspaceMirrorAuditMonitor {
    engine_dir: PathBuf,
    engine_lock: Arc<Mutex<()>>,
    state: Arc<Mutex<MirrorAuditState>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl WorkspaceMirrorAuditMonitor {
    pub fn start(
        engine_dir: impl Into<PathBuf>,
        targets: Vec<MirrorAuditTarget>,
        interval: Duration,
    ) -> Result<Self, WorkspaceError> {
        Self::start_with_engine_lock(engine_dir, targets, interval, Arc::new(Mutex::new(())))
    }

    pub fn start_with_engine_lock(
        engine_dir: impl Into<PathBuf>,
        targets: Vec<MirrorAuditTarget>,
        interval: Duration,
        engine_lock: Arc<Mutex<()>>,
    ) -> Result<Self, WorkspaceError> {
        let engine_dir = engine_dir.into();
        let state = Arc::new(Mutex::new(MirrorAuditState::new(targets)));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_state = Arc::clone(&state);
        let thread_stop = Arc::clone(&stop);
        let thread_engine_dir = engine_dir.clone();
        let thread_engine_lock = Arc::clone(&engine_lock);
        let sleep_interval = interval.max(Duration::from_millis(1));
        let handle = thread::Builder::new()
            .name("workspace-mirror-audit".to_string())
            .spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    run_monitor_cycle(&thread_engine_dir, &thread_engine_lock, &thread_state);
                    sleep_interruptibly(sleep_interval, &thread_stop);
                }
            })
            .map_err(|error| WorkspaceError::Run(format!("spawn mirror audit: {error}")))?;
        let monitor = Self {
            engine_dir,
            engine_lock,
            state,
            stop,
            handle: Some(handle),
        };
        monitor.run_once();
        Ok(monitor)
    }

    pub fn add_target(&self, target: MirrorAuditTarget) {
        if let Ok(mut guard) = self.state.lock() {
            guard.targets.insert(target.key(), target);
        }
    }

    pub fn run_once(&self) -> MirrorAuditSnapshot {
        run_monitor_cycle(&self.engine_dir, &self.engine_lock, &self.state)
    }

    pub fn snapshot(&self) -> MirrorAuditSnapshot {
        self.state
            .lock()
            .map(|guard| guard.latest.clone())
            .unwrap_or_else(|_| MirrorAuditSnapshot {
                prometheus: render_monitor_error_prometheus(
                    0,
                    "workspace mirror audit state lock poisoned",
                ),
                receipts: Vec::new(),
                errors: vec!["workspace mirror audit state lock poisoned".to_string()],
                runs: 0,
            })
    }

    pub fn latest_prometheus(&self) -> String {
        self.snapshot().prometheus
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for WorkspaceMirrorAuditMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_monitor_cycle(
    engine_dir: &Path,
    engine_lock: &Arc<Mutex<()>>,
    state: &Arc<Mutex<MirrorAuditState>>,
) -> MirrorAuditSnapshot {
    let (targets, previous_runs) = match state.lock() {
        Ok(guard) => (
            guard.targets.values().cloned().collect::<Vec<_>>(),
            guard.latest.runs,
        ),
        Err(_) => {
            return MirrorAuditSnapshot {
                prometheus: render_monitor_error_prometheus(
                    0,
                    "workspace mirror audit state lock poisoned",
                ),
                receipts: Vec::new(),
                errors: vec!["workspace mirror audit state lock poisoned".to_string()],
                runs: 0,
            };
        }
    };
    let runs = previous_runs.saturating_add(1);
    let mut snapshot = match engine_lock.lock() {
        Ok(_guard) => match EmbeddedConfig::load_for_dir(engine_dir)
            .and_then(|config| Engine::open(engine_dir, config))
        {
            Ok(engine) => audit_targets(&engine, &targets, runs),
            Err(error) => MirrorAuditSnapshot {
                prometheus: render_monitor_error_prometheus(runs, &format!("open engine: {error}")),
                receipts: Vec::new(),
                errors: vec![format!("open engine: {error}")],
                runs,
            },
        },
        Err(_) => MirrorAuditSnapshot {
            prometheus: render_monitor_error_prometheus(
                runs,
                "workspace mirror audit engine lock poisoned",
            ),
            receipts: Vec::new(),
            errors: vec!["workspace mirror audit engine lock poisoned".to_string()],
            runs,
        },
    };
    if targets.is_empty() && snapshot.prometheus.is_empty() {
        snapshot.prometheus = render_monitor_summary_prometheus(runs, 0, 0, 0);
    }
    if let Ok(mut guard) = state.lock() {
        guard.latest = snapshot.clone();
    }
    snapshot
}

fn audit_targets(engine: &Engine, targets: &[MirrorAuditTarget], runs: u64) -> MirrorAuditSnapshot {
    let mut receipts = Vec::new();
    let mut errors = Vec::new();
    let mut prometheus = String::new();
    for target in targets {
        match audit_with_options(engine, &target.root, &target.options) {
            Ok(receipt) => {
                prometheus.push_str(&render_audit_prometheus(&receipt, &target.options.prefix));
                receipts.push(receipt);
            }
            Err(error) => errors.push(format!(
                "{}:{}: {error}",
                target.root.display(),
                target.options.prefix
            )),
        }
    }
    prometheus.push_str(&render_monitor_summary_prometheus(
        runs,
        targets.len(),
        receipts
            .iter()
            .map(|receipt| receipt.divergence_count)
            .sum(),
        errors.len(),
    ));
    for error in &errors {
        prometheus.push_str(&render_monitor_error_prometheus(runs, error));
    }
    MirrorAuditSnapshot {
        prometheus,
        receipts,
        errors,
        runs,
    }
}

fn render_monitor_summary_prometheus(
    runs: u64,
    targets: usize,
    divergence_count: usize,
    errors: usize,
) -> String {
    format!(
        "# HELP rustyred_workspace_mirror_audit_runs_total Periodic workspace mirror audit cycles run.\n\
# TYPE rustyred_workspace_mirror_audit_runs_total counter\n\
rustyred_workspace_mirror_audit_runs_total {runs}\n\
# HELP rustyred_workspace_mirror_audit_targets Number of workspace mirror audit targets.\n\
# TYPE rustyred_workspace_mirror_audit_targets gauge\n\
rustyred_workspace_mirror_audit_targets {targets}\n\
# HELP rustyred_workspace_mirror_audit_divergence_count Total divergence across all periodic audit targets.\n\
# TYPE rustyred_workspace_mirror_audit_divergence_count gauge\n\
rustyred_workspace_mirror_audit_divergence_count {divergence_count}\n\
# HELP rustyred_workspace_mirror_audit_errors Number of audit targets that failed during the last cycle.\n\
# TYPE rustyred_workspace_mirror_audit_errors gauge\n\
rustyred_workspace_mirror_audit_errors {errors}\n"
    )
}

fn render_monitor_error_prometheus(runs: u64, error: &str) -> String {
    let error = prometheus_label_value(error);
    format!(
        "# HELP rustyred_workspace_mirror_audit_error Last workspace mirror audit error by message.\n\
# TYPE rustyred_workspace_mirror_audit_error gauge\n\
rustyred_workspace_mirror_audit_error{{message=\"{error}\"}} 1\n\
# HELP rustyred_workspace_mirror_audit_runs_total Periodic workspace mirror audit cycles run.\n\
# TYPE rustyred_workspace_mirror_audit_runs_total counter\n\
rustyred_workspace_mirror_audit_runs_total {runs}\n"
    )
}

fn sleep_interruptibly(interval: Duration, stop: &AtomicBool) {
    let started = Instant::now();
    while started.elapsed() < interval && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(10).min(interval - started.elapsed()));
    }
}

type MirrorDebouncer =
    Debouncer<notify_debouncer_full::notify::RecommendedWatcher, RecommendedCache>;

/// Debounced notify watcher over a real workspace mirror. The watcher owns the
/// OS subscription; callers drain events on the engine-owning thread.
pub struct WorkspaceMirrorWatcher {
    root: PathBuf,
    prefix: String,
    max_file_bytes: u64,
    max_total_bytes: u64,
    rx: Receiver<DebounceEventResult>,
    #[cfg(test)]
    tx: Sender<DebounceEventResult>,
    debouncer: MirrorDebouncer,
}

impl WorkspaceMirrorWatcher {
    pub fn start(
        root: impl AsRef<Path>,
        prefix: &str,
        debounce: Duration,
    ) -> Result<Self, WorkspaceError> {
        Self::start_with_limits(
            root,
            prefix,
            DEFAULT_MAX_FILE_BYTES,
            DEFAULT_MAX_TOTAL_BYTES,
            debounce,
        )
    }

    pub fn start_with_options(
        root: impl AsRef<Path>,
        options: &MirrorImportOptions,
        debounce: Duration,
    ) -> Result<Self, WorkspaceError> {
        Self::start_with_limits(
            root,
            &options.prefix,
            options.max_file_bytes,
            options.max_total_bytes,
            debounce,
        )
    }

    fn start_with_limits(
        root: impl AsRef<Path>,
        prefix: &str,
        max_file_bytes: u64,
        max_total_bytes: u64,
        debounce: Duration,
    ) -> Result<Self, WorkspaceError> {
        let root = canonical_root(root.as_ref())?;
        let (tx, rx) = std::sync::mpsc::channel();
        #[cfg(test)]
        let test_tx = tx.clone();
        let mut debouncer = new_debouncer(debounce, None, tx)
            .map_err(|error| WorkspaceError::Watch(error.to_string()))?;
        debouncer
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| WorkspaceError::Watch(error.to_string()))?;
        Ok(Self {
            root,
            prefix: prefix.trim_matches('/').to_string(),
            max_file_bytes,
            max_total_bytes,
            rx,
            #[cfg(test)]
            tx: test_tx,
            debouncer,
        })
    }

    pub fn start_reconciled(
        engine: &Engine,
        root: impl AsRef<Path>,
        prefix: &str,
        debounce: Duration,
    ) -> Result<(Self, IndexReceipt), WorkspaceError> {
        let options = MirrorImportOptions {
            prefix: prefix.to_string(),
            ..MirrorImportOptions::default()
        };
        Self::start_reconciled_with_options(engine, root, &options, debounce)
    }

    pub fn start_reconciled_with_options(
        engine: &Engine,
        root: impl AsRef<Path>,
        options: &MirrorImportOptions,
        debounce: Duration,
    ) -> Result<(Self, IndexReceipt), WorkspaceError> {
        let root = canonical_root(root.as_ref())?;
        let receipt = reconcile_with_options(engine, &root, options)?;
        let watcher = Self::start_with_options(&root, options, debounce)?;
        Ok((watcher, receipt))
    }

    pub fn drain(&mut self, engine: &Engine) -> Result<IndexReceipt, WorkspaceError> {
        let mut saw_event = false;
        loop {
            match self.rx.try_recv() {
                Ok(Ok(_events)) => saw_event = true,
                Ok(Err(_errors)) => saw_event = true,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(WorkspaceError::Watch(
                        "workspace mirror watcher channel disconnected".to_string(),
                    ));
                }
            }
        }
        if saw_event {
            reconcile_with_limits(
                engine,
                &self.root,
                &self.prefix,
                self.max_file_bytes,
                self.max_total_bytes,
            )
        } else {
            Ok(IndexReceipt {
                root: self.root.clone(),
                files_indexed: 0,
                files_removed: 0,
                files_skipped: 0,
                bytes_indexed: 0,
                paths: Vec::new(),
            })
        }
    }

    pub fn force_rescan(&mut self, engine: &Engine) -> Result<IndexReceipt, WorkspaceError> {
        reconcile_with_limits(
            engine,
            &self.root,
            &self.prefix,
            self.max_file_bytes,
            self.max_total_bytes,
        )
    }

    #[cfg(test)]
    fn inject_event_result_for_test(&self, result: DebounceEventResult) {
        self.tx.send(result).expect("inject watcher event result");
    }

    pub fn audit(&self, engine: &Engine) -> Result<AuditReceipt, WorkspaceError> {
        audit_with_limits(
            engine,
            &self.root,
            &self.prefix,
            self.max_file_bytes,
            self.max_total_bytes,
        )
    }

    pub fn audit_prometheus(&self, engine: &Engine) -> Result<String, WorkspaceError> {
        let receipt = self.audit(engine)?;
        Ok(render_audit_prometheus(&receipt, &self.prefix))
    }

    pub fn wait_until_consistent(
        &mut self,
        engine: &Engine,
        timeout: Duration,
    ) -> Result<Divergence, WorkspaceError> {
        let started = Instant::now();
        loop {
            self.drain(engine)?;
            let divergence = audit_consistency_with_limits(
                engine,
                &self.root,
                &self.prefix,
                self.max_file_bytes,
                self.max_total_bytes,
            )?;
            if divergence.is_empty() {
                return Ok(divergence);
            }
            if started.elapsed() >= timeout {
                return Ok(divergence);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn stop(self) {
        self.debouncer.stop();
    }
}

/// Project an engine workspace subtree into a real OS directory.
///
/// Paths are listed from `prefix`, then materialized relative to `dir` with the
/// prefix stripped. This gives W3 a real filesystem for `cargo`, `python`, and
/// `node` without making build artifacts part of the DocTree.
pub fn materialize_workspace(
    engine: &Engine,
    prefix: &str,
    dir: impl AsRef<Path>,
) -> Result<MaterializeReceipt, WorkspaceError> {
    let root = dir.as_ref().to_path_buf();
    fs::create_dir_all(&root).map_err(|error| WorkspaceError::Io {
        path: root.clone(),
        message: error.to_string(),
    })?;

    for engine_path in engine.list_directories(prefix)? {
        let relative = materialized_relative_path(prefix, &engine_path)?;
        if should_skip(&relative) {
            continue;
        }
        fs::create_dir_all(root.join(&relative)).map_err(|error| WorkspaceError::Io {
            path: root.join(&relative),
            message: error.to_string(),
        })?;
    }

    let paths = engine.list_paths(prefix)?;
    let mut written = Vec::new();
    let mut bytes_written = 0u64;
    for engine_path in paths {
        let relative = materialized_relative_path(prefix, &engine_path)?;
        if should_skip(&relative) {
            continue;
        }
        let Some(body) = engine.fs_read(&engine_path)? else {
            continue;
        };
        let out_path = root.join(&relative);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|error| WorkspaceError::Io {
                path: parent.to_path_buf(),
                message: error.to_string(),
            })?;
        }
        fs::write(&out_path, &body).map_err(|error| WorkspaceError::Io {
            path: out_path,
            message: error.to_string(),
        })?;
        bytes_written = bytes_written.saturating_add(body.len() as u64);
        written.push(relative_to_string(&relative)?);
    }
    written.sort();
    Ok(MaterializeReceipt {
        root,
        files_written: written.len(),
        bytes_written,
        paths: written,
    })
}

/// Run a real process over a materialized workspace.
///
/// Environment is deny-by-default: only `inherit_env` keys are copied from the
/// parent, and sensitive variables are stripped even when supplied explicitly.
pub fn run_tool(plan: &RunPlan) -> Result<RunReceipt, WorkspaceError> {
    let mut command = Command::new(&plan.command);
    command
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in &plan.inherit_env {
        if !is_sensitive_env(key) {
            if let Ok(value) = std::env::var(key) {
                command.env(key, value);
            }
        }
    }
    for (key, value) in &plan.env {
        if !is_sensitive_env(key) {
            command.env(key, value);
        }
    }

    let mut child = command
        .spawn()
        .map_err(|error| WorkspaceError::Run(format!("spawn {:?}: {error}", plan.command)))?;
    let started = Instant::now();
    let mut timed_out = false;
    loop {
        if child
            .try_wait()
            .map_err(|error| WorkspaceError::Run(error.to_string()))?
            .is_some()
        {
            break;
        }
        if started.elapsed() >= plan.timeout {
            timed_out = true;
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child
        .wait_with_output()
        .map_err(|error| WorkspaceError::Run(error.to_string()))?;
    Ok(RunReceipt {
        status_code: output.status.code(),
        timed_out,
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

/// Sync source files from a real OS directory back into the engine workspace.
///
/// Build artifacts, hidden paths, and binary files are skipped. The synced files
/// use the same batch write path as W0 so later W1 hooks can coalesce on one
/// batch rather than per-file full persists.
pub fn sync_back_sources(
    engine: &Engine,
    dir: impl AsRef<Path>,
    prefix: &str,
) -> Result<SyncBackReceipt, WorkspaceError> {
    sync_back_sources_inner(engine, dir.as_ref(), prefix, None)
}

/// Sync source files back into the DocTree and update the W1 code graph for
/// indexable source files using the bytes already read from disk.
pub fn sync_back_sources_indexed(
    engine: &Engine,
    dir: impl AsRef<Path>,
    prefix: &str,
    code_index: CodeIndexOptions,
) -> Result<SyncBackReceipt, WorkspaceError> {
    sync_back_sources_inner(engine, dir.as_ref(), prefix, Some(&code_index))
}

fn sync_back_sources_inner(
    engine: &Engine,
    dir: &Path,
    prefix: &str,
    code_index: Option<&CodeIndexOptions>,
) -> Result<SyncBackReceipt, WorkspaceError> {
    let root = fs::canonicalize(dir).map_err(|error| WorkspaceError::Io {
        path: dir.to_path_buf(),
        message: error.to_string(),
    })?;
    let mut writes = Vec::new();
    let mut files_skipped = 0usize;
    let mut bytes_synced = 0u64;

    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true);

    for entry in builder.build() {
        let entry = entry.map_err(|error| WorkspaceError::Walk(error.to_string()))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let relative = path
            .strip_prefix(&root)
            .map_err(|error| WorkspaceError::Path(error.to_string()))?;
        if should_skip(relative) {
            files_skipped += 1;
            continue;
        }
        let body = fs::read(path).map_err(|error| WorkspaceError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        if is_binary(&body) {
            files_skipped += 1;
            continue;
        }
        let code_path = relative_to_string(relative)?;
        let engine_path = import_path(prefix, relative)?;
        if engine.fs_read(&engine_path)?.as_deref() == Some(body.as_slice()) {
            continue;
        }
        bytes_synced = bytes_synced.saturating_add(body.len() as u64);
        writes.push((code_path, FileWrite::new(engine_path, body)));
    }

    writes.sort_by(|left, right| left.1.path.cmp(&right.1.path));
    let receipts = engine.fs_write_batch(writes.iter().map(|(_, write)| write.clone()))?;
    let mut code_symbols_indexed = 0u64;
    let mut code_edges_indexed = 0u64;
    let mut code_edges_retired = 0u64;
    let mut code_bucket_lookups = 0u64;
    if let Some(options) = code_index {
        for (code_path, write) in &writes {
            if !is_code_index_candidate(code_path) {
                continue;
            }
            let output = engine.with_store(|store| {
                index_source_file_write_in_store(
                    store,
                    SourceFileWriteIndexInput {
                        tenant_id: LOCAL_WORKSPACE_TENANT.to_string(),
                        repo_id: options.repo_id.clone(),
                        repo_root_display: options.repo_root_display.clone(),
                        file_path: code_path.clone(),
                        content: write.content.clone(),
                        actor: options.actor.clone(),
                        generation: 0,
                        materialize_symbol_name_index: options.materialize_symbol_name_index,
                    },
                )
            });
            let output = output.map_err(|error| WorkspaceError::Code(error.message))?;
            code_symbols_indexed = code_symbols_indexed.saturating_add(output.symbols_indexed);
            code_edges_indexed = code_edges_indexed.saturating_add(output.edges_indexed);
            code_edges_retired = code_edges_retired.saturating_add(output.edges_retired);
            code_bucket_lookups = code_bucket_lookups.saturating_add(output.bucket_lookups);
        }
    }
    let paths = receipts
        .into_iter()
        .map(|receipt| receipt.path)
        .collect::<Vec<_>>();
    Ok(SyncBackReceipt {
        root,
        files_synced: paths.len(),
        files_skipped,
        bytes_synced,
        paths,
        code_symbols_indexed,
        code_edges_indexed,
        code_edges_retired,
        code_bucket_lookups,
    })
}

/// Run a DocTree workspace through a [`SandboxRuntime`].
///
/// This is the W3 sidecar bridge: files leave the engine through `put_files`,
/// the command runs against the sandbox's target worktree, and fetched source
/// files return through the same W0 batch write path used by local sync-back.
pub fn run_workspace_in_sandbox<R: SandboxRuntime>(
    engine: &Engine,
    prefix: &str,
    runtime: &R,
    request: SandboxProvisionRequest,
    plan: SandboxRunPlan,
) -> Result<SandboxRunReceipt, WorkspaceError> {
    let files = sandbox_files_from_workspace(engine, prefix)?;
    let uploaded_paths = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let handle = runtime
        .provision(request)
        .map_err(|error| WorkspaceError::Sandbox(error.to_string()))?;
    let run_result = (|| {
        runtime
            .put_files(&handle, &files)
            .map_err(|error| WorkspaceError::Sandbox(error.to_string()))?;
        let cwd = sandbox_cwd(&handle.target_worktree, &plan.workdir)?;
        let proof_plan = ProofPlan::new(&plan.command, plan.args.clone(), cwd, plan.timeout);
        let proof = runtime
            .run(&handle, &proof_plan)
            .map_err(|error| WorkspaceError::Sandbox(error.to_string()))?;
        let sync_paths = sandbox_sync_paths(&uploaded_paths, &plan.sync_paths)?;
        let fetched = runtime
            .get_files(&handle, &sync_paths)
            .map_err(|error| WorkspaceError::Sandbox(error.to_string()))?;
        let synced_paths = sync_sandbox_files(engine, prefix, fetched)?;
        Ok(SandboxRunReceipt {
            files_uploaded: uploaded_paths.len(),
            files_synced: synced_paths.len(),
            uploaded_paths,
            synced_paths,
            proof,
        })
    })();
    let destroy_result = runtime
        .destroy(&handle)
        .map_err(|error| WorkspaceError::Sandbox(error.to_string()));
    match (run_result, destroy_result) {
        (Ok(receipt), Ok(())) => Ok(receipt),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

/// Run a DocTree workspace through a [`SandboxRuntime`] with live output events
/// and cooperative cancellation.
pub fn run_workspace_in_sandbox_streaming<R: SandboxRuntime>(
    engine: &Engine,
    prefix: &str,
    runtime: &R,
    request: SandboxProvisionRequest,
    plan: SandboxRunPlan,
    cancel: &SandboxCancelToken,
    on_event: &mut dyn FnMut(&SandboxStreamEvent),
) -> Result<SandboxRunReceipt, WorkspaceError> {
    let files = sandbox_files_from_workspace(engine, prefix)?;
    let uploaded_paths = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let handle = runtime
        .provision(request)
        .map_err(|error| WorkspaceError::Sandbox(error.to_string()))?;
    let run_result = (|| {
        runtime
            .put_files(&handle, &files)
            .map_err(|error| WorkspaceError::Sandbox(error.to_string()))?;
        let cwd = sandbox_cwd(&handle.target_worktree, &plan.workdir)?;
        let proof_plan = ProofPlan::new(&plan.command, plan.args.clone(), cwd, plan.timeout);
        let proof = runtime
            .run_streaming(&handle, &proof_plan, cancel, on_event)
            .map_err(|error| WorkspaceError::Sandbox(error.to_string()))?;
        let sync_paths = sandbox_sync_paths(&uploaded_paths, &plan.sync_paths)?;
        let fetched = runtime
            .get_files(&handle, &sync_paths)
            .map_err(|error| WorkspaceError::Sandbox(error.to_string()))?;
        let synced_paths = sync_sandbox_files(engine, prefix, fetched)?;
        Ok(SandboxRunReceipt {
            files_uploaded: uploaded_paths.len(),
            files_synced: synced_paths.len(),
            uploaded_paths,
            synced_paths,
            proof,
        })
    })();
    let destroy_result = runtime
        .destroy(&handle)
        .map_err(|error| WorkspaceError::Sandbox(error.to_string()));
    match (run_result, destroy_result) {
        (Ok(receipt), Ok(())) => Ok(receipt),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

#[derive(Clone, Debug)]
struct DiskFile {
    real_path: PathBuf,
    body: Vec<u8>,
    content_hash: String,
}

#[derive(Clone, Debug)]
struct DocIndexEntry {
    content_hash: String,
    real_path: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct FilesystemSnapshot {
    files: BTreeMap<String, DiskFile>,
    files_skipped: usize,
}

fn import_checkout_mirror_with_clone_ms(
    engine: &Engine,
    repo: &Path,
    options: MirrorImportOptions,
    clone_ms: u64,
) -> Result<MirrorImportReceipt, WorkspaceError> {
    let repo = fs::canonicalize(repo).map_err(|error| WorkspaceError::Io {
        path: repo.to_path_buf(),
        message: error.to_string(),
    })?;
    let (root, copied) = prepare_mirror_root(&repo, options.workspace_root.as_deref(), &options)?;
    let index = reconcile_with_options(engine, &root, &options)?;
    let files_skipped = copied.files_skipped.saturating_add(index.files_skipped);
    Ok(MirrorImportReceipt {
        root,
        files_mirrored: copied.files_mirrored,
        files_skipped,
        bytes_mirrored: copied.bytes_mirrored,
        paths: index.paths.clone(),
        index,
        clone_ms,
    })
}

#[derive(Clone, Debug, Default)]
struct CopyReceipt {
    files_mirrored: usize,
    files_skipped: usize,
    bytes_mirrored: u64,
}

fn prepare_mirror_root(
    repo: &Path,
    workspace_root: Option<&Path>,
    options: &MirrorImportOptions,
) -> Result<(PathBuf, CopyReceipt), WorkspaceError> {
    let repo = fs::canonicalize(repo).map_err(|error| WorkspaceError::Io {
        path: repo.to_path_buf(),
        message: error.to_string(),
    })?;
    let Some(root) = workspace_root else {
        return Ok((repo, CopyReceipt::default()));
    };
    fs::create_dir_all(root).map_err(|error| WorkspaceError::Io {
        path: root.to_path_buf(),
        message: error.to_string(),
    })?;
    let root = fs::canonicalize(root).map_err(|error| WorkspaceError::Io {
        path: root.to_path_buf(),
        message: error.to_string(),
    })?;
    if repo == root {
        return Ok((root, CopyReceipt::default()));
    }
    clear_directory_contents(&root)?;

    let mut receipt = CopyReceipt::default();
    let mut builder = WalkBuilder::new(&repo);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true);
    for entry in builder.build() {
        let entry = entry.map_err(|error| WorkspaceError::Walk(error.to_string()))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(&repo)
            .map_err(|error| WorkspaceError::Path(error.to_string()))?;
        if should_skip(relative) {
            receipt.files_skipped += 1;
            continue;
        }
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            fs::create_dir_all(root.join(relative)).map_err(|error| WorkspaceError::Io {
                path: root.join(relative),
                message: error.to_string(),
            })?;
            continue;
        }
        if !entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| WorkspaceError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        if metadata.len() > options.max_file_bytes {
            receipt.files_skipped += 1;
            continue;
        }
        let next_total = receipt.bytes_mirrored.saturating_add(metadata.len());
        if next_total > options.max_total_bytes {
            return Err(WorkspaceError::Limit(format!(
                "mirror exceeds max_total_bytes {} at {:?}",
                options.max_total_bytes, relative
            )));
        }
        if let Some(parent) = root.join(relative).parent() {
            fs::create_dir_all(parent).map_err(|error| WorkspaceError::Io {
                path: parent.to_path_buf(),
                message: error.to_string(),
            })?;
        }
        let bytes = fs::copy(path, root.join(relative)).map_err(|error| WorkspaceError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        receipt.files_mirrored += 1;
        receipt.bytes_mirrored = receipt.bytes_mirrored.saturating_add(bytes);
    }
    Ok((root, receipt))
}

fn clear_directory_contents(root: &Path) -> Result<(), WorkspaceError> {
    for entry in fs::read_dir(root).map_err(|error| WorkspaceError::Io {
        path: root.to_path_buf(),
        message: error.to_string(),
    })? {
        let entry = entry.map_err(|error| WorkspaceError::Io {
            path: root.to_path_buf(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| WorkspaceError::Io {
            path: path.clone(),
            message: error.to_string(),
        })?;
        if file_type.is_dir() {
            fs::remove_dir_all(&path).map_err(|error| WorkspaceError::Io {
                path,
                message: error.to_string(),
            })?;
        } else {
            fs::remove_file(&path).map_err(|error| WorkspaceError::Io {
                path,
                message: error.to_string(),
            })?;
        }
    }
    Ok(())
}

fn filesystem_hashes(
    root: &Path,
    prefix: &str,
    max_file_bytes: u64,
    max_total_bytes: u64,
) -> Result<FilesystemSnapshot, WorkspaceError> {
    let mut snapshot = FilesystemSnapshot::default();
    let mut bytes_seen = 0u64;
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .follow_links(true);
    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                return Err(WorkspaceError::Walk(error.to_string()));
            }
        };
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|error| WorkspaceError::Path(error.to_string()))?;
        if relative.as_os_str().is_empty() || should_skip(relative) {
            if !relative.as_os_str().is_empty() {
                snapshot.files_skipped += 1;
            }
            continue;
        }
        let canonical = match fs::canonicalize(path) {
            Ok(canonical) => canonical,
            Err(_error) => continue,
        };
        if !canonical.starts_with(root) {
            continue;
        }
        let metadata = match fs::metadata(&canonical) {
            Ok(metadata) => metadata,
            Err(_error) => continue,
        };
        if !metadata.is_file() {
            continue;
        }
        if metadata.len() > max_file_bytes {
            snapshot.files_skipped += 1;
            continue;
        }
        let next_total = bytes_seen.saturating_add(metadata.len());
        if next_total > max_total_bytes {
            return Err(WorkspaceError::Limit(format!(
                "mirror scan exceeds max_total_bytes {max_total_bytes} at {:?}",
                relative
            )));
        }
        let Some(body) = read_stable_file(&canonical, max_file_bytes) else {
            snapshot.files_skipped += 1;
            continue;
        };
        bytes_seen = bytes_seen.saturating_add(body.len() as u64);
        if bytes_seen > max_total_bytes {
            return Err(WorkspaceError::Limit(format!(
                "mirror scan exceeds max_total_bytes {max_total_bytes} at {:?}",
                relative
            )));
        }
        let engine_path = import_path(prefix, relative)?;
        snapshot.files.insert(
            engine_path,
            DiskFile {
                real_path: canonical,
                content_hash: Engine::file_content_hash(&body),
                body,
            },
        );
    }
    Ok(snapshot)
}

fn doctree_paths(
    engine: &Engine,
    prefix: &str,
) -> Result<BTreeMap<String, DocIndexEntry>, WorkspaceError> {
    let prefix = prefix.trim_matches('/').to_string();
    engine.with_doc_tree(|tree| {
        let entries = if prefix.is_empty() {
            tree.range_prefix(b"")
                .map(|(key, entry)| (key.to_slash_path(), entry.clone()))
                .collect::<Vec<_>>()
        } else {
            let segments = prefix.split('/').filter(|part| !part.is_empty());
            let prefix_key = match rustyred_thg_core::PathKey::prefix_from_segments(segments) {
                Ok(key) => key,
                Err(error) => return Err(WorkspaceError::Path(error.message)),
            };
            tree.range_prefix(prefix_key.as_bytes())
                .map(|(key, entry)| (key.to_slash_path(), entry.clone()))
                .collect::<Vec<_>>()
        };
        let mut out = BTreeMap::new();
        for (path, entry) in entries {
            if let Some(content_hash) = entry.content_hash {
                out.insert(
                    path,
                    DocIndexEntry {
                        content_hash,
                        real_path: entry.real_path,
                    },
                );
            }
        }
        Ok(out)
    })
}

fn file_node_paths(
    engine: &Engine,
    prefix: &str,
) -> Result<BTreeMap<String, String>, WorkspaceError> {
    let prefix = prefix.trim_matches('/').to_string();
    engine.with_store(|store| {
        let nodes = store
            .query_nodes(NodeQuery::label("File").with_limit(100_000))
            .map_err(|error| WorkspaceError::Engine(EngineError::Graphql(error.message)))?;
        let mut out = BTreeMap::new();
        for node in nodes {
            let Some(path) = node
                .properties
                .get(DOC_TREE_PATH_PROPERTY)
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            if !path_in_prefix(path, &prefix) {
                continue;
            }
            let Some(hash) = node
                .properties
                .get(DOC_TREE_CONTENT_HASH_PROPERTY)
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            out.insert(path.to_string(), hash.to_string());
        }
        Ok(out)
    })
}

fn canonical_root(root: &Path) -> Result<PathBuf, WorkspaceError> {
    fs::create_dir_all(root).map_err(|error| WorkspaceError::Io {
        path: root.to_path_buf(),
        message: error.to_string(),
    })?;
    fs::canonicalize(root).map_err(|error| WorkspaceError::Io {
        path: root.to_path_buf(),
        message: error.to_string(),
    })
}

fn path_in_prefix(path: &str, prefix: &str) -> bool {
    prefix.is_empty() || path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn read_stable_file(path: &Path, max_file_bytes: u64) -> Option<Vec<u8>> {
    let mut last_body = None;
    for attempt in 0..5 {
        let before = fs::metadata(path).ok()?;
        if !before.is_file() || before.len() > max_file_bytes {
            return None;
        }
        let body = fs::read(path).ok()?;
        let after = fs::metadata(path).ok()?;
        if !after.is_file() || after.len() > max_file_bytes {
            return None;
        }
        if before.len() == after.len() && metadata_modified_equal(&before, &after) {
            return Some(body);
        }
        last_body = Some(body);
        if attempt < 4 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    last_body
}

fn metadata_modified_equal(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    match (left.modified(), right.modified()) {
        (Ok(left), Ok(right)) => left == right,
        _ => true,
    }
}

fn prometheus_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn import_checkout_with_clone_ms(
    engine: &Engine,
    repo: &Path,
    options: ImportOptions,
    clone_ms: u64,
) -> Result<ImportReceipt, WorkspaceError> {
    let repo = fs::canonicalize(repo).map_err(|error| WorkspaceError::Io {
        path: repo.to_path_buf(),
        message: error.to_string(),
    })?;

    let mut writes = Vec::new();
    let mut files_skipped = 0usize;
    let mut bytes_imported = 0u64;

    let mut builder = WalkBuilder::new(&repo);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true);

    for entry in builder.build() {
        let entry = entry.map_err(|error| WorkspaceError::Walk(error.to_string()))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let relative = path
            .strip_prefix(&repo)
            .map_err(|error| WorkspaceError::Path(error.to_string()))?;
        if should_skip(relative) {
            files_skipped += 1;
            continue;
        }

        let metadata = entry.metadata().map_err(|error| WorkspaceError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        if metadata.len() > options.max_file_bytes {
            files_skipped += 1;
            continue;
        }

        let body = fs::read(path).map_err(|error| WorkspaceError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        if is_binary(&body) {
            files_skipped += 1;
            continue;
        }
        let next_total = bytes_imported.saturating_add(body.len() as u64);
        if next_total > options.max_total_bytes {
            return Err(WorkspaceError::Limit(format!(
                "import exceeds max_total_bytes {} at {:?}",
                options.max_total_bytes, relative
            )));
        }
        bytes_imported = next_total;

        writes.push(FileWrite::new(
            import_path(&options.prefix, relative)?,
            body,
        ));
    }

    writes.sort_by(|left, right| left.path.cmp(&right.path));
    let receipts = engine.fs_write_batch(writes)?;
    Ok(receipt(
        repo,
        receipts,
        files_skipped,
        bytes_imported,
        clone_ms,
    ))
}

fn receipt(
    root: PathBuf,
    receipts: Vec<FileWriteReceipt>,
    files_skipped: usize,
    bytes_imported: u64,
    clone_ms: u64,
) -> ImportReceipt {
    let paths = receipts
        .into_iter()
        .map(|receipt| receipt.path)
        .collect::<Vec<_>>();
    ImportReceipt {
        root,
        files_imported: paths.len(),
        files_skipped,
        bytes_imported,
        paths,
        clone_ms,
    }
}

fn materialized_relative_path(prefix: &str, engine_path: &str) -> Result<PathBuf, WorkspaceError> {
    let prefix = prefix.trim_matches('/');
    let relative = if prefix.is_empty() {
        engine_path
    } else if let Some(rest) = engine_path.strip_prefix(prefix) {
        rest.trim_start_matches('/')
    } else {
        return Err(WorkspaceError::Path(format!(
            "path {engine_path:?} is outside prefix {prefix:?}"
        )));
    };
    safe_relative_path(relative)
}

fn safe_relative_path(path: &str) -> Result<PathBuf, WorkspaceError> {
    let path = Path::new(path);
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => out.push(value),
            Component::CurDir => {}
            _ => {
                return Err(WorkspaceError::Path(format!(
                    "unsafe workspace path {:?}",
                    path
                )));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(WorkspaceError::Path("empty workspace path".to_string()));
    }
    Ok(out)
}

fn safe_mount_path(path: &str) -> Result<PathBuf, WorkspaceError> {
    let path = path.trim_matches('/');
    if path.is_empty() {
        return Ok(PathBuf::new());
    }
    safe_relative_path(path)
}

fn mount_engine_path(prefix: &str, relative: &Path) -> Result<String, WorkspaceError> {
    if relative.as_os_str().is_empty() {
        return Err(WorkspaceError::Path(
            "mount file path cannot be empty".to_string(),
        ));
    }
    import_path(prefix, relative)
}

fn mount_engine_prefix(prefix: &str, relative: &Path) -> Result<String, WorkspaceError> {
    if relative.as_os_str().is_empty() {
        Ok(prefix.trim_matches('/').to_string())
    } else {
        import_path(prefix, relative)
    }
}

fn relative_to_string(path: &Path) -> Result<String, WorkspaceError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_str().ok_or_else(|| {
                WorkspaceError::Path(format!("non-utf8 path component in {:?}", path))
            })?),
            Component::CurDir => {}
            _ => {
                return Err(WorkspaceError::Path(format!(
                    "unsupported workspace path {:?}",
                    path
                )));
            }
        }
    }
    Ok(parts.join("/"))
}

fn import_path(prefix: &str, relative: &Path) -> Result<String, WorkspaceError> {
    let mut parts = Vec::new();
    parts.extend(prefix.split('/').filter(|part| !part.is_empty()));
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_str().ok_or_else(|| {
                WorkspaceError::Path(format!("non-utf8 path component in {:?}", relative))
            })?),
            Component::CurDir => {}
            _ => {
                return Err(WorkspaceError::Path(format!(
                    "unsupported checkout path {:?}",
                    relative
                )));
            }
        }
    }
    if parts.is_empty() {
        return Err(WorkspaceError::Path("empty import path".to_string()));
    }
    Ok(parts.join("/"))
}

fn sandbox_files_from_workspace(
    engine: &Engine,
    prefix: &str,
) -> Result<Vec<SandboxFile>, WorkspaceError> {
    let paths = engine.list_paths(prefix)?;
    let mut files = Vec::new();
    for engine_path in paths {
        let relative = materialized_relative_path(prefix, &engine_path)?;
        if should_skip(&relative) {
            continue;
        }
        let Some(content) = engine.fs_read(&engine_path)? else {
            continue;
        };
        if is_binary(&content) {
            continue;
        }
        files.push(SandboxFile {
            path: relative_to_string(&relative)?,
            content,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn sandbox_cwd(root: &Path, workdir: &Path) -> Result<PathBuf, WorkspaceError> {
    if workdir.as_os_str().is_empty() {
        return Ok(root.to_path_buf());
    }
    Ok(root.join(safe_relative_dir(workdir)?))
}

fn safe_relative_dir(path: &Path) -> Result<PathBuf, WorkspaceError> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => out.push(value),
            Component::CurDir => {}
            _ => {
                return Err(WorkspaceError::Path(format!(
                    "unsafe sandbox workdir {:?}",
                    path
                )));
            }
        }
    }
    Ok(out)
}

fn sandbox_sync_paths(
    uploaded_paths: &[String],
    requested_paths: &[String],
) -> Result<Vec<String>, WorkspaceError> {
    let source = if requested_paths.is_empty() {
        uploaded_paths
    } else {
        requested_paths
    };
    let mut paths = Vec::new();
    for path in source {
        let relative = safe_relative_path(path)?;
        if should_skip(&relative) {
            continue;
        }
        paths.push(relative_to_string(&relative)?);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn sync_sandbox_files(
    engine: &Engine,
    prefix: &str,
    files: Vec<SandboxFile>,
) -> Result<Vec<String>, WorkspaceError> {
    let mut writes = Vec::new();
    for file in files {
        let relative = safe_relative_path(&file.path)?;
        if should_skip(&relative) || is_binary(&file.content) {
            continue;
        }
        writes.push(FileWrite::new(
            import_path(prefix, &relative)?,
            file.content,
        ));
    }
    writes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(engine
        .fs_write_batch(writes)?
        .into_iter()
        .map(|receipt| receipt.path)
        .collect())
}

fn should_skip(relative: &Path) -> bool {
    relative.components().any(|component| {
        let Component::Normal(value) = component else {
            return true;
        };
        if value
            .to_str()
            .map(|part| part.starts_with('.'))
            .unwrap_or(true)
        {
            return true;
        }
        matches!(
            value.to_str(),
            Some("target") | Some("node_modules") | Some("dist") | Some("build") | Some("coverage")
        )
    })
}

fn is_code_index_candidate(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some(
            "rs" | "go"
                | "swift"
                | "py"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "mjs"
                | "cjs"
                | "proto"
                | "toml"
                | "md"
                | "json"
        )
    )
}

fn is_binary(body: &[u8]) -> bool {
    body.contains(&0)
}

fn default_inherited_env() -> Vec<String> {
    [
        "PATH",
        "HOME",
        "TMPDIR",
        "TEMP",
        "TMP",
        "CARGO_HOME",
        "RUSTUP_HOME",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn is_sensitive_env(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.contains("TOKEN")
        || upper.contains("SECRET")
        || upper.contains("PASSWORD")
        || upper.contains("API_KEY")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_embedded::{EmbeddedConfig, Engine};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;
    use tempfile::TempDir;

    const MIRROR_FUZZ_OPERATION_KINDS: usize = 11;
    const MIRROR_FUZZ_MIN_OPERATIONS: usize = 33;

    #[test]
    fn imports_checkout_with_gitignore_artifact_filter_restart_proof() {
        let checkout = TempDir::new().expect("checkout tempdir");
        let large_body = "x".repeat(5000);
        write(checkout.path().join(".gitignore"), "ignored.log\n");
        write(checkout.path().join("README.md"), "# fixture\n");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        );
        write(checkout.path().join("src/large.txt"), large_body.as_bytes());
        write(
            checkout.path().join("ignored.log"),
            "ignored by gitignore\n",
        );
        write(checkout.path().join("target/debug/app"), "artifact\n");
        write(checkout.path().join(".git/config"), "[core]\n");

        let data = TempDir::new().expect("engine tempdir");
        let config = EmbeddedConfig::from_toml_str("durability = \"always\"\n").expect("config");
        let engine = Engine::open(data.path(), config.clone()).expect("open engine");
        let receipt =
            import_checkout(&engine, checkout.path(), ImportOptions::default()).expect("import");

        assert_eq!(
            receipt.paths,
            vec![
                "README.md".to_string(),
                "src/large.txt".to_string(),
                "src/lib.rs".to_string(),
            ],
            "imports source files in deterministic order"
        );
        assert_eq!(receipt.files_imported, 3);
        assert!(
            receipt.files_skipped >= 1,
            "artifact files are counted as skipped: {receipt:?}"
        );

        let listing = engine.list_paths("").expect("list paths");
        assert_eq!(listing, receipt.paths);
        for path in &receipt.paths {
            assert!(
                engine.fs_read(path).expect("fs_read").is_some(),
                "{path} must be readable"
            );
            let query = format!("query{{ graphNode(id:\"file:{path}\") }}");
            let node = engine.query(&query).expect("graph node");
            assert!(!node["graphNode"].is_null(), "{path} must be a File node");
        }
        assert!(
            engine
                .fs_read("target/debug/app")
                .expect("target read")
                .is_none(),
            "build artifacts stay out of the DocTree"
        );
        assert!(
            engine.fs_read(".git/config").expect("git read").is_none(),
            ".git stays out of the DocTree"
        );

        drop(engine);
        let reopened = Engine::open(data.path(), config).expect("reopen engine");
        assert_eq!(
            reopened
                .fs_read("src/large.txt")
                .expect("large read")
                .as_deref(),
            Some(large_body.as_bytes()),
            "overflow bodies rehydrate after restart"
        );
        let direct_node = reopened
            .with_store(|store| store.get_node("file:src/lib.rs"))
            .expect("direct store get_node")
            .expect("direct file node after restart");
        let node = reopened
            .query("query{ graphNode(id:\"file:src/lib.rs\") }")
            .expect("graph node after restart");
        assert!(
            !node["graphNode"].is_null(),
            "File nodes survive the durable store restart; direct={direct_node:?}, graphql={node}"
        );
    }

    #[test]
    fn import_prefix_places_checkout_under_workspace_subtree() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(checkout.path().join("src/main.rs"), "fn main() {}\n");

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let receipt = import_checkout(
            &engine,
            checkout.path(),
            ImportOptions {
                prefix: "repos/demo".to_string(),
                ..ImportOptions::default()
            },
        )
        .expect("import");

        assert_eq!(receipt.paths, vec!["repos/demo/src/main.rs".to_string()]);
        assert_eq!(
            engine
                .fs_read("repos/demo/src/main.rs")
                .expect("read prefixed file")
                .as_deref(),
            Some(&b"fn main() {}\n"[..])
        );
    }

    #[test]
    fn materialized_workspace_runs_cargo_build_and_keeps_artifacts_out() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("Cargo.toml"),
            "[package]\nname = \"w3_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        );
        write(checkout.path().join("src/main.rs"), "fn main() {}\n");

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout(&engine, checkout.path(), ImportOptions::default()).expect("import");

        let run_dir = TempDir::new().expect("run tempdir");
        let materialized = materialize_workspace(&engine, "", run_dir.path()).expect("materialize");
        assert_eq!(
            materialized.paths,
            vec!["Cargo.toml".to_string(), "src/main.rs".to_string()]
        );

        let receipt = run_tool(
            &RunPlan::new("cargo", run_dir.path())
                .args(["build", "--quiet"])
                .timeout(Duration::from_secs(90))
                .env(
                    "CARGO_TARGET_DIR",
                    run_dir.path().join("target").to_string_lossy().into_owned(),
                ),
        )
        .expect("run cargo");
        assert!(
            receipt.success(),
            "cargo build should succeed; stderr={}",
            String::from_utf8_lossy(&receipt.stderr)
        );
        assert!(
            run_dir.path().join("target").exists(),
            "cargo creates target on disk"
        );

        let synced = sync_back_sources(&engine, run_dir.path(), "").expect("sync back");
        assert!(
            synced.paths.iter().any(|path| path == "Cargo.lock"),
            "source-ish tool output like Cargo.lock syncs back"
        );
        assert!(
            engine
                .list_paths("")
                .expect("list")
                .iter()
                .all(|path| !path.starts_with("target/")),
            "build artifacts never sync into DocTree"
        );
    }

    #[test]
    fn materialized_workspace_creates_explicit_empty_directories() {
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        engine
            .fs_mkdir("src/generated")
            .expect("mkdir empty generated");
        engine
            .fs_write("src/lib.rs", b"pub fn lib() {}\n")
            .expect("write lib");

        let run_dir = TempDir::new().expect("run tempdir");
        let materialized = materialize_workspace(&engine, "", run_dir.path()).expect("materialize");
        assert_eq!(materialized.paths, vec!["src/lib.rs".to_string()]);
        assert!(
            run_dir.path().join("src/generated").is_dir(),
            "explicit empty directories materialize as real directories"
        );
        assert!(
            run_dir.path().join("src/lib.rs").is_file(),
            "files still materialize normally"
        );
    }

    #[test]
    fn run_rewrite_syncs_source_and_strips_sensitive_env() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(checkout.path().join("src/main.rs"), "fn main() {}\n");

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout(&engine, checkout.path(), ImportOptions::default()).expect("import");

        let run_dir = TempDir::new().expect("run tempdir");
        materialize_workspace(&engine, "", run_dir.path()).expect("materialize");
        let receipt = run_tool(
            &RunPlan::new("/bin/sh", run_dir.path())
                .args([
                    "-c",
                    "test -z \"$ANTHROPIC_API_KEY\" && printf 'fn main() { println!(\"updated\"); }\\n' > src/main.rs",
                ])
                .timeout(Duration::from_secs(10))
                .env("ANTHROPIC_API_KEY", "should-not-leak"),
        )
        .expect("rewrite");
        assert!(
            receipt.success(),
            "rewrite should succeed and secret should be absent; stderr={}",
            String::from_utf8_lossy(&receipt.stderr)
        );

        sync_back_sources(&engine, run_dir.path(), "").expect("sync");
        assert_eq!(
            engine.fs_read("src/main.rs").expect("read").as_deref(),
            Some(&b"fn main() { println!(\"updated\"); }\n"[..])
        );
    }

    #[cfg(feature = "fuse-host")]
    #[test]
    fn fuse_host_feature_builds_single_threaded_rw_adapter() {
        struct FixtureBackend;

        impl fuse_host::DocTreeFuseBackend for FixtureBackend {
            fn read_file(&self, _path: &str) -> Result<Option<Vec<u8>>, WorkspaceError> {
                Ok(None)
            }

            fn write_file(
                &self,
                _path: &str,
                _content: &[u8],
            ) -> Result<MountWriteDisposition, WorkspaceError> {
                Ok(MountWriteDisposition::Stored)
            }

            fn mkdir(&self, _path: &str) -> Result<MountMakeDirDisposition, WorkspaceError> {
                Ok(MountMakeDirDisposition::Created)
            }

            fn unlink(&self, _path: &str) -> Result<MountUnlinkDisposition, WorkspaceError> {
                Ok(MountUnlinkDisposition::Missing)
            }

            fn rmdir(&self, _path: &str) -> Result<MountRemoveDirDisposition, WorkspaceError> {
                Ok(MountRemoveDirDisposition::Missing)
            }

            fn rename(
                &self,
                _from: &str,
                _to: &str,
            ) -> Result<MountRenameDisposition, WorkspaceError> {
                Ok(MountRenameDisposition::Missing)
            }

            fn getattr(&self, path: &str) -> Result<Option<MountMetadata>, WorkspaceError> {
                Ok(if path.is_empty() {
                    Some(MountMetadata {
                        kind: MountEntryKind::Directory,
                        len: 0,
                    })
                } else {
                    None
                })
            }

            fn read_dir(&self, _path: &str) -> Result<Vec<MountDirEntry>, WorkspaceError> {
                Ok(Vec::new())
            }
        }

        let _host = fuse_host::DocTreeFuseHost::new(FixtureBackend);
        let config = fuse_host::default_fuse_config();
        assert_eq!(config.n_threads, Some(1));
        assert!(
            config.mount_options.contains(&fuser::MountOption::RW),
            "DocTree FUSE host is writable"
        );
    }

    #[cfg(feature = "fuse-host")]
    #[test]
    fn fuse_host_attr_policy_controls_permissions_and_owner() {
        struct FixtureBackend;

        impl fuse_host::DocTreeFuseBackend for FixtureBackend {
            fn read_file(&self, _path: &str) -> Result<Option<Vec<u8>>, WorkspaceError> {
                Ok(None)
            }

            fn write_file(
                &self,
                _path: &str,
                _content: &[u8],
            ) -> Result<MountWriteDisposition, WorkspaceError> {
                Ok(MountWriteDisposition::Stored)
            }

            fn mkdir(&self, _path: &str) -> Result<MountMakeDirDisposition, WorkspaceError> {
                Ok(MountMakeDirDisposition::Created)
            }

            fn unlink(&self, _path: &str) -> Result<MountUnlinkDisposition, WorkspaceError> {
                Ok(MountUnlinkDisposition::Missing)
            }

            fn rmdir(&self, _path: &str) -> Result<MountRemoveDirDisposition, WorkspaceError> {
                Ok(MountRemoveDirDisposition::Missing)
            }

            fn rename(
                &self,
                _from: &str,
                _to: &str,
            ) -> Result<MountRenameDisposition, WorkspaceError> {
                Ok(MountRenameDisposition::Missing)
            }

            fn getattr(&self, path: &str) -> Result<Option<MountMetadata>, WorkspaceError> {
                Ok(match path {
                    "" => Some(MountMetadata {
                        kind: MountEntryKind::Directory,
                        len: 0,
                    }),
                    "src/lib.rs" => Some(MountMetadata {
                        kind: MountEntryKind::File,
                        len: 17,
                    }),
                    _ => None,
                })
            }

            fn read_dir(&self, _path: &str) -> Result<Vec<MountDirEntry>, WorkspaceError> {
                Ok(Vec::new())
            }
        }

        let host = fuse_host::DocTreeFuseHost::with_attr_policy(
            FixtureBackend,
            fuse_host::FuseAttrPolicy {
                file_perm: 0o640,
                dir_perm: 0o750,
                uid: 501,
                gid: 20,
                block_size: 8192,
            },
        );
        let (_, root_attr) = host
            .test_metadata_for_path(Path::new(""))
            .expect("root attr")
            .expect("root exists");
        assert_eq!(root_attr.perm, 0o750);
        assert_eq!(root_attr.uid, 501);
        assert_eq!(root_attr.gid, 20);
        assert_eq!(root_attr.blksize, 8192);

        let (_, file_attr) = host
            .test_metadata_for_path(Path::new("src/lib.rs"))
            .expect("file attr")
            .expect("file exists");
        assert_eq!(file_attr.perm, 0o640);
        assert_eq!(file_attr.uid, 501);
        assert_eq!(file_attr.gid, 20);
        assert_eq!(file_attr.blksize, 8192);
        assert_eq!(file_attr.size, 17);
    }

    #[cfg(feature = "fuse-host")]
    #[test]
    fn fuse_host_forget_evicts_descendant_write_buffers() {
        struct FixtureBackend;

        impl fuse_host::DocTreeFuseBackend for FixtureBackend {
            fn read_file(&self, _path: &str) -> Result<Option<Vec<u8>>, WorkspaceError> {
                Ok(None)
            }

            fn write_file(
                &self,
                _path: &str,
                _content: &[u8],
            ) -> Result<MountWriteDisposition, WorkspaceError> {
                Ok(MountWriteDisposition::Stored)
            }

            fn mkdir(&self, _path: &str) -> Result<MountMakeDirDisposition, WorkspaceError> {
                Ok(MountMakeDirDisposition::Created)
            }

            fn unlink(&self, _path: &str) -> Result<MountUnlinkDisposition, WorkspaceError> {
                Ok(MountUnlinkDisposition::Missing)
            }

            fn rmdir(&self, _path: &str) -> Result<MountRemoveDirDisposition, WorkspaceError> {
                Ok(MountRemoveDirDisposition::Missing)
            }

            fn rename(
                &self,
                _from: &str,
                _to: &str,
            ) -> Result<MountRenameDisposition, WorkspaceError> {
                Ok(MountRenameDisposition::Missing)
            }

            fn getattr(&self, path: &str) -> Result<Option<MountMetadata>, WorkspaceError> {
                Ok(if path.is_empty() {
                    Some(MountMetadata {
                        kind: MountEntryKind::Directory,
                        len: 0,
                    })
                } else {
                    None
                })
            }

            fn read_dir(&self, _path: &str) -> Result<Vec<MountDirEntry>, WorkspaceError> {
                Ok(Vec::new())
            }
        }

        let host = fuse_host::DocTreeFuseHost::new(FixtureBackend);
        let parent = host
            .test_record_path(Path::new("target"))
            .expect("record parent");
        let child = host
            .test_record_path(Path::new("target/debug/app"))
            .expect("record child");
        let sibling = host
            .test_record_path(Path::new("src/lib.rs"))
            .expect("record sibling");
        host.test_seed_write_buffer(parent, b"dir-buffer".to_vec());
        host.test_seed_write_buffer(child, b"artifact-buffer".to_vec());
        host.test_seed_write_buffer(sibling, b"source-buffer".to_vec());

        host.test_forget_path(Path::new("target"))
            .expect("forget target");
        assert!(host.test_path_for(parent).is_none());
        assert!(host.test_path_for(child).is_none());
        assert!(!host.test_has_write_buffer(parent));
        assert!(!host.test_has_write_buffer(child));
        assert_eq!(
            host.test_path_for(sibling),
            Some(PathBuf::from("src/lib.rs"))
        );
        assert!(host.test_has_write_buffer(sibling));
    }

    #[cfg(feature = "fuse-host")]
    #[test]
    fn fuse_host_backs_throwaway_artifacts_with_disk_tree() {
        struct FixtureBackend;

        impl fuse_host::DocTreeFuseBackend for FixtureBackend {
            fn read_file(&self, _path: &str) -> Result<Option<Vec<u8>>, WorkspaceError> {
                Ok(None)
            }

            fn write_file(
                &self,
                path: &str,
                _content: &[u8],
            ) -> Result<MountWriteDisposition, WorkspaceError> {
                Ok(if should_skip(Path::new(path)) {
                    MountWriteDisposition::Throwaway
                } else {
                    MountWriteDisposition::Stored
                })
            }

            fn mkdir(&self, path: &str) -> Result<MountMakeDirDisposition, WorkspaceError> {
                Ok(if should_skip(Path::new(path)) {
                    MountMakeDirDisposition::Throwaway
                } else {
                    MountMakeDirDisposition::Created
                })
            }

            fn unlink(&self, path: &str) -> Result<MountUnlinkDisposition, WorkspaceError> {
                Ok(if should_skip(Path::new(path)) {
                    MountUnlinkDisposition::Throwaway
                } else {
                    MountUnlinkDisposition::Missing
                })
            }

            fn rmdir(&self, path: &str) -> Result<MountRemoveDirDisposition, WorkspaceError> {
                Ok(if should_skip(Path::new(path)) {
                    MountRemoveDirDisposition::Throwaway
                } else {
                    MountRemoveDirDisposition::Missing
                })
            }

            fn rename(
                &self,
                from: &str,
                to: &str,
            ) -> Result<MountRenameDisposition, WorkspaceError> {
                Ok(
                    if should_skip(Path::new(from)) || should_skip(Path::new(to)) {
                        MountRenameDisposition::Throwaway
                    } else {
                        MountRenameDisposition::Missing
                    },
                )
            }

            fn getattr(&self, path: &str) -> Result<Option<MountMetadata>, WorkspaceError> {
                Ok(if path.is_empty() {
                    Some(MountMetadata {
                        kind: MountEntryKind::Directory,
                        len: 0,
                    })
                } else {
                    None
                })
            }

            fn read_dir(&self, _path: &str) -> Result<Vec<MountDirEntry>, WorkspaceError> {
                Ok(Vec::new())
            }
        }

        let host = fuse_host::DocTreeFuseHost::new(FixtureBackend);
        host.test_mkdir(Path::new("target")).expect("mkdir target");
        host.test_mkdir(Path::new("target/debug"))
            .expect("mkdir debug");
        assert_eq!(
            host.test_write_file(Path::new("target/debug/app"), b"compiled")
                .expect("artifact write"),
            MountWriteDisposition::Throwaway
        );
        assert_eq!(
            host.test_read_file(Path::new("target/debug/app"))
                .expect("artifact read")
                .as_deref(),
            Some(&b"compiled"[..])
        );
        let (_, attr) = host
            .test_metadata_for_path(Path::new("target/debug/app"))
            .expect("artifact attr")
            .expect("artifact exists");
        assert_eq!(attr.size, 8);
        assert_eq!(
            host.test_read_dir(Path::new("target")).expect("target dir"),
            vec![MountDirEntry {
                name: "debug".to_string(),
                kind: MountEntryKind::Directory,
            }]
        );

        host.test_rename(
            Path::new("target/debug/app"),
            Path::new("target/debug/app2"),
        )
        .expect("artifact rename");
        assert_eq!(
            host.test_read_file(Path::new("target/debug/app"))
                .expect("old artifact"),
            None
        );
        assert_eq!(
            host.test_read_file(Path::new("target/debug/app2"))
                .expect("renamed artifact")
                .as_deref(),
            Some(&b"compiled"[..])
        );
        host.test_unlink(Path::new("target/debug/app2"))
            .expect("artifact unlink");
        assert_eq!(
            host.test_read_file(Path::new("target/debug/app2"))
                .expect("removed artifact"),
            None
        );
        host.test_rmdir(Path::new("target/debug"))
            .expect("rmdir debug");
        host.test_rmdir(Path::new("target")).expect("rmdir target");
    }

    #[cfg(feature = "fuse-host")]
    #[test]
    fn fuse_host_renames_between_throwaway_and_source_boundaries() {
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct FixtureBackend {
            source: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        }

        impl fuse_host::DocTreeFuseBackend for FixtureBackend {
            fn read_file(&self, path: &str) -> Result<Option<Vec<u8>>, WorkspaceError> {
                Ok(self.source.lock().expect("source lock").get(path).cloned())
            }

            fn write_file(
                &self,
                path: &str,
                content: &[u8],
            ) -> Result<MountWriteDisposition, WorkspaceError> {
                if should_skip(Path::new(path)) {
                    return Ok(MountWriteDisposition::Throwaway);
                }
                self.source
                    .lock()
                    .expect("source lock")
                    .insert(path.to_string(), content.to_vec());
                Ok(MountWriteDisposition::Stored)
            }

            fn mkdir(&self, path: &str) -> Result<MountMakeDirDisposition, WorkspaceError> {
                Ok(if should_skip(Path::new(path)) {
                    MountMakeDirDisposition::Throwaway
                } else {
                    MountMakeDirDisposition::Created
                })
            }

            fn unlink(&self, path: &str) -> Result<MountUnlinkDisposition, WorkspaceError> {
                if should_skip(Path::new(path)) {
                    return Ok(MountUnlinkDisposition::Throwaway);
                }
                Ok(
                    if self
                        .source
                        .lock()
                        .expect("source lock")
                        .remove(path)
                        .is_some()
                    {
                        MountUnlinkDisposition::Removed
                    } else {
                        MountUnlinkDisposition::Missing
                    },
                )
            }

            fn rmdir(&self, path: &str) -> Result<MountRemoveDirDisposition, WorkspaceError> {
                Ok(if should_skip(Path::new(path)) {
                    MountRemoveDirDisposition::Throwaway
                } else {
                    MountRemoveDirDisposition::Missing
                })
            }

            fn rename(
                &self,
                from: &str,
                to: &str,
            ) -> Result<MountRenameDisposition, WorkspaceError> {
                if should_skip(Path::new(from)) || should_skip(Path::new(to)) {
                    return Ok(MountRenameDisposition::Throwaway);
                }
                let mut source = self.source.lock().expect("source lock");
                let Some(body) = source.remove(from) else {
                    return Ok(MountRenameDisposition::Missing);
                };
                source.insert(to.to_string(), body);
                Ok(MountRenameDisposition::Renamed)
            }

            fn getattr(&self, path: &str) -> Result<Option<MountMetadata>, WorkspaceError> {
                if path.is_empty() {
                    return Ok(Some(MountMetadata {
                        kind: MountEntryKind::Directory,
                        len: 0,
                    }));
                }
                Ok(self
                    .source
                    .lock()
                    .expect("source lock")
                    .get(path)
                    .map(|body| MountMetadata {
                        kind: MountEntryKind::File,
                        len: body.len() as u64,
                    }))
            }

            fn read_dir(&self, _path: &str) -> Result<Vec<MountDirEntry>, WorkspaceError> {
                Ok(Vec::new())
            }
        }

        let backend = FixtureBackend::default();
        let source = Arc::clone(&backend.source);
        let host = fuse_host::DocTreeFuseHost::new(backend);

        host.test_write_file(Path::new(".lib.rs.tmp"), b"from temp")
            .expect("write hidden temp");
        host.test_rename(Path::new(".lib.rs.tmp"), Path::new("src/lib.rs"))
            .expect("temp into source");
        assert_eq!(
            source
                .lock()
                .expect("source lock")
                .get("src/lib.rs")
                .map(Vec::as_slice),
            Some(&b"from temp"[..]),
            "artifact-to-source rename enters the backend instead of staying throwaway"
        );
        assert_eq!(
            host.test_read_file(Path::new(".lib.rs.tmp"))
                .expect("old temp read"),
            None
        );

        host.test_rename(Path::new("src/lib.rs"), Path::new("target/debug/lib.rs"))
            .expect("source into artifact");
        assert!(
            !source
                .lock()
                .expect("source lock")
                .contains_key("src/lib.rs"),
            "source-to-artifact rename removes the DocTree/source entry"
        );
        assert_eq!(
            host.test_read_file(Path::new("target/debug/lib.rs"))
                .expect("artifact read")
                .as_deref(),
            Some(&b"from temp"[..])
        );
    }

    #[cfg(feature = "fuse-host")]
    #[test]
    fn fuse_host_renames_directory_trees_across_throwaway_boundary() {
        use std::collections::{BTreeMap, BTreeSet};
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct TreeBackend {
            files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
            dirs: Arc<Mutex<BTreeSet<String>>>,
        }

        impl TreeBackend {
            fn seed_file(&self, path: &str, body: &[u8]) {
                self.files
                    .lock()
                    .expect("files lock")
                    .insert(path.to_string(), body.to_vec());
                self.seed_parent_dirs(path);
            }

            fn seed_parent_dirs(&self, path: &str) {
                let mut dirs = self.dirs.lock().expect("dirs lock");
                let mut parts = path.split('/').collect::<Vec<_>>();
                parts.pop();
                let mut current = String::new();
                for part in parts {
                    if !current.is_empty() {
                        current.push('/');
                    }
                    current.push_str(part);
                    dirs.insert(current.clone());
                }
            }

            fn has_descendant(&self, path: &str) -> bool {
                let prefix = child_prefix(path);
                self.files
                    .lock()
                    .expect("files lock")
                    .keys()
                    .any(|candidate| candidate.starts_with(&prefix))
                    || self
                        .dirs
                        .lock()
                        .expect("dirs lock")
                        .iter()
                        .any(|candidate| candidate.starts_with(&prefix))
            }
        }

        impl fuse_host::DocTreeFuseBackend for TreeBackend {
            fn read_file(&self, path: &str) -> Result<Option<Vec<u8>>, WorkspaceError> {
                Ok(self.files.lock().expect("files lock").get(path).cloned())
            }

            fn write_file(
                &self,
                path: &str,
                content: &[u8],
            ) -> Result<MountWriteDisposition, WorkspaceError> {
                if should_skip(Path::new(path)) {
                    return Ok(MountWriteDisposition::Throwaway);
                }
                self.seed_file(path, content);
                Ok(MountWriteDisposition::Stored)
            }

            fn mkdir(&self, path: &str) -> Result<MountMakeDirDisposition, WorkspaceError> {
                if should_skip(Path::new(path)) {
                    return Ok(MountMakeDirDisposition::Throwaway);
                }
                if self.files.lock().expect("files lock").contains_key(path) {
                    return Ok(MountMakeDirDisposition::FileExists);
                }
                let inserted = self
                    .dirs
                    .lock()
                    .expect("dirs lock")
                    .insert(path.to_string());
                Ok(if inserted {
                    MountMakeDirDisposition::Created
                } else {
                    MountMakeDirDisposition::AlreadyExists
                })
            }

            fn unlink(&self, path: &str) -> Result<MountUnlinkDisposition, WorkspaceError> {
                if should_skip(Path::new(path)) {
                    return Ok(MountUnlinkDisposition::Throwaway);
                }
                Ok(
                    if self
                        .files
                        .lock()
                        .expect("files lock")
                        .remove(path)
                        .is_some()
                    {
                        MountUnlinkDisposition::Removed
                    } else {
                        MountUnlinkDisposition::Missing
                    },
                )
            }

            fn rmdir(&self, path: &str) -> Result<MountRemoveDirDisposition, WorkspaceError> {
                if should_skip(Path::new(path)) {
                    return Ok(MountRemoveDirDisposition::Throwaway);
                }
                if self.files.lock().expect("files lock").contains_key(path) {
                    return Ok(MountRemoveDirDisposition::NotDirectory);
                }
                if self.has_descendant(path) {
                    return Ok(MountRemoveDirDisposition::NotEmpty);
                }
                Ok(if self.dirs.lock().expect("dirs lock").remove(path) {
                    MountRemoveDirDisposition::Removed
                } else {
                    MountRemoveDirDisposition::Missing
                })
            }

            fn rename(
                &self,
                from: &str,
                to: &str,
            ) -> Result<MountRenameDisposition, WorkspaceError> {
                if should_skip(Path::new(from)) || should_skip(Path::new(to)) {
                    return Ok(MountRenameDisposition::Throwaway);
                }
                if let Some(body) = self.files.lock().expect("files lock").remove(from) {
                    self.seed_file(to, &body);
                    return Ok(MountRenameDisposition::Renamed);
                }
                let from_prefix = child_prefix(from);
                let mut moved_any = false;
                let files = self
                    .files
                    .lock()
                    .expect("files lock")
                    .iter()
                    .filter_map(|(path, body)| {
                        path.strip_prefix(&from_prefix)
                            .map(|suffix| (path.clone(), format!("{to}/{suffix}"), body.clone()))
                    })
                    .collect::<Vec<_>>();
                for (old, new, body) in files {
                    self.files.lock().expect("files lock").remove(&old);
                    self.seed_file(&new, &body);
                    moved_any = true;
                }
                let dirs = self
                    .dirs
                    .lock()
                    .expect("dirs lock")
                    .iter()
                    .filter_map(|path| {
                        if path == from {
                            Some((path.clone(), to.to_string()))
                        } else {
                            path.strip_prefix(&from_prefix)
                                .map(|suffix| (path.clone(), format!("{to}/{suffix}")))
                        }
                    })
                    .collect::<Vec<_>>();
                for (old, new) in dirs {
                    self.dirs.lock().expect("dirs lock").remove(&old);
                    self.dirs.lock().expect("dirs lock").insert(new);
                    moved_any = true;
                }
                Ok(if moved_any {
                    MountRenameDisposition::Renamed
                } else {
                    MountRenameDisposition::Missing
                })
            }

            fn getattr(&self, path: &str) -> Result<Option<MountMetadata>, WorkspaceError> {
                if path.is_empty() {
                    return Ok(Some(MountMetadata {
                        kind: MountEntryKind::Directory,
                        len: 0,
                    }));
                }
                if let Some(body) = self.files.lock().expect("files lock").get(path) {
                    return Ok(Some(MountMetadata {
                        kind: MountEntryKind::File,
                        len: body.len() as u64,
                    }));
                }
                Ok(
                    if self.dirs.lock().expect("dirs lock").contains(path)
                        || self.has_descendant(path)
                    {
                        Some(MountMetadata {
                            kind: MountEntryKind::Directory,
                            len: 0,
                        })
                    } else {
                        None
                    },
                )
            }

            fn read_dir(&self, path: &str) -> Result<Vec<MountDirEntry>, WorkspaceError> {
                let prefix = if path.is_empty() {
                    String::new()
                } else {
                    child_prefix(path)
                };
                let mut entries = BTreeMap::<String, MountEntryKind>::new();
                for candidate in self.files.lock().expect("files lock").keys() {
                    if let Some((name, kind)) =
                        child_entry(&prefix, candidate, MountEntryKind::File)
                    {
                        entries.insert(name, kind);
                    }
                }
                for candidate in self.dirs.lock().expect("dirs lock").iter() {
                    if let Some((name, kind)) =
                        child_entry(&prefix, candidate, MountEntryKind::Directory)
                    {
                        entries
                            .entry(name)
                            .and_modify(|existing| {
                                if kind == MountEntryKind::Directory {
                                    *existing = MountEntryKind::Directory;
                                }
                            })
                            .or_insert(kind);
                    }
                }
                Ok(entries
                    .into_iter()
                    .map(|(name, kind)| MountDirEntry { name, kind })
                    .collect())
            }
        }

        fn child_prefix(path: &str) -> String {
            if path.is_empty() {
                String::new()
            } else {
                format!("{path}/")
            }
        }

        fn child_entry(
            prefix: &str,
            candidate: &str,
            leaf_kind: MountEntryKind,
        ) -> Option<(String, MountEntryKind)> {
            let rest = candidate.strip_prefix(prefix)?;
            if rest.is_empty() {
                return None;
            }
            let mut parts = rest.split('/');
            let name = parts.next()?.to_string();
            let kind = if parts.next().is_some() {
                MountEntryKind::Directory
            } else {
                leaf_kind
            };
            Some((name, kind))
        }

        let backend = TreeBackend::default();
        let files = Arc::clone(&backend.files);
        let host = fuse_host::DocTreeFuseHost::new(backend.clone());

        host.test_mkdir(Path::new("target/generated"))
            .expect("mkdir artifact dir");
        host.test_mkdir(Path::new("target/generated/nested"))
            .expect("mkdir nested artifact dir");
        host.test_write_file(Path::new("target/generated/mod.rs"), b"pub mod nested;\n")
            .expect("write artifact mod");
        host.test_write_file(
            Path::new("target/generated/nested/item.rs"),
            b"pub fn item() {}\n",
        )
        .expect("write artifact nested");
        host.test_rename(Path::new("target/generated"), Path::new("src/generated"))
            .expect("artifact dir into source");
        assert_eq!(
            files
                .lock()
                .expect("files lock")
                .get("src/generated/nested/item.rs")
                .map(Vec::as_slice),
            Some(&b"pub fn item() {}\n"[..])
        );
        assert_eq!(
            host.test_read_file(Path::new("target/generated/mod.rs"))
                .expect("old artifact dir file"),
            None
        );

        host.test_rename(Path::new("src/generated"), Path::new("target/generated2"))
            .expect("source dir into artifact");
        assert!(
            !files
                .lock()
                .expect("files lock")
                .contains_key("src/generated/mod.rs"),
            "source directory files are removed from backend after moving to artifact"
        );
        assert_eq!(
            host.test_read_file(Path::new("target/generated2/nested/item.rs"))
                .expect("artifact tree read")
                .as_deref(),
            Some(&b"pub fn item() {}\n"[..])
        );
    }

    #[cfg(feature = "fuse-host")]
    #[test]
    fn engine_fuse_backend_round_trips_source_ops_through_actor() {
        use crate::fuse_host::DocTreeFuseBackend;

        let data = TempDir::new().expect("engine tempdir");
        let backend = fuse_host::EngineFuseBackend::open(
            data.path().to_path_buf(),
            EmbeddedConfig::default(),
            "repos/demo",
        )
        .expect("open actor backend");
        let body = b"pub fn answer() -> u8 { 42 }\n";

        backend.write_file("src/lib.rs", body).expect("actor write");
        assert_eq!(
            backend.read_file("src/lib.rs").expect("actor read"),
            Some(body.to_vec())
        );
        assert_eq!(
            backend.getattr("src/lib.rs").expect("file attr"),
            Some(MountMetadata {
                kind: MountEntryKind::File,
                len: body.len() as u64,
            })
        );
        assert_eq!(
            backend.read_dir("").expect("root dir"),
            vec![MountDirEntry {
                name: "src".to_string(),
                kind: MountEntryKind::Directory,
            }]
        );
        assert_eq!(
            backend.read_dir("src").expect("src dir"),
            vec![MountDirEntry {
                name: "lib.rs".to_string(),
                kind: MountEntryKind::File,
            }]
        );

        assert_eq!(
            backend
                .rename("src/lib.rs", "src/main.rs")
                .expect("actor rename"),
            MountRenameDisposition::Renamed
        );
        assert_eq!(backend.read_file("src/lib.rs").expect("old read"), None);
        assert_eq!(
            backend.read_file("src/main.rs").expect("new read"),
            Some(body.to_vec())
        );
        assert_eq!(
            backend.unlink("src/main.rs").expect("actor unlink"),
            MountUnlinkDisposition::Removed
        );
        assert_eq!(backend.getattr("src/main.rs").expect("removed attr"), None);
    }

    #[cfg(feature = "fuse-host")]
    #[test]
    fn engine_fuse_backend_runs_setup_hooks_inside_actor() {
        use crate::fuse_host::DocTreeFuseBackend;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        let data = TempDir::new().expect("engine tempdir");
        let hook_count = Arc::new(AtomicUsize::new(0));
        let hook_count_for_setup = Arc::clone(&hook_count);
        let backend = fuse_host::EngineFuseBackend::open_with_setup(
            data.path().to_path_buf(),
            EmbeddedConfig::default(),
            "",
            move |engine| {
                let hook_count_for_hook = Arc::clone(&hook_count_for_setup);
                engine.register_file_write_hook(
                    move |_engine: &Engine,
                          writes: &[FileWrite],
                          _receipts: &[FileWriteReceipt]| {
                        hook_count_for_hook.fetch_add(writes.len(), Ordering::SeqCst);
                        Ok(())
                    },
                );
                Ok(())
            },
        )
        .expect("open actor backend with setup");

        backend
            .write_file("src/lib.rs", b"pub fn hooked() {}\n")
            .expect("actor write");
        assert_eq!(
            hook_count.load(Ordering::SeqCst),
            1,
            "setup hook runs inside the engine actor on FUSE writes"
        );
    }

    #[cfg(feature = "fuse-host")]
    #[test]
    fn engine_fuse_backend_round_trips_directory_ops_through_actor() {
        use crate::fuse_host::DocTreeFuseBackend;

        let data = TempDir::new().expect("engine tempdir");
        let backend = fuse_host::EngineFuseBackend::open(
            data.path().to_path_buf(),
            EmbeddedConfig::default(),
            "",
        )
        .expect("open actor backend");
        backend
            .write_file("src/lib.rs", b"pub fn lib() {}\n")
            .expect("write lib");
        backend
            .write_file("src/nested/mod.rs", b"pub mod nested {}\n")
            .expect("write nested");
        assert_eq!(
            backend.mkdir("src/generated").expect("mkdir generated"),
            MountMakeDirDisposition::Created
        );
        assert_eq!(
            backend.mkdir("src/generated").expect("mkdir existing"),
            MountMakeDirDisposition::AlreadyExists
        );
        assert_eq!(
            backend.getattr("src/generated").expect("generated attr"),
            Some(MountMetadata {
                kind: MountEntryKind::Directory,
                len: 0,
            })
        );

        assert_eq!(
            backend.rmdir("src").expect("non-empty rmdir"),
            MountRemoveDirDisposition::NotEmpty
        );
        assert_eq!(
            backend
                .rename("src", "crates/app/src")
                .expect("directory rename"),
            MountRenameDisposition::Renamed
        );
        assert_eq!(backend.read_file("src/lib.rs").expect("old lib"), None);
        assert_eq!(
            backend
                .read_file("crates/app/src/lib.rs")
                .expect("new lib")
                .as_deref(),
            Some(&b"pub fn lib() {}\n"[..])
        );
        assert_eq!(
            backend
                .read_file("crates/app/src/nested/mod.rs")
                .expect("new nested")
                .as_deref(),
            Some(&b"pub mod nested {}\n"[..])
        );
        assert_eq!(
            backend
                .getattr("crates/app/src/generated")
                .expect("renamed empty dir"),
            Some(MountMetadata {
                kind: MountEntryKind::Directory,
                len: 0,
            })
        );
        assert_eq!(
            backend
                .rmdir("crates/app/src/generated")
                .expect("remove empty dir"),
            MountRemoveDirDisposition::Removed
        );
        assert_eq!(
            backend
                .getattr("crates/app/src/generated")
                .expect("removed generated attr"),
            None
        );
    }

    #[test]
    fn doc_tree_mount_core_reads_writes_lists_and_fires_hooks() {
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let hook_count = Rc::new(Cell::new(0usize));
        let hook_count_for_hook = Rc::clone(&hook_count);
        engine.register_file_write_hook(
            move |_engine: &Engine, writes: &[FileWrite], _receipts: &[FileWriteReceipt]| {
                hook_count_for_hook.set(hook_count_for_hook.get() + writes.len());
                Ok(())
            },
        );

        let mount = DocTreeMount::new(&engine, "repos/demo");
        let write = mount
            .write_file("src/lib.rs", b"pub fn answer() -> u8 { 42 }\n")
            .expect("mount write");
        assert_eq!(
            write,
            MountWriteReceipt {
                path: "src/lib.rs".to_string(),
                bytes_written: 29,
                disposition: MountWriteDisposition::Stored,
            }
        );
        assert_eq!(hook_count.get(), 1, "mount writes use the W0 write seam");
        assert_eq!(
            engine
                .fs_read("repos/demo/src/lib.rs")
                .expect("engine read")
                .as_deref(),
            Some(&b"pub fn answer() -> u8 { 42 }\n"[..]),
            "mount write is immediately visible through fs_read, no sync-back"
        );
        assert_eq!(
            mount
                .read_file("src/lib.rs")
                .expect("mount read")
                .as_deref(),
            Some(&b"pub fn answer() -> u8 { 42 }\n"[..])
        );

        engine
            .fs_write("repos/demo/src/main.rs", b"fn main() {}\n")
            .expect("direct engine write");
        assert_eq!(
            mount
                .read_file("src/main.rs")
                .expect("read direct")
                .as_deref(),
            Some(&b"fn main() {}\n"[..]),
            "direct Engine writes are immediately visible through the mount core"
        );
        assert_eq!(
            mount.read_dir("").expect("root dir"),
            vec![MountDirEntry {
                name: "src".to_string(),
                kind: MountEntryKind::Directory,
            }]
        );
        assert_eq!(
            mount.read_dir("src").expect("src dir"),
            vec![
                MountDirEntry {
                    name: "lib.rs".to_string(),
                    kind: MountEntryKind::File,
                },
                MountDirEntry {
                    name: "main.rs".to_string(),
                    kind: MountEntryKind::File,
                },
            ]
        );
        assert_eq!(
            mount.getattr("src").expect("src attr"),
            Some(MountMetadata {
                kind: MountEntryKind::Directory,
                len: 0,
            })
        );
        assert_eq!(
            mount.getattr("src/main.rs").expect("file attr"),
            Some(MountMetadata {
                kind: MountEntryKind::File,
                len: 13,
            })
        );
    }

    #[test]
    fn doc_tree_mount_core_routes_artifacts_out_of_doctree() {
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let hook_count = Rc::new(Cell::new(0usize));
        let hook_count_for_hook = Rc::clone(&hook_count);
        engine.register_file_write_hook(
            move |_engine: &Engine, writes: &[FileWrite], _receipts: &[FileWriteReceipt]| {
                hook_count_for_hook.set(hook_count_for_hook.get() + writes.len());
                Ok(())
            },
        );
        let mount = DocTreeMount::new(&engine, "");

        let write = mount
            .write_file("target/debug/app", b"compiled artifact")
            .expect("artifact write");
        assert_eq!(
            write,
            MountWriteReceipt {
                path: "target/debug/app".to_string(),
                bytes_written: 17,
                disposition: MountWriteDisposition::Throwaway,
            }
        );
        assert_eq!(hook_count.get(), 0, "artifact writes do not fire W1 hooks");
        assert!(
            engine
                .fs_read("target/debug/app")
                .expect("engine read")
                .is_none(),
            "artifact bytes never enter DocTree"
        );
        assert_eq!(
            mount.read_file("target/debug/app").expect("mount read"),
            None
        );
        assert_eq!(
            mount.getattr("target/debug/app").expect("artifact attr"),
            None
        );
        assert!(
            engine
                .list_paths("")
                .expect("list")
                .iter()
                .all(|path| !path.starts_with("target/")),
            "no File node path is created for target artifacts"
        );
    }

    #[test]
    fn doc_tree_mount_core_unlinks_and_renames_source_files() {
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let hook_count = Rc::new(Cell::new(0usize));
        let hook_count_for_hook = Rc::clone(&hook_count);
        engine.register_file_write_hook(
            move |_engine: &Engine, writes: &[FileWrite], _receipts: &[FileWriteReceipt]| {
                hook_count_for_hook.set(hook_count_for_hook.get() + writes.len());
                Ok(())
            },
        );
        let mount = DocTreeMount::new(&engine, "repos/demo");
        mount
            .write_file("src/old.rs", b"pub fn renamed() {}\n")
            .expect("seed through mount");
        assert_eq!(hook_count.get(), 1);

        let renamed = mount
            .rename("src/old.rs", "src/new.rs")
            .expect("mount rename");
        assert_eq!(
            renamed,
            MountRenameReceipt {
                from: "src/old.rs".to_string(),
                to: "src/new.rs".to_string(),
                disposition: MountRenameDisposition::Renamed,
            }
        );
        assert_eq!(
            hook_count.get(),
            2,
            "rename writes the destination through the W0 write seam"
        );
        assert_eq!(mount.read_file("src/old.rs").expect("old read"), None);
        assert_eq!(
            mount.read_file("src/new.rs").expect("new read").as_deref(),
            Some(&b"pub fn renamed() {}\n"[..])
        );
        assert_eq!(
            engine
                .fs_read("repos/demo/src/new.rs")
                .expect("engine read")
                .as_deref(),
            Some(&b"pub fn renamed() {}\n"[..])
        );
        assert_eq!(
            mount.read_dir("src").expect("src dir"),
            vec![MountDirEntry {
                name: "new.rs".to_string(),
                kind: MountEntryKind::File,
            }]
        );

        let removed = mount.unlink("src/new.rs").expect("mount unlink");
        assert_eq!(
            removed,
            MountUnlinkReceipt {
                path: "src/new.rs".to_string(),
                disposition: MountUnlinkDisposition::Removed,
            }
        );
        assert_eq!(
            hook_count.get(),
            2,
            "unlink removes File metadata without firing write hooks"
        );
        assert_eq!(mount.getattr("src/new.rs").expect("new attr"), None);
        assert_eq!(
            mount.unlink("src/new.rs").expect("missing unlink"),
            MountUnlinkReceipt {
                path: "src/new.rs".to_string(),
                disposition: MountUnlinkDisposition::Missing,
            }
        );
    }

    #[test]
    fn doc_tree_mount_core_renames_source_directories() {
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let hook_count = Rc::new(Cell::new(0usize));
        let hook_count_for_hook = Rc::clone(&hook_count);
        engine.register_file_write_hook(
            move |_engine: &Engine, writes: &[FileWrite], _receipts: &[FileWriteReceipt]| {
                hook_count_for_hook.set(hook_count_for_hook.get() + writes.len());
                Ok(())
            },
        );
        let mount = DocTreeMount::new(&engine, "repos/demo");
        mount
            .write_file("src/lib.rs", b"pub fn lib() {}\n")
            .expect("seed lib");
        mount
            .write_file("src/nested/mod.rs", b"pub mod nested {}\n")
            .expect("seed nested");
        assert_eq!(hook_count.get(), 2);

        assert_eq!(
            mount
                .rename("src", "crates/app/src")
                .expect("directory rename"),
            MountRenameReceipt {
                from: "src".to_string(),
                to: "crates/app/src".to_string(),
                disposition: MountRenameDisposition::Renamed,
            }
        );
        assert_eq!(
            hook_count.get(),
            4,
            "directory rename writes each destination through the W0 write seam"
        );
        assert_eq!(mount.getattr("src").expect("old src attr"), None);
        assert_eq!(
            mount
                .read_file("crates/app/src/lib.rs")
                .expect("renamed lib")
                .as_deref(),
            Some(&b"pub fn lib() {}\n"[..])
        );
        assert_eq!(
            mount
                .read_file("crates/app/src/nested/mod.rs")
                .expect("renamed nested")
                .as_deref(),
            Some(&b"pub mod nested {}\n"[..])
        );
        assert_eq!(
            mount
                .read_dir("crates/app/src")
                .expect("renamed dir listing"),
            vec![
                MountDirEntry {
                    name: "lib.rs".to_string(),
                    kind: MountEntryKind::File,
                },
                MountDirEntry {
                    name: "nested".to_string(),
                    kind: MountEntryKind::Directory,
                },
            ]
        );
        assert!(mount
            .rename("crates/app", "crates/app/src/moved")
            .expect_err("self-subtree rename should fail")
            .to_string()
            .contains("own subtree"));
    }

    #[test]
    fn doc_tree_mount_core_reports_directory_remove_outcomes() {
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let mount = DocTreeMount::new(&engine, "");
        mount
            .write_file("src/lib.rs", b"pub fn lib() {}\n")
            .expect("seed lib");
        assert_eq!(
            mount.mkdir("empty").expect("mkdir empty"),
            MountMakeDirReceipt {
                path: "empty".to_string(),
                disposition: MountMakeDirDisposition::Created,
            }
        );
        assert_eq!(
            mount.mkdir("empty").expect("mkdir existing"),
            MountMakeDirReceipt {
                path: "empty".to_string(),
                disposition: MountMakeDirDisposition::AlreadyExists,
            }
        );
        assert_eq!(
            mount.getattr("empty").expect("empty attr"),
            Some(MountMetadata {
                kind: MountEntryKind::Directory,
                len: 0,
            })
        );

        assert_eq!(
            mount.rmdir("src").expect("non-empty rmdir"),
            MountRemoveDirReceipt {
                path: "src".to_string(),
                disposition: MountRemoveDirDisposition::NotEmpty,
            }
        );
        assert_eq!(
            mount.rmdir("missing").expect("missing rmdir"),
            MountRemoveDirReceipt {
                path: "missing".to_string(),
                disposition: MountRemoveDirDisposition::Missing,
            }
        );
        assert_eq!(
            mount.rmdir("src/lib.rs").expect("file rmdir"),
            MountRemoveDirReceipt {
                path: "src/lib.rs".to_string(),
                disposition: MountRemoveDirDisposition::NotDirectory,
            }
        );
        assert_eq!(
            mount.rmdir("empty").expect("remove empty"),
            MountRemoveDirReceipt {
                path: "empty".to_string(),
                disposition: MountRemoveDirDisposition::Removed,
            }
        );
        assert_eq!(mount.getattr("empty").expect("empty removed"), None);
        assert_eq!(
            mount.rmdir("target/debug").expect("artifact rmdir"),
            MountRemoveDirReceipt {
                path: "target/debug".to_string(),
                disposition: MountRemoveDirDisposition::Throwaway,
            }
        );
    }

    #[test]
    fn doc_tree_mount_core_routes_artifact_unlink_and_rename_to_throwaway() {
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let mount = DocTreeMount::new(&engine, "");

        assert_eq!(
            mount.unlink("target/debug/app").expect("artifact unlink"),
            MountUnlinkReceipt {
                path: "target/debug/app".to_string(),
                disposition: MountUnlinkDisposition::Throwaway,
            }
        );
        assert_eq!(
            mount
                .rename("target/debug/app", "target/debug/app2")
                .expect("artifact rename"),
            MountRenameReceipt {
                from: "target/debug/app".to_string(),
                to: "target/debug/app2".to_string(),
                disposition: MountRenameDisposition::Throwaway,
            }
        );
        assert!(
            engine.list_paths("").expect("list paths").is_empty(),
            "artifact unlink/rename never creates or removes DocTree File nodes"
        );
    }

    #[test]
    fn indexed_sync_back_updates_code_graph_for_changed_source() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("helper.py"),
            "def helper():\n    return 1\n",
        );
        write(
            checkout.path().join("caller.py"),
            "def caller():\n    return 1\n",
        );

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let code_index =
            CodeIndexOptions::new("repo:workspace-sync", checkout.path().display().to_string());
        let seeded = sync_back_sources_indexed(&engine, checkout.path(), "", code_index.clone())
            .expect("seed indexed workspace");
        assert_eq!(seeded.files_synced, 2);
        assert_eq!(seeded.code_symbols_indexed, 2);

        let run_dir = TempDir::new().expect("run tempdir");
        materialize_workspace(&engine, "", run_dir.path()).expect("materialize");
        write(
            run_dir.path().join("caller.py"),
            "def caller():\n    return helper()\n",
        );
        let synced = sync_back_sources_indexed(&engine, run_dir.path(), "", code_index)
            .expect("indexed sync");
        assert_eq!(synced.paths, vec!["caller.py".to_string()]);
        assert_eq!(synced.code_symbols_indexed, 1);
        assert_eq!(synced.code_edges_indexed, 1);
        assert!(synced.code_bucket_lookups < 10);

        let caller = engine
            .with_store(|store| {
                rustyred_thg_code::search_code_in_store(
                    store,
                    rustyred_thg_code::SearchCodeInput {
                        tenant_id: LOCAL_WORKSPACE_TENANT.to_string(),
                        query: "caller".to_string(),
                        repo_id: "repo:workspace-sync".to_string(),
                        path_prefix: String::new(),
                        kinds: Vec::new(),
                        limit: 5,
                    },
                )
            })
            .expect("search caller");
        assert_eq!(caller.total_returned, 1, "{:?}", caller.to_json());
        let explored = engine
            .with_store(|store| {
                rustyred_thg_code::explore_code_in_store(
                    store,
                    rustyred_thg_code::ExploreCodeInput {
                        tenant_id: LOCAL_WORKSPACE_TENANT.to_string(),
                        node_id: caller.hits[0].node_id.clone(),
                        query: String::new(),
                        repo_id: "repo:workspace-sync".to_string(),
                        max_depth: 1,
                        limit: 10,
                    },
                )
            })
            .expect("explore caller");
        assert_eq!(
            explored.focus.as_ref().map(|focus| focus.callees.clone()),
            Some(vec!["helper".to_string()])
        );
    }

    #[test]
    fn embedded_file_write_hook_indexes_code_without_sync_back() {
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        install_code_index_file_write_hook(
            &engine,
            CodeIndexOptions::new("repo:file-write-hook", "embedded://repo/file-write-hook"),
        );

        let receipts = engine
            .fs_write_batch([
                FileWrite::new("helper.py", b"def helper():\n    return 1\n".to_vec()),
                FileWrite::new(
                    "caller.py",
                    b"def caller():\n    return helper()\n".to_vec(),
                ),
            ])
            .expect("hooked file write");
        assert_eq!(
            receipts
                .iter()
                .map(|receipt| receipt.path.as_str())
                .collect::<Vec<_>>(),
            vec!["helper.py", "caller.py"]
        );

        let caller = engine
            .with_store(|store| {
                rustyred_thg_code::search_code_in_store(
                    store,
                    rustyred_thg_code::SearchCodeInput {
                        tenant_id: LOCAL_WORKSPACE_TENANT.to_string(),
                        query: "caller".to_string(),
                        repo_id: "repo:file-write-hook".to_string(),
                        path_prefix: String::new(),
                        kinds: Vec::new(),
                        limit: 5,
                    },
                )
            })
            .expect("search caller");
        assert_eq!(caller.total_returned, 1, "{:?}", caller.to_json());
        let explored = engine
            .with_store(|store| {
                rustyred_thg_code::explore_code_in_store(
                    store,
                    rustyred_thg_code::ExploreCodeInput {
                        tenant_id: LOCAL_WORKSPACE_TENANT.to_string(),
                        node_id: caller.hits[0].node_id.clone(),
                        query: String::new(),
                        repo_id: "repo:file-write-hook".to_string(),
                        max_depth: 1,
                        limit: 10,
                    },
                )
            })
            .expect("explore caller");
        assert_eq!(
            explored.focus.as_ref().map(|focus| focus.callees.clone()),
            Some(vec!["helper".to_string()]),
            "direct embedded File writes maintain the CodeCrawler graph without W3 sync-back"
        );
    }

    fn explicit_index_source(engine: &Engine, repo_id: &str, path: &str, content: &[u8]) {
        engine
            .with_store(|store| {
                index_source_file_write_in_store(
                    store,
                    SourceFileWriteIndexInput {
                        tenant_id: LOCAL_WORKSPACE_TENANT.to_string(),
                        repo_id: repo_id.to_string(),
                        repo_root_display: format!("embedded://{repo_id}"),
                        file_path: path.to_string(),
                        content: content.to_vec(),
                        actor: "collapse-measurement".to_string(),
                        generation: 0,
                        materialize_symbol_name_index: true,
                    },
                )
            })
            .expect("explicit source index");
    }

    fn helper_callers(engine: &Engine, repo_id: &str) -> Vec<String> {
        let explored = engine
            .with_store(|store| {
                rustyred_thg_code::explore_code_in_store(
                    store,
                    rustyred_thg_code::ExploreCodeInput {
                        tenant_id: LOCAL_WORKSPACE_TENANT.to_string(),
                        node_id: String::new(),
                        query: "helper".to_string(),
                        repo_id: repo_id.to_string(),
                        max_depth: 1,
                        limit: 10,
                    },
                )
            })
            .expect("explore helper");
        let mut callers = explored
            .focus
            .as_ref()
            .map(|focus| focus.callers.clone())
            .unwrap_or_default();
        callers.sort();
        callers
    }

    fn proxy_token_count(steps: &[&str]) -> u64 {
        steps
            .iter()
            .map(|step| step.split_whitespace().count() as u64)
            .sum()
    }

    #[test]
    fn on_write_path_collapses_explicit_code_maintenance_round_trips() {
        const HELPER: &[u8] = b"def helper():\n    return 1\n";
        const CALLER_INITIAL: &[u8] = b"def caller():\n    return 1\n";
        const CALLER_EDITED: &[u8] = b"def caller():\n    return helper()\n";
        let imperative_steps = [
            "fs_write_batch seed helper.py and caller.py into DocTree",
            "code_index_source_file helper.py",
            "code_index_source_file caller.py",
            "fs_write edit caller.py",
            "code_index_source_file caller.py",
            "code_explore what calls helper",
        ];
        let on_write_steps = [
            "fs_write_batch seed helper.py and caller.py into DocTree",
            "fs_write edit caller.py",
            "code_explore what calls helper",
        ];

        let imperative_data = TempDir::new().expect("imperative engine tempdir");
        let imperative =
            Engine::open(imperative_data.path(), EmbeddedConfig::default()).expect("open engine");
        let imperative_repo = "repo:collapse-imperative";
        imperative
            .fs_write_batch([
                FileWrite::new("helper.py", HELPER.to_vec()),
                FileWrite::new("caller.py", CALLER_INITIAL.to_vec()),
            ])
            .expect("imperative seed write");
        explicit_index_source(&imperative, imperative_repo, "helper.py", HELPER);
        explicit_index_source(&imperative, imperative_repo, "caller.py", CALLER_INITIAL);
        imperative
            .fs_write("caller.py", CALLER_EDITED)
            .expect("imperative edit write");
        explicit_index_source(&imperative, imperative_repo, "caller.py", CALLER_EDITED);
        let imperative_callers = helper_callers(&imperative, imperative_repo);

        let on_write_data = TempDir::new().expect("on-write engine tempdir");
        let on_write =
            Engine::open(on_write_data.path(), EmbeddedConfig::default()).expect("open engine");
        let on_write_repo = "repo:collapse-on-write";
        install_code_index_file_write_hook(
            &on_write,
            CodeIndexOptions::new(on_write_repo, format!("embedded://{on_write_repo}")),
        );
        on_write
            .fs_write_batch([
                FileWrite::new("helper.py", HELPER.to_vec()),
                FileWrite::new("caller.py", CALLER_INITIAL.to_vec()),
            ])
            .expect("on-write seed write");
        on_write
            .fs_write("caller.py", CALLER_EDITED)
            .expect("on-write edit write");
        let on_write_callers = helper_callers(&on_write, on_write_repo);

        assert_eq!(imperative_callers, vec!["caller".to_string()]);
        assert_eq!(
            on_write_callers, imperative_callers,
            "on-write maintenance returns the same semantic query result"
        );
        assert!(
            on_write_steps.len() < imperative_steps.len(),
            "on-write round trips must collapse explicit maintenance: {:?} vs {:?}",
            on_write_steps,
            imperative_steps
        );
        assert!(
            proxy_token_count(&on_write_steps) < proxy_token_count(&imperative_steps),
            "on-write proxy-token count must be lower"
        );
    }

    #[test]
    fn sandbox_bridge_rewrites_source_and_keeps_artifacts_out() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(checkout.path().join("src/main.rs"), "fn main() {}\n");
        write(checkout.path().join("target/debug/stale"), "old artifact\n");

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout(
            &engine,
            checkout.path(),
            ImportOptions {
                prefix: "repos/demo".to_string(),
                ..ImportOptions::default()
            },
        )
        .expect("import");

        let runtime = theorem_receiver::LocalProcessSandbox::new();
        let receipt = run_workspace_in_sandbox(
            &engine,
            "repos/demo",
            &runtime,
            SandboxProvisionRequest::new("demo", "w3-local"),
            SandboxRunPlan::new("/bin/sh")
                .args([
                    "-c",
                    "test -f src/main.rs && mkdir -p target/debug && printf artifact > target/debug/app && printf 'fn main() { println!(\"sandbox\"); }\\n' > src/main.rs && printf sandbox-ok",
                ])
                .timeout(Duration::from_secs(10)),
        )
        .expect("sandbox run");

        assert_eq!(receipt.files_uploaded, 1);
        assert_eq!(receipt.uploaded_paths, vec!["src/main.rs".to_string()]);
        assert_eq!(receipt.files_synced, 1);
        assert_eq!(
            receipt.synced_paths,
            vec!["repos/demo/src/main.rs".to_string()]
        );
        assert_eq!(
            receipt.proof.exit_code,
            Some(0),
            "stderr={}",
            receipt.proof.stderr
        );
        assert_eq!(receipt.proof.stdout, "sandbox-ok");
        assert_eq!(
            engine
                .fs_read("repos/demo/src/main.rs")
                .expect("read")
                .as_deref(),
            Some(&b"fn main() { println!(\"sandbox\"); }\n"[..])
        );
        assert!(
            engine
                .list_paths("repos/demo")
                .expect("list")
                .iter()
                .all(|path| !path.contains("/target/") && !path.starts_with("target/")),
            "sandbox artifacts stay out of the DocTree"
        );
    }

    #[test]
    fn sandbox_streaming_bridge_cancels_and_syncs_changed_source() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(checkout.path().join("src/main.rs"), "fn main() {}\n");

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout(
            &engine,
            checkout.path(),
            ImportOptions {
                prefix: "repos/demo-stream".to_string(),
                ..ImportOptions::default()
            },
        )
        .expect("import");

        let runtime = theorem_receiver::LocalProcessSandbox::new();
        let cancel = SandboxCancelToken::new();
        let cancel_from_event = cancel.clone();
        let mut events = Vec::new();
        let receipt = run_workspace_in_sandbox_streaming(
            &engine,
            "repos/demo-stream",
            &runtime,
            SandboxProvisionRequest::new("demo", "w3-local-stream"),
            SandboxRunPlan::new("/bin/sh")
                .args([
                    "-c",
                    "printf ready && printf 'fn main() { println!(\"cancelled\"); }\\n' > src/main.rs && sleep 5 && printf late",
                ])
                .timeout(Duration::from_secs(30)),
            &cancel,
            &mut |event| {
                if matches!(event, SandboxStreamEvent::Stdout(bytes) if bytes == b"ready") {
                    cancel_from_event.cancel();
                }
                events.push(event.clone());
            },
        )
        .expect("streaming sandbox run");

        assert_eq!(receipt.files_uploaded, 1);
        assert_eq!(receipt.files_synced, 1);
        assert_eq!(receipt.proof.status, "cancelled");
        assert_eq!(receipt.proof.exit_code, None);
        assert_eq!(receipt.proof.stdout, "ready");
        assert!(
            events.iter().any(
                |event| matches!(event, SandboxStreamEvent::Stdout(bytes) if bytes == b"ready")
            ),
            "stream callback saw stdout before cancellation: {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                SandboxStreamEvent::Exit {
                    cancelled: true,
                    timed_out: false,
                    ..
                }
            )),
            "stream callback saw cancellation exit event: {events:?}"
        );
        assert_eq!(
            engine
                .fs_read("repos/demo-stream/src/main.rs")
                .expect("read")
                .as_deref(),
            Some(&b"fn main() { println!(\"cancelled\"); }\n"[..])
        );
    }

    #[test]
    fn mirror_import_indexes_real_files_and_oracle_is_clean() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(checkout.path().join("README.md"), "# fixture\n");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        );
        write(checkout.path().join("target/debug/app"), "artifact\n");
        write(checkout.path().join(".git/config"), "[core]\n");

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let receipt = import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");

        assert_eq!(receipt.index.files_indexed, 2);
        assert!(checkout.path().join("src/lib.rs").is_file());
        assert_eq!(
            engine
                .fs_read("repos/demo/src/lib.rs")
                .expect("fs_read")
                .as_deref(),
            Some(&b"pub fn answer() -> u8 { 42 }\n"[..]),
            "engine fs_read resolves the real mirrored file"
        );
        assert_eq!(
            engine.list_paths("repos/demo").expect("list paths"),
            vec![
                "repos/demo/README.md".to_string(),
                "repos/demo/src/lib.rs".to_string(),
            ]
        );
        let node = engine
            .query("query{ graphNode(id:\"file:repos/demo/src/lib.rs\") }")
            .expect("graph node");
        assert!(!node["graphNode"].is_null(), "File node is queryable");
        assert!(
            audit_consistency(&engine, checkout.path(), "repos/demo")
                .expect("audit")
                .is_empty(),
            "fresh mirror import is consistent"
        );
    }

    #[test]
    fn oracle_detects_manual_change_and_reconcile_repairs_it() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");

        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 2 }\n",
        );
        let divergence =
            audit_consistency(&engine, checkout.path(), "repos/demo").expect("audit dirty");
        assert_eq!(
            divergence.hash_mismatches,
            vec!["repos/demo/src/lib.rs".to_string()],
            "oracle reads fresh filesystem bytes and spots the stale hash"
        );

        reconcile(&engine, checkout.path(), "repos/demo").expect("reconcile");
        assert!(audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("audit clean")
            .is_empty());
        assert_eq!(
            engine
                .fs_read("repos/demo/src/lib.rs")
                .expect("read repaired")
                .as_deref(),
            Some(&b"pub fn answer() -> u8 { 2 }\n"[..])
        );
    }

    #[test]
    fn index_path_accepts_paths_relative_to_the_mirror_root() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");

        index_path(&engine, checkout.path(), "repos/demo", "src/lib.rs")
            .expect("index relative path")
            .expect("indexed file");

        assert!(audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("audit")
            .is_empty());
        assert_eq!(
            engine
                .fs_read("repos/demo/src/lib.rs")
                .expect("read")
                .as_deref(),
            Some(&b"pub fn answer() -> u8 { 1 }\n"[..])
        );
    }

    #[test]
    fn agent_session_plan_points_at_real_mirror_root() {
        let checkout = TempDir::new().expect("checkout tempdir");
        let plan = agent_session_plan(checkout.path(), "codex", "edit directly")
            .expect("agent session plan");
        assert_eq!(plan.program, "codex");
        assert_eq!(plan.cwd, fs::canonicalize(checkout.path()).unwrap());
        assert_eq!(
            plan.args,
            vec!["exec".to_string(), "edit directly".to_string()]
        );
        assert!(
            plan.strip_env.iter().any(|key| key == "ANTHROPIC_API_KEY"),
            "agent session inherits receiver secret stripping"
        );
    }

    #[test]
    fn reconcile_removes_stray_file_node_and_preserves_first_class_content() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");
        engine
            .mutate(
                "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
                serde_json::json!({
                    "n": [{
                        "id": "mem:keep",
                        "labels": ["Memory"],
                        "properties": { "body": "first-class content" }
                    }]
                }),
            )
            .expect("insert memory node");
        let before = engine
            .query("query{ graphNode(id:\"mem:keep\") }")
            .expect("memory before");

        engine
            .fs_write("repos/demo/ghost.rs", b"pub fn ghost() {}\n")
            .expect("stray old-style File node");
        let divergence =
            audit_consistency(&engine, checkout.path(), "repos/demo").expect("audit stray");
        assert_eq!(
            divergence.indexed_but_missing_on_disk,
            vec!["repos/demo/ghost.rs".to_string()]
        );
        assert_eq!(
            divergence.doctree_without_real_file,
            vec!["repos/demo/ghost.rs".to_string()]
        );

        let repaired = reconcile(&engine, checkout.path(), "repos/demo").expect("reconcile");
        assert_eq!(repaired.files_removed, 1);
        assert!(audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("audit clean")
            .is_empty());
        let after = engine
            .query("query{ graphNode(id:\"mem:keep\") }")
            .expect("memory after");
        assert_eq!(before, after, "code-index rebuild leaves Memory untouched");
    }

    #[test]
    fn watcher_reconciles_create_modify_delete_and_rename() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");
        let mut watcher = WorkspaceMirrorWatcher::start(
            checkout.path(),
            "repos/demo",
            Duration::from_millis(100),
        )
        .expect("start watcher");

        write(checkout.path().join("src/new.rs"), "pub fn created() {}\n");
        assert!(watcher
            .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
            .expect("created convergence")
            .is_empty());
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 3 }\n",
        );
        assert!(watcher
            .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
            .expect("modify convergence")
            .is_empty());
        fs::rename(
            checkout.path().join("src/new.rs"),
            checkout.path().join("src/renamed.rs"),
        )
        .expect("rename");
        assert!(watcher
            .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
            .expect("rename convergence")
            .is_empty());
        fs::remove_file(checkout.path().join("src/renamed.rs")).expect("delete");
        assert!(watcher
            .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
            .expect("delete convergence")
            .is_empty());
        watcher.stop();

        let paths = engine.list_paths("repos/demo").expect("list after watch");
        assert_eq!(paths, vec!["repos/demo/src/lib.rs".to_string()]);
        assert_eq!(
            engine
                .fs_read("repos/demo/src/lib.rs")
                .expect("read modified")
                .as_deref(),
            Some(&b"pub fn answer() -> u8 { 3 }\n"[..])
        );
    }

    #[test]
    fn watcher_start_reconciled_repairs_changes_missed_while_stopped() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");

        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 99 }\n",
        );
        let (watcher, receipt) = WorkspaceMirrorWatcher::start_reconciled(
            &engine,
            checkout.path(),
            "repos/demo",
            Duration::from_millis(100),
        )
        .expect("start reconciled");
        assert_eq!(receipt.files_indexed, 1);
        assert!(audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("audit")
            .is_empty());
        watcher.stop();
    }

    #[test]
    fn mirror_handles_binary_file_without_crashing() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(checkout.path().join("src/blob.bin"), b"\0binary\0bytes");
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");
        assert!(audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("audit")
            .is_empty());
        assert_eq!(
            engine
                .fs_read("repos/demo/src/blob.bin")
                .expect("binary read")
                .as_deref(),
            Some(&b"\0binary\0bytes"[..])
        );
    }

    #[cfg(unix)]
    #[test]
    fn mirror_symlink_cannot_escape_workspace_root() {
        let checkout = TempDir::new().expect("checkout tempdir");
        let outside = TempDir::new().expect("outside tempdir");
        write(outside.path().join("secret.rs"), "pub fn secret() {}\n");
        fs::create_dir_all(checkout.path().join("src")).expect("mkdir src");
        std::os::unix::fs::symlink(
            outside.path().join("secret.rs"),
            checkout.path().join("src/secret.rs"),
        )
        .expect("symlink");
        write(checkout.path().join("src/lib.rs"), "pub fn visible() {}\n");

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");
        assert!(audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("audit")
            .is_empty());
        assert_eq!(
            engine.list_paths("repos/demo").expect("list"),
            vec!["repos/demo/src/lib.rs".to_string()],
            "escaping symlink is not indexed"
        );
    }

    #[test]
    fn mirror_options_skip_oversized_file_and_remove_stale_projection() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(checkout.path().join("src/blob.txt"), "tiny\n");
        let options = MirrorImportOptions {
            prefix: "repos/demo".to_string(),
            max_file_bytes: 8,
            ..MirrorImportOptions::default()
        };

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(&engine, checkout.path(), options.clone()).expect("mirror import");
        assert_eq!(
            engine.list_paths("repos/demo").expect("list"),
            vec!["repos/demo/src/blob.txt".to_string()]
        );

        write(
            checkout.path().join("src/blob.txt"),
            "this is now larger than the mirror limit\n",
        );
        let dirty =
            audit_consistency_with_options(&engine, checkout.path(), &options).expect("audit");
        assert_eq!(
            dirty.indexed_but_missing_on_disk,
            vec!["repos/demo/src/blob.txt".to_string()]
        );

        let repaired = reconcile_with_options(&engine, checkout.path(), &options)
            .expect("reconcile with limit");
        assert_eq!(repaired.files_removed, 1);
        assert!(
            audit_consistency_with_options(&engine, checkout.path(), &options)
                .expect("clean audit")
                .is_empty()
        );
        assert!(
            engine
                .list_paths("repos/demo")
                .expect("list after oversized")
                .is_empty(),
            "oversized files are outside the captured mirror region"
        );
    }

    #[test]
    fn reconcile_with_options_enforces_total_byte_limit() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(checkout.path().join("src/a.rs"), "aaaa\n");
        write(checkout.path().join("src/b.rs"), "bbbb\n");
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let error = reconcile_with_options(
            &engine,
            checkout.path(),
            &MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                max_total_bytes: 5,
                ..MirrorImportOptions::default()
            },
        )
        .expect_err("reconcile should enforce max_total_bytes");
        assert!(
            error.to_string().contains("max_total_bytes"),
            "expected max_total_bytes error, got {error}"
        );
    }

    #[test]
    fn mirror_copy_cleans_stale_workspace_files_before_reconcile() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(checkout.path().join("src/new.rs"), "pub fn new() {}\n");
        let workspace = TempDir::new().expect("workspace tempdir");
        write(workspace.path().join("src/stale.rs"), "pub fn stale() {}\n");
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                workspace_root: Some(workspace.path().to_path_buf()),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");

        assert!(workspace.path().join("src/new.rs").is_file());
        assert!(!workspace.path().join("src/stale.rs").exists());
        let paths = engine
            .list_paths("repos/demo")
            .expect("list mirrored paths");
        assert_eq!(paths, vec!["repos/demo/src/new.rs".to_string()]);
    }

    #[test]
    fn drop_and_rebuild_code_index_preserves_non_file_graph_content() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        write(checkout.path().join("src/other.rs"), "pub fn other() {}\n");

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");
        engine
            .mutate(
                "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
                serde_json::json!({
                    "n": [{
                        "id": "mem:keep",
                        "labels": ["Memory"],
                        "properties": { "body": "first-class content" }
                    }]
                }),
            )
            .expect("insert memory node");
        let before = engine
            .query("query{ graphNode(id:\"mem:keep\") }")
            .expect("memory before");

        let dropped = drop_code_index(&engine, "repos/demo").expect("drop code index");
        assert_eq!(dropped.files_removed, 2);
        assert!(engine
            .list_paths("repos/demo")
            .expect("list after drop")
            .is_empty());
        let missing = audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("audit dropped projection");
        assert_eq!(
            missing.missing_from_index,
            vec![
                "repos/demo/src/lib.rs".to_string(),
                "repos/demo/src/other.rs".to_string(),
            ]
        );

        let rebuilt =
            rebuild_code_index(&engine, checkout.path(), "repos/demo").expect("rebuild index");
        assert_eq!(rebuilt.files_indexed, 2);
        assert!(audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("clean audit")
            .is_empty());
        let after = engine
            .query("query{ graphNode(id:\"mem:keep\") }")
            .expect("memory after");
        assert_eq!(before, after, "non-File graph content survives rebuild");
    }

    #[test]
    fn audit_prometheus_reports_divergence_classes() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");

        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 2 }\n",
        );
        let receipt = audit(&engine, checkout.path(), "repos/demo").expect("audit receipt");
        let metrics = render_audit_prometheus(&receipt, "repos/demo");
        assert!(metrics.contains("rustyred_workspace_mirror_divergence_count"));
        assert!(metrics.contains("prefix=\"repos/demo\""));
        assert!(metrics.contains("class=\"hash_mismatches\"} 1"));
    }

    #[test]
    fn periodic_audit_monitor_reports_drift_and_recovery_metrics() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine_lock = Arc::new(Mutex::new(()));
        let options = MirrorImportOptions {
            prefix: "repos/demo".to_string(),
            ..MirrorImportOptions::default()
        };
        {
            let _guard = engine_lock.lock().expect("engine lock");
            let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
            import_checkout_mirror(&engine, checkout.path(), options.clone())
                .expect("mirror import");
        }
        let monitor = WorkspaceMirrorAuditMonitor::start_with_engine_lock(
            data.path(),
            vec![MirrorAuditTarget::new(checkout.path(), options.clone())],
            Duration::from_millis(20),
            Arc::clone(&engine_lock),
        )
        .expect("start audit monitor");

        let clean_metrics = wait_for_prometheus_contains(
            &monitor,
            "rustyred_workspace_mirror_audit_divergence_count 0",
        );
        assert!(clean_metrics.contains("rustyred_workspace_mirror_audit_targets 1"));

        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 2 }\n",
        );
        let dirty_metrics = wait_for_prometheus_contains(
            &monitor,
            "rustyred_workspace_mirror_audit_divergence_count 1",
        );
        assert!(dirty_metrics.contains("class=\"hash_mismatches\"} 1"));

        {
            let _guard = engine_lock.lock().expect("engine lock");
            let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
            reconcile_with_options(&engine, checkout.path(), &options)
                .expect("reconcile dirty file");
        }
        let recovered_metrics = wait_for_prometheus_contains(
            &monitor,
            "rustyred_workspace_mirror_audit_divergence_count 0",
        );
        assert!(recovered_metrics.contains("rustyred_workspace_mirror_audit_errors 0"));
        monitor.stop();
    }

    #[test]
    fn watcher_force_rescan_repairs_event_overflow_style_gap() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let (mut watcher, _) = WorkspaceMirrorWatcher::start_reconciled(
            &engine,
            checkout.path(),
            "repos/demo",
            Duration::from_millis(100),
        )
        .expect("start reconciled");

        write(
            checkout.path().join("src/missed.rs"),
            "pub fn missed() {}\n",
        );
        let forced = watcher.force_rescan(&engine).expect("force rescan");
        assert_eq!(forced.files_indexed, 1);
        assert!(watcher
            .audit(&engine)
            .expect("watcher audit")
            .divergence
            .is_empty());
        watcher.stop();
    }

    #[test]
    fn watcher_debouncer_error_repairs_event_overflow_gap() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let (mut watcher, _) = WorkspaceMirrorWatcher::start_reconciled(
            &engine,
            checkout.path(),
            "repos/demo",
            Duration::from_millis(100),
        )
        .expect("start reconciled");

        write(
            checkout.path().join("src/missed_after_overflow.rs"),
            "pub fn missed_after_overflow() {}\n",
        );
        watcher.inject_event_result_for_test(Err(vec![
            notify_debouncer_full::notify::Error::generic("simulated event overflow"),
        ]));

        let repaired = watcher.drain(&engine).expect("overflow drain repairs");
        assert_eq!(repaired.files_indexed, 1);
        assert!(watcher
            .audit(&engine)
            .expect("watcher audit")
            .divergence
            .is_empty());
        watcher.stop();
    }

    #[test]
    fn watcher_converges_atomic_save_bursts_and_subtree_moves() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let (mut watcher, _) = WorkspaceMirrorWatcher::start_reconciled(
            &engine,
            checkout.path(),
            "repos/demo",
            Duration::from_millis(50),
        )
        .expect("start reconciled");

        write(
            checkout.path().join("src/lib.rs.tmp"),
            "pub fn answer() -> u8 { 7 }\n",
        );
        fs::rename(
            checkout.path().join("src/lib.rs.tmp"),
            checkout.path().join("src/lib.rs"),
        )
        .expect("atomic rename");
        assert!(watcher
            .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
            .expect("atomic save convergence")
            .is_empty());
        assert_eq!(
            engine
                .fs_read("repos/demo/src/lib.rs")
                .expect("read atomic")
                .as_deref(),
            Some(&b"pub fn answer() -> u8 { 7 }\n"[..])
        );
        assert!(!engine
            .list_paths("repos/demo")
            .expect("list atomic")
            .contains(&"repos/demo/src/lib.rs.tmp".to_string()));

        for i in 0..25 {
            write(
                checkout.path().join(format!("src/burst_{i}.rs")),
                format!("pub fn burst_{i}() -> u8 {{ {i} }}\n"),
            );
        }
        assert!(watcher
            .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
            .expect("burst convergence")
            .is_empty());

        write(checkout.path().join("src/tree/a.rs"), "pub fn a() {}\n");
        write(checkout.path().join("src/tree/b.rs"), "pub fn b() {}\n");
        assert!(watcher
            .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
            .expect("tree convergence")
            .is_empty());
        fs::rename(
            checkout.path().join("src/tree"),
            checkout.path().join("src/moved"),
        )
        .expect("subtree rename");
        assert!(watcher
            .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
            .expect("subtree move convergence")
            .is_empty());
        assert!(engine
            .list_paths("repos/demo")
            .expect("list moved")
            .contains(&"repos/demo/src/moved/a.rs".to_string()));
        assert!(!engine
            .list_paths("repos/demo")
            .expect("list old tree")
            .contains(&"repos/demo/src/tree/a.rs".to_string()));

        fs::remove_dir_all(checkout.path().join("src/moved")).expect("subtree delete");
        assert!(watcher
            .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
            .expect("subtree delete convergence")
            .is_empty());
        assert!(!engine
            .list_paths("repos/demo")
            .expect("list delete")
            .contains(&"repos/demo/src/moved/a.rs".to_string()));
        watcher.stop();
    }

    #[test]
    fn deterministic_mirror_fuzz_harness_converges() {
        let checkout = TempDir::new().expect("checkout tempdir");
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let (mut watcher, _) = WorkspaceMirrorWatcher::start_reconciled(
            &engine,
            checkout.path(),
            "repos/demo",
            Duration::from_millis(25),
        )
        .expect("start reconciled");

        for step in 0..30 {
            match step % 6 {
                0 => write(
                    checkout.path().join(format!("src/fuzz_{step}.rs")),
                    format!("pub fn fuzz_{step}() -> usize {{ {step} }}\n"),
                ),
                1 => write(
                    checkout.path().join("src/shared.rs"),
                    format!("pub fn shared() -> usize {{ {step} }}\n"),
                ),
                2 => {
                    let from = checkout.path().join("src/shared.rs");
                    if from.exists() {
                        fs::rename(from, checkout.path().join("src/shared_renamed.rs"))
                            .expect("rename shared");
                    }
                }
                3 => {
                    let path = checkout.path().join("src/shared_renamed.rs");
                    if path.exists() {
                        fs::remove_file(path).expect("remove renamed shared");
                    }
                }
                4 => write(
                    checkout.path().join("target/ignored.rs"),
                    "pub fn ignored() {}\n",
                ),
                _ => {
                    write(
                        checkout.path().join("src/atomic.tmp"),
                        format!("pub fn atomic() -> usize {{ {step} }}\n"),
                    );
                    fs::rename(
                        checkout.path().join("src/atomic.tmp"),
                        checkout.path().join("src/atomic.rs"),
                    )
                    .expect("atomic fuzz rename");
                }
            }
            assert!(watcher
                .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
                .expect("fuzz convergence")
                .is_empty());
        }

        let paths = engine.list_paths("repos/demo").expect("list fuzz");
        assert!(!paths.iter().any(|path| path.contains("/target/")));
        watcher.stop();
    }

    #[test]
    fn randomized_mirror_property_harness_covers_full_operation_space() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/delete_me.rs"),
            "pub fn delete_me() {}\n",
        );
        write(
            checkout.path().join("src/rename_pool.rs"),
            "pub fn rename_pool() {}\n",
        );
        write(
            checkout.path().join("src/tree_pool/a.rs"),
            "pub fn tree_a() {}\n",
        );
        write(
            checkout.path().join("src/tree_pool/b.rs"),
            "pub fn tree_b() {}\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let options = MirrorImportOptions {
            prefix: "repos/demo".to_string(),
            max_file_bytes: 128,
            ..MirrorImportOptions::default()
        };
        let (mut watcher, _) = WorkspaceMirrorWatcher::start_reconciled_with_options(
            &engine,
            checkout.path(),
            &options,
            Duration::from_millis(25),
        )
        .expect("start reconciled");

        let mut rng = FuzzRng::new(0x7072_6f70_6675_7a7a);
        let mut covered = [false; MIRROR_FUZZ_OPERATION_KINDS];
        for step in 0..MIRROR_FUZZ_MIN_OPERATIONS {
            let op = if step < MIRROR_FUZZ_OPERATION_KINDS {
                step
            } else {
                rng.next_usize(MIRROR_FUZZ_OPERATION_KINDS)
            };
            covered[op] = true;
            let op_name = apply_generated_mirror_operation(checkout.path(), step, op, &mut rng);
            let divergence = watcher
                .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
                .unwrap_or_else(|error| panic!("fuzz op {op_name} at step {step}: {error}"));
            assert!(
                divergence.is_empty(),
                "fuzz op {op_name} at step {step} left divergence: {divergence:?}"
            );
        }

        assert!(
            covered.iter().all(|seen| *seen),
            "fuzz harness must cover every configured operation kind"
        );
        watcher.stop();
    }

    #[test]
    fn fuzz_harness_leaves_first_class_graph_content_untouched() {
        let checkout = TempDir::new().expect("checkout tempdir");
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        engine
            .mutate(
                "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
                serde_json::json!({
                    "n": [
                        {
                            "id": "mem:fuzz-keep",
                            "labels": ["Memory", "Coordination"],
                            "properties": { "body": "first-class fuzz sentinel" }
                        },
                        {
                            "id": "item:fuzz-keep",
                            "labels": ["Item", "Epistemic"],
                            "properties": { "body": "item sentinel", "cluster": "alpha" }
                        }
                    ]
                }),
            )
            .expect("insert first-class nodes");
        let before = engine
            .query(
                "query{ a: graphNode(id:\"mem:fuzz-keep\") b: graphNode(id:\"item:fuzz-keep\") }",
            )
            .expect("first-class before");
        let (mut watcher, _) = WorkspaceMirrorWatcher::start_reconciled(
            &engine,
            checkout.path(),
            "repos/demo",
            Duration::from_millis(25),
        )
        .expect("start reconciled");

        let mut rng = FuzzRng::new(0x5eed_f00d);
        for step in 0..40 {
            match rng.next_usize(8) {
                0 => write(
                    checkout.path().join(format!("src/rand_{step}.rs")),
                    format!("pub fn rand_{step}() -> usize {{ {step} }}\n"),
                ),
                1 => write(
                    checkout.path().join("src/random_shared.rs"),
                    format!("pub fn random_shared() -> usize {{ {step} }}\n"),
                ),
                2 => {
                    let from = checkout.path().join("src/random_shared.rs");
                    if from.exists() {
                        fs::rename(from, checkout.path().join("src/random_moved.rs"))
                            .expect("rename random shared");
                    }
                }
                3 => {
                    let path = checkout.path().join("src/random_moved.rs");
                    if path.exists() {
                        fs::remove_file(path).expect("remove random moved");
                    }
                }
                4 => {
                    write(
                        checkout.path().join("src/random_tree/a.rs"),
                        "pub fn a() {}\n",
                    );
                    write(
                        checkout.path().join("src/random_tree/b.rs"),
                        "pub fn b() {}\n",
                    );
                    let moved = checkout.path().join("src/random_tree_moved");
                    if moved.exists() {
                        fs::remove_dir_all(&moved).expect("clear previous moved tree");
                    }
                    fs::rename(checkout.path().join("src/random_tree"), moved)
                        .expect("move random tree");
                }
                5 => write(
                    checkout.path().join("node_modules/pkg/ignored.js"),
                    "module.exports = 1;\n",
                ),
                6 => write(
                    checkout.path().join(format!("src/blob_{step}.bin")),
                    [0u8, 159, 146, step as u8, 0],
                ),
                _ => {
                    write(
                        checkout.path().join("src/random_atomic.tmp"),
                        format!("pub fn random_atomic() -> usize {{ {step} }}\n"),
                    );
                    fs::rename(
                        checkout.path().join("src/random_atomic.tmp"),
                        checkout.path().join("src/random_atomic.rs"),
                    )
                    .expect("random atomic rename");
                }
            }
            assert!(watcher
                .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
                .expect("random fuzz convergence")
                .is_empty());
            let current = engine
                .query("query{ a: graphNode(id:\"mem:fuzz-keep\") b: graphNode(id:\"item:fuzz-keep\") }")
                .expect("first-class during fuzz");
            assert_eq!(before, current, "fuzz changed first-class graph content");
        }
        watcher.stop();
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_file_is_skipped_and_rest_of_tree_stays_consistent() {
        use std::os::unix::fs::PermissionsExt;

        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/readable.rs"),
            "pub fn readable() {}\n",
        );
        let unreadable = checkout.path().join("src/unreadable.rs");
        write(unreadable.clone(), "pub fn hidden() {}\n");
        let original_permissions = fs::metadata(&unreadable)
            .expect("metadata before chmod")
            .permissions();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000))
            .expect("chmod unreadable");
        if fs::read(&unreadable).is_ok() {
            fs::set_permissions(&unreadable, original_permissions).expect("restore permissions");
            return;
        }

        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let receipt = import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");
        assert!(receipt.files_skipped >= 1);
        assert!(audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("audit")
            .is_empty());
        assert_eq!(
            engine.list_paths("repos/demo").expect("list"),
            vec!["repos/demo/src/readable.rs".to_string()]
        );
        fs::set_permissions(&unreadable, original_permissions).expect("restore permissions");
    }

    #[test]
    fn startup_reconcile_repairs_crash_gaps_on_both_sides() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");

        write(
            checkout.path().join("src/after_crash.rs"),
            "pub fn after() {}\n",
        );
        let (watcher, receipt) = WorkspaceMirrorWatcher::start_reconciled(
            &engine,
            checkout.path(),
            "repos/demo",
            Duration::from_millis(25),
        )
        .expect("start reconciled after missed file write");
        assert_eq!(receipt.files_indexed, 1);
        watcher.stop();
        assert!(audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("clean after file-side crash")
            .is_empty());

        assert!(engine
            .fs_remove_file_index("repos/demo/src/lib.rs")
            .expect("remove index while file remains"));
        let divergence =
            audit_consistency(&engine, checkout.path(), "repos/demo").expect("audit missing index");
        assert_eq!(
            divergence.missing_from_index,
            vec!["repos/demo/src/lib.rs".to_string()]
        );
        let (watcher, receipt) = WorkspaceMirrorWatcher::start_reconciled(
            &engine,
            checkout.path(),
            "repos/demo",
            Duration::from_millis(25),
        )
        .expect("start reconciled after index-side crash");
        assert_eq!(receipt.files_indexed, 1);
        watcher.stop();
        assert!(audit_consistency(&engine, checkout.path(), "repos/demo")
            .expect("clean after index-side crash")
            .is_empty());
    }

    #[test]
    fn concurrent_edit_during_index_converges_to_final_bytes() {
        use std::io::{Seek as _, Write as _};

        let checkout = TempDir::new().expect("checkout tempdir");
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        let (mut watcher, _) = WorkspaceMirrorWatcher::start_reconciled(
            &engine,
            checkout.path(),
            "repos/demo",
            Duration::from_millis(25),
        )
        .expect("start reconciled");
        let path = checkout.path().join("src/concurrent.rs");
        let final_body = "pub fn concurrent() -> usize { 2 }\n".repeat(128);
        let writer_path = path.clone();
        let writer_body = final_body.clone();
        let handle = std::thread::spawn(move || {
            if let Some(parent) = writer_path.parent() {
                fs::create_dir_all(parent).expect("create concurrent parent");
            }
            let mut file = fs::File::create(&writer_path).expect("create concurrent file");
            file.write_all(b"pub fn concurrent() -> usize { 1 }\n")
                .expect("write first half");
            file.flush().expect("flush first half");
            std::thread::sleep(Duration::from_millis(75));
            file.set_len(0).expect("truncate concurrent file");
            file.rewind().expect("rewind concurrent file");
            file.write_all(writer_body.as_bytes())
                .expect("write final body");
            file.flush().expect("flush final body");
        });
        handle.join().expect("writer joins");

        assert!(watcher
            .wait_until_consistent(&engine, DEFAULT_CONVERGENCE_TIMEOUT)
            .expect("concurrent edit convergence")
            .is_empty());
        assert_eq!(
            engine
                .fs_read("repos/demo/src/concurrent.rs")
                .expect("read concurrent")
                .as_deref(),
            Some(final_body.as_bytes())
        );
        watcher.stop();
    }

    #[test]
    fn oracle_catches_injected_rename_modify_defect() {
        let checkout = TempDir::new().expect("checkout tempdir");
        write(
            checkout.path().join("src/old.rs"),
            "pub fn value() -> u8 { 1 }\n",
        );
        let data = TempDir::new().expect("engine tempdir");
        let engine = Engine::open(data.path(), EmbeddedConfig::default()).expect("open engine");
        import_checkout_mirror(
            &engine,
            checkout.path(),
            MirrorImportOptions {
                prefix: "repos/demo".to_string(),
                ..MirrorImportOptions::default()
            },
        )
        .expect("mirror import");

        fs::rename(
            checkout.path().join("src/old.rs"),
            checkout.path().join("src/new.rs"),
        )
        .expect("rename");
        write(
            checkout.path().join("src/new.rs"),
            "pub fn value() -> u8 { 9 }\n",
        );

        let divergence =
            audit_consistency(&engine, checkout.path(), "repos/demo").expect("audit injected bug");
        assert_eq!(
            divergence.indexed_but_missing_on_disk,
            vec!["repos/demo/src/old.rs".to_string()]
        );
        assert_eq!(
            divergence.missing_from_index,
            vec!["repos/demo/src/new.rs".to_string()]
        );
    }

    fn apply_generated_mirror_operation(
        root: &Path,
        step: usize,
        op: usize,
        rng: &mut FuzzRng,
    ) -> &'static str {
        match op {
            0 => {
                write(
                    root.join(format!("src/generated_{step}.rs")),
                    format!("pub fn generated_{step}() -> usize {{ {step} }}\n"),
                );
                "create"
            }
            1 => {
                write(
                    root.join("src/shared_generated.rs"),
                    format!("pub fn shared_generated() -> usize {{ {step} }}\n"),
                );
                "modify"
            }
            2 => {
                let path = root.join("src/delete_me.rs");
                if !path.exists() {
                    write(path.clone(), "pub fn delete_me() {}\n");
                }
                fs::remove_file(path).expect("delete generated file");
                "delete"
            }
            3 => {
                let from = root.join("src/rename_pool.rs");
                if !from.exists() {
                    write(from.clone(), "pub fn rename_pool() {}\n");
                }
                let to = root.join(format!("src/renamed_{step}.rs"));
                if to.exists() {
                    fs::remove_file(&to).expect("remove old rename target");
                }
                fs::rename(from, to).expect("rename generated file");
                "rename"
            }
            4 => {
                let from = root.join("src/tree_pool");
                if !from.exists() {
                    write(from.join("a.rs"), "pub fn tree_a() {}\n");
                    write(from.join("b.rs"), "pub fn tree_b() {}\n");
                }
                let to = root.join(format!("src/tree_moved_{step}"));
                if to.exists() {
                    fs::remove_dir_all(&to).expect("remove old moved tree");
                }
                fs::rename(from, to).expect("move generated subtree");
                "move_subtree"
            }
            5 => {
                let tmp = root.join("src/atomic_generated.rs.tmp");
                write(
                    tmp.clone(),
                    format!("pub fn atomic_generated() -> usize {{ {step} }}\n"),
                );
                fs::rename(tmp, root.join("src/atomic_generated.rs"))
                    .expect("atomic generated save");
                "atomic_save"
            }
            6 => {
                for item in 0..8 {
                    write(
                        root.join(format!("src/burst_{step}_{item}.rs")),
                        format!("pub fn burst_{step}_{item}() -> usize {{ {item} }}\n"),
                    );
                }
                "burst"
            }
            7 => {
                write(
                    root.join(format!("src/large_{step}.bin")),
                    vec![b'x'; 129 + rng.next_usize(32)],
                );
                "large_file"
            }
            8 => {
                write(
                    root.join(format!("src/binary_{step}.bin")),
                    [0, 159, 146, (step % 255) as u8, 0],
                );
                "binary_file"
            }
            9 => {
                let path = root.join(format!("src/symlink_{step}.rs"));
                if path.exists() {
                    fs::remove_file(&path).expect("remove old symlink");
                }
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink("/etc/passwd", &path)
                        .expect("create generated symlink");
                }
                #[cfg(not(unix))]
                {
                    write(path, "pub fn symlink_placeholder() {}\n");
                }
                "symlink"
            }
            10 => {
                write(
                    root.join(format!("node_modules/pkg/ignored_{step}.js")),
                    "module.exports = 1;\n",
                );
                write(
                    root.join(format!("target/debug/ignored_{step}.rs")),
                    "pub fn ignored() {}\n",
                );
                "excluded_region"
            }
            _ => unreachable!("operation index is bounded by MIRROR_FUZZ_OPERATION_KINDS"),
        }
    }

    fn write(path: PathBuf, body: impl AsRef<[u8]>) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, body).expect("write fixture");
    }

    fn wait_for_prometheus_contains(monitor: &WorkspaceMirrorAuditMonitor, needle: &str) -> String {
        let started = Instant::now();
        let mut latest = String::new();
        while started.elapsed() < DEFAULT_CONVERGENCE_TIMEOUT {
            latest = monitor.latest_prometheus();
            if latest.contains(needle) {
                return latest;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for metric `{needle}`; latest metrics:\n{latest}");
    }

    struct FuzzRng {
        state: u64,
    }

    impl FuzzRng {
        fn new(seed: u64) -> Self {
            Self { state: seed }
        }

        fn next_usize(&mut self, modulo: usize) -> usize {
            self.state = self
                .state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            ((self.state >> 32) as usize) % modulo
        }
    }
}
