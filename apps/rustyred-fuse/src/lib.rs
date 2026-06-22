//! rustyred-fuse: W6, the FUSE endgame.
//!
//! Mount the DocTree as a real POSIX filesystem so the toolchain reads RustyRed's
//! files directly, with no materialize/sync copy. This first slice is read-only.
//!
//! The DocTree is a flat set of (path -> bytes) pairs; a POSIX filesystem is a
//! tree of inodes with directories. The translation (which paths are files vs
//! implicit directories, a directory's immediate children, inode assignment) is
//! the bug-prone part, so it lives in the pure [`DirView`] + [`Inodes`] types and
//! is unit-tested without a mount. The `fuser` glue ([`DocTreeFs`]) is mechanical
//! wiring of those onto the kernel callbacks; its proof is the `#[ignore]` live
//! mount test, which needs the macFUSE kext loaded.
//!
//! Plan: docs/plans/rustyred-code-workspace/W6-fuse-endgame.md

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    Request,
};

/// The content the filesystem serves: a flat path namespace mapping to bytes.
/// This is exactly the DocTree's shape, so the live adapter (Engine.fs_ls /
/// fs_read) is a thin impl; an in-memory impl drives the tests.
pub trait FileSource: Send + 'static {
    /// All file paths (slash-relative, no leading slash), source-only by the time
    /// they reach here (W0's import excludes build artifacts).
    fn paths(&self) -> Vec<String>;
    /// A file's bytes, or `None` if absent.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
}

/// Whether a path is a file or an (implicit) directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
}

/// One immediate child of a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub kind: NodeKind,
}

/// Trim leading/trailing slashes; `""` is the root.
fn normalize(path: &str) -> String {
    path.trim_matches('/').to_string()
}

/// The pure DocTree -> directory-tree view over a flat list of file paths.
#[derive(Clone, Debug)]
pub struct DirView {
    files: Vec<String>,
}

impl DirView {
    pub fn new(mut files: Vec<String>) -> Self {
        for f in files.iter_mut() {
            *f = normalize(f);
        }
        files.retain(|f| !f.is_empty());
        files.sort();
        files.dedup();
        Self { files }
    }

    /// The kind of a path: `Dir` (root, or any prefix that has descendants),
    /// `File` (an exact file path), or `None`. Directory wins if a path is both,
    /// so a directory is never shadowed by a same-named file.
    pub fn kind(&self, path: &str) -> Option<NodeKind> {
        let path = normalize(path);
        if path.is_empty() {
            return Some(NodeKind::Dir); // root
        }
        let dir_prefix = format!("{path}/");
        if self.files.iter().any(|f| f.starts_with(&dir_prefix)) {
            return Some(NodeKind::Dir);
        }
        if self.files.iter().any(|f| f == &path) {
            return Some(NodeKind::File);
        }
        None
    }

    /// The immediate children of a directory (not its full descendant set),
    /// sorted by name. Directory wins over file for a same-named entry.
    pub fn entries_in(&self, dir: &str) -> Vec<DirEntry> {
        let dir = normalize(dir);
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut children: BTreeMap<String, NodeKind> = BTreeMap::new();
        for file in &self.files {
            if !file.starts_with(&prefix) {
                continue;
            }
            let rest = &file[prefix.len()..];
            if rest.is_empty() {
                continue;
            }
            let (name, kind) = match rest.find('/') {
                Some(i) => (&rest[..i], NodeKind::Dir),
                None => (rest, NodeKind::File),
            };
            let entry = children.entry(name.to_string()).or_insert(kind);
            if kind == NodeKind::Dir {
                *entry = NodeKind::Dir;
            }
        }
        children
            .into_iter()
            .map(|(name, kind)| DirEntry { name, kind })
            .collect()
    }
}

/// A stable inode <-> path table. Root is inode 1 (path `""`); other inodes are
/// assigned lazily on first reference and never reused (read-only slice).
#[derive(Debug)]
pub struct Inodes {
    by_ino: HashMap<u64, String>,
    by_path: HashMap<String, u64>,
    next: u64,
}

impl Default for Inodes {
    fn default() -> Self {
        let mut by_ino = HashMap::new();
        let mut by_path = HashMap::new();
        by_ino.insert(1, String::new());
        by_path.insert(String::new(), 1);
        Self {
            by_ino,
            by_path,
            next: 2,
        }
    }
}

impl Inodes {
    pub fn intern(&mut self, path: &str) -> u64 {
        if let Some(&ino) = self.by_path.get(path) {
            return ino;
        }
        let ino = self.next;
        self.next += 1;
        self.by_ino.insert(ino, path.to_string());
        self.by_path.insert(path.to_string(), ino);
        ino
    }

