//! Notify-backed ambient watcher for the RustyRed desktop runtime.
//!
//! Watches a canonical git working tree and emits settled change sets. This
//! layer is READ-ONLY on the tree: it never writes tracked files (the
//! "canonical-git boundary" from the ambient-layer handoff). Settled change
//! sets are handed to a [`ChangeSink`], which is where ingest plus the ambient
//! passes (reconstruction, offload-eligible derivation, standing-seed
//! evaluation) run and write provenance into the RustyRed sidecar -- never back
//! into the tree.
//!
//! notify-rs (via `notify-debouncer-full`) is the event source; the debouncer
//! coalesces rename/create/modify/remove noise into a settled batch, so this
//! crate only has to map a settled batch to graph-facing changes.
//!
//! Slice 1 (this file) is the watcher core: config, the canonical-git boundary,
//! ignore handling, and the settled-change-set seam, all unit-testable plus one
//! real-filesystem acceptance test. The commonplace ingest sink and the ambient
//! passes are later slices implemented as [`ChangeSink`]s.

#![forbid(unsafe_code)]

mod authorization;
mod control;
mod discovery;
mod pairing;
mod passes;
mod presence;
mod process_executor;
mod relay;
mod runs;
mod sink;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify_debouncer_full::notify::event::EventKind;
use notify_debouncer_full::notify::{Event, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebouncedEvent};
use serde::{Deserialize, Serialize};

pub use authorization::{
    authorize_tier, ActionTierTable, AuthorizationDecision, TIER_ONE, TIER_THREE, TIER_TWO,
};
pub use control::{
    build_router as build_control_router, generate_pairing_code, serve as serve_control,
    BlobRefJson, CollectionJson, CollectionListResponse, ControlServer, ControlState, DeviceAuth,
    ItemDetailResponse, ItemJson, ItemListResponse, SearchHitJson, SearchResponse, LOOPBACK,
    PAIRING_CODE_HEADER,
};
pub use discovery::{
    advertise, browse, DiscoveredInstance, DiscoveryAdvertiser, ServiceAdvertisement,
    CONTROL_API_VERSION, SERVICE_TYPE, TXT_API_VERSION, TXT_INSTANCE_ID, TXT_LOOPBACK_PORT,
};
pub use relay::{
    InstanceFrame, MockRelay, MockRelayConnection, RelayClient, RelayClientConfig, RelayCredential,
    RelayFrame, TunnelRequest, TunnelResponse,
};
pub use pairing::{
    DevicePairing, DeviceSummary, LocalAccess, PairedDevice, PairingResult, VerifyOutcome,
    LOCAL_ACCESS_LABEL,
};
pub use runs::{
    MockExecutor, RunError, RunEvent, RunEventKind, RunEventSink, RunExecutor, RunRecord,
    RunRegistry, RunSpec, RunState,
};
pub use process_executor::{
    CommandFactory, ProcessRunExecutor, ResolvedCommand, ShellCommandFactory,
};
pub use presence::{
    ranges_overlap, AgentPresence, CodeEditFootprint, CodePresenceSnapshot, FileRange,
    FootprintAnnouncement, PresenceAnnouncement, PresenceKind, PresenceRegistry,
};
pub use passes::{
    write_pass_receipt, AmbientCycleReport, AmbientPass, AmbientRuntime, OffloadPass, PassReceipt,
    PassStatus, ReconstructionPass, SidecarCommonplace, StandingSeedPass, PASS_RECEIPT_LABEL,
    PRODUCED_FOR_EDGE,
};
pub use sink::{
    CommonplaceIngestSink, IngestOutcome, IngestedPath, SharedSink, SidecarCommonplaceStore,
    CHANGE_SET_LABEL, FILE_SOURCE, FOLLOWS_EDGE,
};

/// Crate result type. Setup errors (notify, ignore) flow through here.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// What kind of change a settled filesystem event represents.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Created,
    Modified,
    Removed,
}

/// A single settled change to one path in the working tree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: ChangeKind,
}

/// A settled batch of changes from one debounce window.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub changes: Vec<FileChange>,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }
}

/// Where settled change sets go. The production sink runs ingest plus the
/// ambient passes and writes provenance into the sidecar; per the canonical-git
/// boundary it must never write tracked files. Tests use [`RecordingSink`].
pub trait ChangeSink: Send {
    fn apply(&mut self, change_set: ChangeSet);
}

