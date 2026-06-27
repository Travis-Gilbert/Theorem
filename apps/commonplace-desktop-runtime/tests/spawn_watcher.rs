//! Slice 4 acceptance (Part 4a): the stoppable background watcher.
//!
//! [`spawn_watcher`] runs the watch loop on a thread and returns a
//! [`WatchHandle`]; [`spawn_ambient`] is the one-call entry that wires the
//! durable commonplace sink + default passes to it. Coverage:
//!
//! * `spawn_ambient` over a tempdir, write a file: the ambient layer durably
//!   ingests it into the sidecar graph (verified after a clean stop), then the
//!   thread joins without hanging;
//! * `spawn_watcher` is sink-generic: a `RecordingSink` sees a real write
//!   delivered through the threaded loop;
//! * dropping the handle also stops the watcher cleanly (no hang).
//!
//! These are the first tests that drive the REAL notify event source through the
//! ambient runtime, so they also pin two boundary fixes the live watcher exposed:
//! the sidecar self-ignore and the relative-path keying must both survive the
//! macOS FSEvents canonicalized-path form (`/private/var/...`).

use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::{Duration, Instant};

use commonplace_desktop_runtime::{
    spawn_ambient, spawn_watcher, ChangeKind, CommonplaceIngestSink, RecordingSink, WatchConfig,
    FILE_SOURCE,
};

/// Poll `condition` until it returns true or `timeout` elapses. Returns whether
/// the condition was observed. Keeps the threaded tests from racing the watcher.
fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    condition()
}

/// Run `f` on a thread and assert it finishes within `timeout` (i.e. the watcher
/// stops/joins without hanging). Fails the test rather than blocking forever.
fn assert_completes_within(timeout: Duration, f: impl FnOnce() + Send + 'static) {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        f();
        // Ignore send errors: if the assertion below already timed out and the
        // receiver was dropped, there is nothing to report.
        let _ = tx.send(());
    });
    match rx.recv_timeout(timeout) {
        Ok(()) => worker.join().expect("worker thread should not panic"),
        Err(RecvTimeoutError::Timeout) => panic!("operation did not complete within {timeout:?}"),
        Err(RecvTimeoutError::Disconnected) => {
            panic!("worker dropped the channel without completing")
        }
    }
}

#[test]
fn spawn_ambient_records_a_real_write_then_stops_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    // A short debounce keeps the test responsive; the stop poll is shorter still.
    let mut config = WatchConfig::new(&root);
    config.debounce = Duration::from_millis(150);

    let handle = spawn_ambient(config.clone()).expect("spawn the bundled ambient layer");

    // Give the spawned thread a moment to establish the recursive watch before
    // writing, so the create event is not raced by a just-started watcher.
    thread::sleep(Duration::from_millis(400));
    // Write a text file into the watched tree; the watcher should ingest it.
    std::fs::write(root.join("note.md"), b"# hello from the watcher").unwrap();

    // We cannot open a second handle on the live sidecar to poll (RedCoreGraphStore
    // holds a process-level directory lock while the watcher owns it). So: wait out
    // the debounce + ingest, then stop the watcher (which joins its thread and
    // releases the lock), and verify the ambient layer DURABLY ingested the file by
    // reopening the sidecar. The reopen succeeding is itself proof the watcher
    // released the lock on a clean stop; the item being keyed by its RELATIVE path
    // proves the watcher's canonicalized paths were normalized against the root.
    thread::sleep(Duration::from_secs(2));
    assert_completes_within(Duration::from_secs(10), move || {
        handle.stop().expect("watcher stops cleanly");
    });

    let item = wait_until(Duration::from_secs(10), || {
        // Reopen may briefly race the OS releasing the lock; retry until it opens.
        let Ok(sink) = CommonplaceIngestSink::open(&config) else {
            return false;
        };
        sink.commonplace()
            .item_by_source_ref(FILE_SOURCE, "note.md")
            .ok()
            .flatten()
            .is_some()
    });
    assert!(
        item,
        "spawn_ambient ingested note.md into the durable sidecar, keyed by its relative path"
    );
}

#[test]
fn spawn_watcher_is_sink_generic_and_delivers_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let mut config = WatchConfig::new(&root);
    config.debounce = Duration::from_millis(150);

    // A RecordingSink (the reference sink) proves spawn_watcher is generic over
    // any ChangeSink, not just the ambient runtime.
    let sink = RecordingSink::new();
    let observer = sink.clone();
    let handle = spawn_watcher(config, sink).expect("spawn over a recording sink");

    // Let the spawned thread establish the recursive watch before writing.
    thread::sleep(Duration::from_millis(400));
    std::fs::write(root.join("data.txt"), b"payload").unwrap();

    let saw_write = wait_until(Duration::from_secs(15), || {
        observer.batches().iter().any(|batch| {
            batch.changes.iter().any(|change| {
                change.path.ends_with("data.txt")
                    && matches!(change.kind, ChangeKind::Created | ChangeKind::Modified)
            })
        })
    });
    assert!(
        saw_write,
        "the threaded watch loop should deliver the write to the sink"
    );

    assert_completes_within(Duration::from_secs(10), move || {
        handle.stop().expect("watcher stops cleanly");
    });
}

#[test]
fn dropping_the_handle_stops_the_watcher_without_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let mut config = WatchConfig::new(&root);
    config.debounce = Duration::from_millis(150);

    // Drop the handle on a worker thread and assert the drop (which signals +
    // joins the watch thread) returns promptly rather than blocking forever.
    assert_completes_within(Duration::from_secs(10), move || {
        let handle = spawn_ambient(config).expect("spawn the ambient layer");
        assert!(handle.is_running(), "the watcher is running after spawn");
        drop(handle); // Drop must signal-and-join, not hang on the rx loop.
    });
}