    pub fn path(&self, ino: u64) -> Option<&str> {
        self.by_ino.get(&ino).map(String::as_str)
    }
}

const TTL: Duration = Duration::from_secs(1);

fn attr_for(ino: u64, kind: NodeKind, size: u64) -> FileAttr {
    let now: SystemTime = UNIX_EPOCH;
    let (ftype, perm, nlink) = match kind {
        NodeKind::Dir => (FileType::Directory, 0o555, 2),
        NodeKind::File => (FileType::RegularFile, 0o444, 1),
    };
    FileAttr {
        ino,
        size,
        blocks: 0,
        atime: now,
        mtime: now,
        ctime: now,
        crtime: now,
        kind: ftype,
        perm,
        nlink,
        uid: 0,
        gid: 0,
        rdev: 0,
        blksize: 512,
        flags: 0,
    }
}

/// A read-only FUSE filesystem over a [`FileSource`]. Source paths are snapshotted
/// into a [`DirView`] at construction (read-only slice; a read-write slice would
/// rebuild on write).
pub struct DocTreeFs<S: FileSource> {
    source: S,
    view: DirView,
    inodes: Inodes,
}

impl<S: FileSource> DocTreeFs<S> {
    pub fn new(source: S) -> Self {
        let view = DirView::new(source.paths());
        Self {
            source,
            view,
            inodes: Inodes::default(),
        }
    }

    fn size_of(&self, path: &str, kind: NodeKind) -> u64 {
        match kind {
            NodeKind::File => self.source.read(path).map(|b| b.len() as u64).unwrap_or(0),
            NodeKind::Dir => 0,
        }
    }
}