/// A sink that records every settled change set, for tests and as a reference
/// implementation. Cloneable so a test can hold a handle while the watcher owns
/// the sink.
#[derive(Clone, Debug, Default)]
pub struct RecordingSink {
    batches: Arc<Mutex<Vec<ChangeSet>>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every change set applied so far.
    pub fn batches(&self) -> Vec<ChangeSet> {
        self.batches.lock().expect("recording sink mutex").clone()
    }
}

impl ChangeSink for RecordingSink {
    fn apply(&mut self, change_set: ChangeSet) {
        self.batches
            .lock()
            .expect("recording sink mutex")
            .push(change_set);
    }
}

/// Watcher configuration. `sidecar_dir` is RustyRed's own state location; it is
/// always ignored so the watcher never reacts to its own writes.
#[derive(Clone, Debug)]
pub struct WatchConfig {
    /// The canonical git working tree to watch.
    pub root: PathBuf,
    /// RustyRed sidecar state directory (always ignored).
    pub sidecar_dir: PathBuf,
    /// Debounce window for coalescing filesystem events into a settled batch.
    pub debounce: Duration,
    /// Extra gitignore-syntax lines beyond the repo's own `.gitignore`.
    pub extra_ignores: Vec<String>,
}

impl WatchConfig {
    /// Default config: sidecar at `<root>/.rustyred`, a 400ms debounce.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let sidecar_dir = root.join(".rustyred");
        Self {
            root,
            sidecar_dir,
            debounce: Duration::from_millis(400),
            extra_ignores: Vec::new(),
        }
    }
}

/// Ignore matcher: honors the repo `.gitignore`, an explicit ignore list, the
/// RustyRed sidecar, and always `.git/`.
pub struct IgnoreMatcher {
    gitignore: Gitignore,
    /// The sidecar path as configured (covers a sidecar placed outside the root).
    sidecar_dir: PathBuf,
    /// The sidecar path canonicalized at build time, if it resolves. The watcher
    /// (FSEvents on macOS) reports canonicalized paths (`/private/var/...` for a
    /// `/var/...` symlink), so the configured prefix alone misses the sink's own
    /// writes and the watcher self-triggers a feedback loop. Comparing against the
    /// canonical prefix too closes that loop.
    sidecar_dir_canonical: Option<PathBuf>,
    /// The sidecar's final directory name (e.g. `.rustyred`). Matched as a path
    /// component anywhere in a path, the same way `.git` is, so the sidecar is
    /// ignored regardless of any symlink canonicalization of its parents.
    sidecar_component: Option<std::ffi::OsString>,
}

impl IgnoreMatcher {
    pub fn build(config: &WatchConfig) -> Result<Self> {
        let mut builder = GitignoreBuilder::new(&config.root);
        let gitignore_path = config.root.join(".gitignore");
        if gitignore_path.is_file() {
            let _ = builder.add(&gitignore_path);
        }
        for line in &config.extra_ignores {
            builder.add_line(None, line)?;
        }
        let sidecar_component = config
            .sidecar_dir
            .file_name()
            .map(|name| name.to_os_string());
        Ok(Self {
            gitignore: builder.build()?,
            sidecar_dir: config.sidecar_dir.clone(),
            // `canonicalize` requires the path to exist; the sink creates the
            // sidecar before the watcher starts, but tolerate it being absent.
            sidecar_dir_canonical: config.sidecar_dir.canonicalize().ok(),
            sidecar_component,
        })
    }

    /// Whether a path should be ignored by the ambient layer.
    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        if self.is_in_sidecar(path) {
            return true;
        }
        if path.components().any(|c| c.as_os_str() == ".git") {
            return true;
        }
        self.gitignore.matched(path, is_dir).is_ignore()
    }

    /// Whether a path is inside the RustyRed sidecar (so the watcher must never
    /// react to it). Checks the configured prefix, the canonicalized prefix (for
    /// FSEvents' `/private/...` paths), and the sidecar directory name as a path
    /// component (symlink-canonicalization-proof, like the `.git` check).
    fn is_in_sidecar(&self, path: &Path) -> bool {
        if path.starts_with(&self.sidecar_dir) {
            return true;
        }
        if self
            .sidecar_dir_canonical
            .as_ref()
            .is_some_and(|canonical| path.starts_with(canonical))
        {
            return true;
        }
        if let Some(component) = &self.sidecar_component {
            if path
                .components()
                .any(|c| c.as_os_str() == component.as_os_str())
            {
                return true;
            }
        }
        false
    }
}