impl<S: FileSource> Filesystem for DocTreeFs<S> {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let Some(parent_path) = self.inodes.path(parent).map(str::to_string) else {
            reply.error(libc::ENOENT);
            return;
        };
        let Some(name) = name.to_str() else {
            reply.error(libc::ENOENT);
            return;
        };
        let child = if parent_path.is_empty() {
            name.to_string()
        } else {
            format!("{parent_path}/{name}")
        };
        match self.view.kind(&child) {
            Some(kind) => {
                let ino = self.inodes.intern(&child);
                let size = self.size_of(&child, kind);
                reply.entry(&TTL, &attr_for(ino, kind, size), 0);
            }
            None => reply.error(libc::ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        let Some(path) = self.inodes.path(ino).map(str::to_string) else {
            reply.error(libc::ENOENT);
            return;
        };
        match self.view.kind(&path) {
            Some(kind) => {
                let size = self.size_of(&path, kind);
                reply.attr(&TTL, &attr_for(ino, kind, size));
            }
            None => reply.error(libc::ENOENT),
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let Some(path) = self.inodes.path(ino).map(str::to_string) else {
            reply.error(libc::ENOENT);
            return;
        };
        match self.source.read(&path) {
            Some(bytes) => {
                let start = (offset.max(0) as usize).min(bytes.len());
                let end = start.saturating_add(size as usize).min(bytes.len());
                reply.data(&bytes[start..end]);
            }
            None => reply.error(libc::ENOENT),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let Some(dir_path) = self.inodes.path(ino).map(str::to_string) else {
            reply.error(libc::ENOENT);
            return;
        };
        // `.` and `..` first, then the immediate children.
        let mut rows: Vec<(u64, FileType, String)> = vec![
            (ino, FileType::Directory, ".".to_string()),
            (ino, FileType::Directory, "..".to_string()),
        ];
        for entry in self.view.entries_in(&dir_path) {
            let child = if dir_path.is_empty() {
                entry.name.clone()
            } else {
                format!("{dir_path}/{}", entry.name)
            };
            let cino = self.inodes.intern(&child);
            let ftype = match entry.kind {
                NodeKind::Dir => FileType::Directory,
                NodeKind::File => FileType::RegularFile,
            };
            rows.push((cino, ftype, entry.name));
        }
        for (i, (eino, ftype, name)) in rows.into_iter().enumerate().skip(offset as usize) {
            // The offset handed back is the index of the NEXT entry.
            if reply.add(eino, (i + 1) as i64, ftype, name) {
                break; // reply buffer full
            }
        }
        reply.ok();
    }
}

/// Mount `source` read-only at `mountpoint` (blocking until unmounted). Needs the
/// macFUSE kext loaded.
pub fn mount_readonly<S: FileSource>(source: S, mountpoint: &Path) -> std::io::Result<()> {
    let options = vec![
        MountOption::RO,
        MountOption::FSName("rustyred".to_string()),
        MountOption::Subtype("doctree".to_string()),
    ];
    fuser::mount2(DocTreeFs::new(source), mountpoint, &options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> DirView {
        DirView::new(vec![
            "src/main.rs".into(),
            "src/util.rs".into(),
            "README.md".into(),
            "data/big.bin".into(),
            "src/inner/deep.rs".into(),
        ])
    }

    #[test]
    fn kind_distinguishes_files_dirs_root_and_absent() {
        let v = view();
        assert_eq!(v.kind(""), Some(NodeKind::Dir), "root is a dir");
        assert_eq!(v.kind("src"), Some(NodeKind::Dir));
        assert_eq!(v.kind("src/inner"), Some(NodeKind::Dir));
        assert_eq!(v.kind("src/main.rs"), Some(NodeKind::File));
        assert_eq!(v.kind("README.md"), Some(NodeKind::File));
        assert_eq!(v.kind("nope"), None);
        assert_eq!(v.kind("src/nope.rs"), None);
    }

    #[test]
    fn entries_in_returns_immediate_children_only() {
        let v = view();
        assert_eq!(
            v.entries_in(""),
            vec![
                DirEntry { name: "README.md".into(), kind: NodeKind::File },
                DirEntry { name: "data".into(), kind: NodeKind::Dir },
                DirEntry { name: "src".into(), kind: NodeKind::Dir },
            ],
            "root lists immediate children, dirs not flattened"
        );
        assert_eq!(
            v.entries_in("src"),
            vec![
                DirEntry { name: "inner".into(), kind: NodeKind::Dir },
                DirEntry { name: "main.rs".into(), kind: NodeKind::File },
                DirEntry { name: "util.rs".into(), kind: NodeKind::File },
            ],
            "src lists only its direct children (inner is a dir, not inner/deep.rs)"
        );
        assert_eq!(
            v.entries_in("src/inner"),
            vec![DirEntry { name: "deep.rs".into(), kind: NodeKind::File }]
        );
        assert!(v.entries_in("data").iter().any(|e| e.name == "big.bin"));
    }

    #[test]
    fn dir_wins_over_a_same_named_file() {
        // Invalid tree: "x" is both a file and a directory prefix. The directory
        // must win so it is never shadowed.
        let v = DirView::new(vec!["x".into(), "x/y.rs".into()]);
        assert_eq!(v.kind("x"), Some(NodeKind::Dir));
        assert_eq!(
            v.entries_in(""),
            vec![DirEntry { name: "x".into(), kind: NodeKind::Dir }]
        );
    }

    #[test]
    fn inodes_are_stable_and_root_is_one() {
        let mut inodes = Inodes::default();
        assert_eq!(inodes.path(1), Some(""), "root is inode 1");
        let a = inodes.intern("src/main.rs");
        let b = inodes.intern("src/main.rs");
        assert_eq!(a, b, "same path -> same inode");
        assert_ne!(a, inodes.intern("README.md"), "distinct paths -> distinct inodes");
        assert_eq!(inodes.path(a), Some("src/main.rs"));
    }

    // Live mount: proves the kernel actually serves DocTree files through the
    // mount. Needs the macFUSE kext loaded (System Settings -> Privacy & Security
    // approval, then reboot). Run with: cargo test -- --ignored
    #[test]
    #[ignore = "needs macFUSE kext loaded (approve + reboot); run with --ignored"]
    fn live_mount_reads_files_through_the_kernel() {
        struct Mem(Vec<(String, Vec<u8>)>);
        impl FileSource for Mem {
            fn paths(&self) -> Vec<String> {
                self.0.iter().map(|(p, _)| p.clone()).collect()
            }
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                self.0.iter().find(|(p, _)| p == path).map(|(_, b)| b.clone())
            }
        }
        let source = Mem(vec![
            ("src/main.rs".to_string(), b"fn main() {}".to_vec()),
            ("README.md".to_string(), b"# hi".to_vec()),
        ]);

        let mountpoint = std::env::temp_dir().join(format!("rustyred-fuse-{}", std::process::id()));
        std::fs::create_dir_all(&mountpoint).unwrap();
        let session = fuser::spawn_mount2(
            DocTreeFs::new(source),
            &mountpoint,
            &[MountOption::RO, MountOption::FSName("rustyred".into())],
        )
        .expect("mount (needs macFUSE kext loaded)");

        // Give the mount a moment, then read a file through the kernel.
        std::thread::sleep(Duration::from_millis(200));
        let body = std::fs::read(mountpoint.join("src/main.rs")).expect("read through mount");
        assert_eq!(body, b"fn main() {}");
        let mut listing: Vec<_> = std::fs::read_dir(&mountpoint)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        listing.sort();
        assert_eq!(listing, vec!["README.md".to_string(), "src".to_string()]);

        drop(session); // unmounts
        let _ = std::fs::remove_dir_all(&mountpoint);
    }
}