/// Map one settled notify event to the file changes it represents, dropping
/// ignored paths and non-content events (access/metadata-only).
fn changes_from_event(event: &Event, matcher: &IgnoreMatcher) -> Vec<FileChange> {
    let kind = match event.kind {
        EventKind::Create(_) => ChangeKind::Created,
        EventKind::Modify(_) => ChangeKind::Modified,
        EventKind::Remove(_) => ChangeKind::Removed,
        EventKind::Access(_) | EventKind::Any | EventKind::Other => return Vec::new(),
    };
    event
        .paths
        .iter()
        .filter(|path| !matcher.is_ignored(path, path.is_dir()))
        .map(|path| FileChange {
            path: path.clone(),
            kind,
        })
        .collect()
}

/// Collapse a debounced batch into one settled [`ChangeSet`].
fn change_set_from_batch(events: &[DebouncedEvent], matcher: &IgnoreMatcher) -> ChangeSet {
    let mut changes = Vec::new();
    for event in events {
        changes.extend(changes_from_event(event, matcher));
    }
    ChangeSet { changes }
}

/// Run the ambient watcher, blocking the calling thread until the underlying
/// channel closes (the production runtime calls this on a dedicated thread).
/// Read-only on the tree: it only observes and forwards settled change sets.
///
/// This blocks forever in practice (the debouncer's sender outlives the loop),
/// so prefer [`spawn_watcher`] / [`spawn_ambient`] for an app that must stop the
/// watcher on shutdown. `run` stays as the simple, sink-generic blocking entry.
pub fn run(config: WatchConfig, mut sink: impl ChangeSink) -> Result<()> {
    let matcher = IgnoreMatcher::build(&config)?;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(config.debounce, None, tx)?;
    debouncer.watch(&config.root, RecursiveMode::Recursive)?;
    for result in rx {
        match result {
            Ok(events) => {
                let change_set = change_set_from_batch(&events, &matcher);
                if !change_set.is_empty() {
                    sink.apply(change_set);
                }
            }
            Err(_errors) => {
                // notify surfaced watch errors for this batch; keep watching.
                // The production sink/runtime records these as a degraded state.
            }
        }
    }
    drop(debouncer);
    Ok(())
}

/// How often the stoppable watch loop wakes to check the stop flag while no
/// filesystem events are arriving. Small enough that `stop()` is responsive,
/// large enough that an idle watcher does not busy-spin.
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A running watcher on a background thread. Hold this for as long as the watch
/// should run; call [`stop`](WatchHandle::stop) (or drop it) to end the loop and
/// join the thread cleanly. The desktop app stores this in app state and stops
/// it on shutdown.
///
/// Dropping the handle is equivalent to `stop()`: it signals the loop, drops the
/// debouncer (so the channel closes), and joins. A panicked watch thread is
/// surfaced through `stop()`'s `Result`; `Drop` swallows it after logging.
pub struct WatchHandle {
    /// Set true to ask the watch loop to exit at its next poll.
    stop: Arc<AtomicBool>,
    /// The watch thread; `take`n by `stop`/`Drop` so each joins at most once.
    thread: Option<JoinHandle<Result<()>>>,
}

impl WatchHandle {
    /// Signal the watch loop to stop and join the thread, surfacing any setup
    /// error or a panic from the watch thread. Returns `Ok(())` on a clean stop.
    pub fn stop(mut self) -> Result<()> {
        Self::shutdown(&mut self)
    }

    /// Whether the watch thread is still alive (has not exited or been joined).
    /// Useful for a health check; a watcher normally runs until `stop`.
    pub fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }

    /// Shared stop+join used by both [`stop`](Self::stop) and [`Drop`]. Signals
    /// the flag, joins the thread (which drops the debouncer on its way out), and
    /// flattens the join + the thread's own `Result`.
    fn shutdown(handle: &mut WatchHandle) -> Result<()> {
        handle.stop.store(true, Ordering::SeqCst);
        match handle.thread.take() {
            // Already joined (e.g. `stop()` ran, then `Drop`): nothing to do.
            None => Ok(()),
            Some(thread) => match thread.join() {
                Ok(thread_result) => thread_result,
                Err(_panic) => Err("ambient watch thread panicked".into()),
            },
        }
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        // Best-effort clean stop on drop so a forgotten handle still tears the
        // watcher down. Surface a failure on stderr; Drop cannot return it.
        if self.thread.is_some() {
            if let Err(error) = Self::shutdown(self) {
                eprintln!("commonplace-desktop-runtime: watcher shutdown on drop failed: {error}");
            }
        }
    }
}

/// Spawn the ambient watcher on a background thread and return a [`WatchHandle`]
/// that stops it cleanly. The loop is stoppable: it polls the stop flag on a
/// short timeout between filesystem events, so [`WatchHandle::stop`] (or dropping
/// the handle) ends the loop and joins without hanging.
///
/// Setup that must fail loudly (building the ignore matcher, starting the
/// debouncer, beginning the watch) runs on the spawned thread and is surfaced
/// through the handle's `stop()` result if it fails before the loop starts.
/// Read-only on the tree, exactly like [`run`].
pub fn spawn_watcher(
    config: WatchConfig,
    mut sink: impl ChangeSink + 'static,
) -> Result<WatchHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("ambient-watcher".to_string())
        .spawn(move || -> Result<()> {
            let matcher = IgnoreMatcher::build(&config)?;
            let (tx, rx) = std::sync::mpsc::channel();
            let mut debouncer = new_debouncer(config.debounce, None, tx)?;
            debouncer.watch(&config.root, RecursiveMode::Recursive)?;

            // Poll the channel on a short timeout so the stop flag is honored
            // even while the tree is idle. The debouncer is owned here and
            // dropped when the loop exits, closing the channel.
            while !thread_stop.load(Ordering::SeqCst) {
                match rx.recv_timeout(STOP_POLL_INTERVAL) {
                    Ok(Ok(events)) => {
                        let change_set = change_set_from_batch(&events, &matcher);
                        if !change_set.is_empty() {
                            sink.apply(change_set);
                        }
                    }
                    // notify surfaced watch errors for this batch; keep watching.
                    Ok(Err(_errors)) => {}
                    // No events this window: re-check the stop flag and loop.
                    Err(RecvTimeoutError::Timeout) => {}
                    // The sender was dropped (debouncer gone): nothing more will
                    // arrive, so exit the loop.
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            drop(debouncer);
            Ok(())
        })?;

    Ok(WatchHandle {
        stop,
        thread: Some(thread),
    })
}

/// The one-call entry point that starts the bundled ambient layer: it opens the
/// durable commonplace sink over the sidecar configured by `config`, registers
/// the default ambient passes, and spawns the stoppable watcher over it. This is
/// what the desktop app calls so a fresh download runs the watcher with no
/// separate install step.
///
/// Returns a [`WatchHandle`] to stop the layer on shutdown. The sink is opened
/// eagerly (before the thread spawns) so a sidecar open failure is reported here
/// rather than swallowed on the watch thread.
pub fn spawn_ambient(config: WatchConfig) -> Result<WatchHandle> {
    Ok(spawn_ambient_with_data(config)?.0)
}

/// Like [`spawn_ambient`], but also returns a [`SharedSink`] read handle onto the
/// SAME durable commonplace the watcher writes to. Hand the handle to
/// [`ControlState::with_data`](crate::ControlState::with_data) so the control
/// endpoint's data routes (`/v1/items*`, `/v1/collections`, `/v1/search`) serve
/// the live graph -- one process, one graph, no second AOF handle. This is the
/// local-first connection path: the desktop starts the watcher AND serves its
/// data from one in-process commonplace.
pub fn spawn_ambient_with_data(config: WatchConfig) -> Result<(WatchHandle, SharedSink)> {
    let sink = SharedSink::open(&config)?;
    let runtime = AmbientRuntime::from_shared(sink.clone()).default_passes();
    let handle = spawn_watcher(config, runtime)?;
    Ok((handle, sink))
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify_debouncer_full::notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};

    fn matcher_for(root: &Path) -> IgnoreMatcher {
        IgnoreMatcher::build(&WatchConfig::new(root)).unwrap()
    }

    #[test]
    fn maps_create_modify_remove_and_skips_access() {
        let root = PathBuf::from("/repo");
        let matcher = matcher_for(&root);
        let file = root.join("src/main.rs");
        let event = |kind| Event::new(kind).add_path(file.clone());

        let created = changes_from_event(&event(EventKind::Create(CreateKind::File)), &matcher);
        assert_eq!(
            created,
            vec![FileChange {
                path: file.clone(),
                kind: ChangeKind::Created,
            }]
        );
        assert_eq!(
            changes_from_event(&event(EventKind::Modify(ModifyKind::Any)), &matcher)[0].kind,
            ChangeKind::Modified
        );
        assert_eq!(
            changes_from_event(&event(EventKind::Remove(RemoveKind::File)), &matcher)[0].kind,
            ChangeKind::Removed
        );
        assert!(
            changes_from_event(&event(EventKind::Access(AccessKind::Any)), &matcher).is_empty(),
            "access events are not content changes"
        );
    }

    #[test]
    fn ignores_git_sidecar_and_gitignored_paths() {
        let root = PathBuf::from("/repo");
        let mut config = WatchConfig::new(&root);
        config.extra_ignores.push("*.log".to_string());
        let matcher = IgnoreMatcher::build(&config).unwrap();

        assert!(matcher.is_ignored(&root.join(".git/HEAD"), false));
        assert!(matcher.is_ignored(&root.join(".rustyred/state.db"), false));
        assert!(matcher.is_ignored(&root.join("debug.log"), false));
        assert!(!matcher.is_ignored(&root.join("src/main.rs"), false));
    }

    // Regression: the live watcher (FSEvents on macOS) reports canonicalized
    // paths whose parents differ from the configured root (`/private/var/...`
    // vs `/var/...`). The sidecar self-ignore must still fire on those, or the
    // sink's own writes re-trigger the watcher in a feedback loop. The `.rustyred`
    // directory name is matched as a path component, like `.git`, so a sidecar
    // path under ANY parent is ignored.
    #[test]
    fn ignores_sidecar_under_a_canonicalized_parent() {
        let configured_root = PathBuf::from("/var/folders/xx/repo");
        let matcher = matcher_for(&configured_root);

        // A sidecar write the watcher reports with the canonicalized parent the
        // configured root prefix would miss.
        let canonical_sidecar_write =
            PathBuf::from("/private/var/folders/xx/repo/.rustyred/graph/graph.aof.current");
        assert!(
            matcher.is_ignored(&canonical_sidecar_write, false),
            "the sidecar must be ignored even when the watcher canonicalizes its parents"
        );
        // A real tracked file under the same canonicalized parent is NOT ignored.
        let canonical_source = PathBuf::from("/private/var/folders/xx/repo/src/lib.rs");
        assert!(
            !matcher.is_ignored(&canonical_source, false),
            "a tracked source file is not collateral of the sidecar component match"
        );
    }

    // Acceptance: editing a file in the working tree produces a settled change
    // set, via the real notify event source.
    #[test]
    fn watcher_reports_a_real_file_write() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let config = WatchConfig {
            root: root.clone(),
            sidecar_dir: root.join(".rustyred"),
            debounce: Duration::from_millis(200),
            extra_ignores: Vec::new(),
        };
        let matcher = IgnoreMatcher::build(&config).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let mut debouncer = new_debouncer(config.debounce, None, tx).unwrap();
        debouncer
            .watch(&root, RecursiveMode::Recursive)
            .unwrap();

        std::fs::write(root.join("hello.txt"), b"hi").unwrap();

        let mut saw_hello = false;
        // Drain debounced batches until we observe the write or time out.
        while let Ok(result) = rx.recv_timeout(Duration::from_secs(10)) {
            if let Ok(events) = result {
                let change_set = change_set_from_batch(&events, &matcher);
                if change_set.changes.iter().any(|change| {
                    change.path.ends_with("hello.txt")
                        && matches!(change.kind, ChangeKind::Created | ChangeKind::Modified)
                }) {
                    saw_hello = true;
                    break;
                }
            }
        }
        drop(debouncer);
        assert!(saw_hello, "watcher should report the hello.txt write");
    }
}
