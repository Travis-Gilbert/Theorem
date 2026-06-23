//! End-to-end proof of the graph-level hook dispatch path against a real
//! `RedCoreGraphStore`. Covers the spec's observable acceptance criteria for
//! the primitive itself (coalescing, failure isolation, loop convergence, and
//! the off-critical-path non-blocking emit). Handler-specific criteria
//! (centrality population, crawl reprioritization) live with their plugins.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rustyred_thg_core::{
    HookContext, HookDispatcher, HookDispatcherConfig, HookError, HookHandler, HookOutcome,
    HookRegistration, MutationEvent, MutationKind, MutationMatcher, NodeRecord, RedCoreGraphStore,
};
use serde_json::json;

// coalesce_key must be a plain fn pointer (no captures).
fn coalesce_all(_event: &MutationEvent) -> Option<String> {
    Some("all".to_string())
}

fn coalesce_per_id(_event: &MutationEvent) -> Option<String> {
    None
}

fn fast_config() -> HookDispatcherConfig {
    HookDispatcherConfig {
        debounce: Duration::from_millis(80),
        idle_poll: Duration::from_millis(10),
        ..Default::default()
    }
}

fn shared_store() -> Arc<Mutex<RedCoreGraphStore>> {
    Arc::new(Mutex::new(RedCoreGraphStore::memory()))
}

/// Acceptance 1: an ingest of N CodeSymbol upserts fires the hook exactly once
/// per coalesced batch, sees all N events, and its derived write lands.
#[test]
fn coalesces_ingest_storm_into_one_call_and_writes_back() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(AtomicUsize::new(0));
    let calls_h = Arc::clone(&calls);
    let seen_h = Arc::clone(&seen);

    let handler: HookHandler = Arc::new(move |ctx: &mut HookContext, events: &[MutationEvent]| {
        calls_h.fetch_add(1, Ordering::SeqCst);
        seen_h.fetch_add(events.len(), Ordering::SeqCst);
        // Derived write-back: a summary node carrying the coalesced count.
        ctx.store.upsert_node(NodeRecord::new(
            "repo-summary",
            ["RepoSummary"],
            json!({ "symbol_count": events.len() }),
        ))?;
        Ok(HookOutcome::Wrote { mutations: 1 })
    });

    let reg = HookRegistration::new(
        "counter",
        MutationMatcher::any()
            .with_kinds([MutationKind::NodeUpserted])
            .with_labels(["CodeSymbol"]),
        coalesce_all,
        handler,
    );

    let store = shared_store();
    let dispatcher = HookDispatcher::start(Arc::clone(&store), vec![reg], fast_config());
    store
        .lock()
        .unwrap()
        .attach_hook_emitter(dispatcher.emitter());

    for i in 0..50 {
        store
            .lock()
            .unwrap()
            .upsert_node(NodeRecord::new(
                format!("code:symbol:{i}"),
                ["CodeSymbol"],
                json!({ "repo_id": "repo:x", "name": format!("f{i}") }),
            ))
            .unwrap();
    }

    assert!(dispatcher.quiesce(Duration::from_secs(5)), "queue drained");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "one coalesced handler call"
    );
    assert_eq!(seen.load(Ordering::SeqCst), 50, "handler saw all 50 events");

    let summary = store.lock().unwrap().get_node("repo-summary").unwrap();
    assert_eq!(
        summary.expect("summary written back").properties["symbol_count"],
        json!(50)
    );
}

/// Acceptance 3: a handler that returns Err is logged + skipped; the triggering
/// write still commits, the store stays usable, and the queue keeps draining
/// (a subsequent good hook still fires).
#[test]
fn handler_error_is_isolated_and_queue_keeps_draining() {
    let good_calls = Arc::new(AtomicUsize::new(0));
    let gc = Arc::clone(&good_calls);

    let boom: HookHandler = Arc::new(|_ctx: &mut HookContext, _events: &[MutationEvent]| {
        Err(HookError::new("intentional handler failure"))
    });
    let boom_reg = HookRegistration::new(
        "boom",
        MutationMatcher::any().with_labels(["Boom"]),
        coalesce_per_id,
        boom,
    );

    let good: HookHandler = Arc::new(move |ctx: &mut HookContext, events: &[MutationEvent]| {
        gc.fetch_add(1, Ordering::SeqCst);
        ctx.store.upsert_node(NodeRecord::new(
            "good-marker",
            ["Marker"],
            json!({ "n": events.len() }),
        ))?;
        Ok(HookOutcome::Done)
    });
    let good_reg = HookRegistration::new(
        "good",
        MutationMatcher::any().with_labels(["Good"]),
        coalesce_per_id,
        good,
    );

    let store = shared_store();
    let dispatcher =
        HookDispatcher::start(Arc::clone(&store), vec![boom_reg, good_reg], fast_config());
    store
        .lock()
        .unwrap()
        .attach_hook_emitter(dispatcher.emitter());

    store
        .lock()
        .unwrap()
        .upsert_node(NodeRecord::new("b1", ["Boom"], json!({})))
        .unwrap();
    assert!(dispatcher.quiesce(Duration::from_secs(5)));

    // Triggering write committed despite the handler erroring.
    assert!(store.lock().unwrap().get_node("b1").unwrap().is_some());
    // Store remains usable.
    store
        .lock()
        .unwrap()
        .upsert_node(NodeRecord::new("after", ["X"], json!({})))
        .unwrap();
    assert!(store.lock().unwrap().get_node("after").unwrap().is_some());

    // Queue still draining: the good hook fires on a later write.
    store
        .lock()
        .unwrap()
        .upsert_node(NodeRecord::new("g1", ["Good"], json!({})))
        .unwrap();
    assert!(dispatcher.quiesce(Duration::from_secs(5)));
    assert_eq!(good_calls.load(Ordering::SeqCst), 1);
    assert!(store
        .lock()
        .unwrap()
        .get_node("good-marker")
        .unwrap()
        .is_some());
}

/// Extends acceptance 3: a panicking handler must not poison the store mutex.
/// After the panic the store still locks and serves reads/writes.
#[test]
fn handler_panic_does_not_poison_the_store() {
    let after_calls = Arc::new(AtomicUsize::new(0));
    let ac = Arc::clone(&after_calls);

    let panicky: HookHandler = Arc::new(|_ctx: &mut HookContext, _events: &[MutationEvent]| {
        panic!("intentional handler panic");
    });
    let panic_reg = HookRegistration::new(
        "panic",
        MutationMatcher::any().with_labels(["Panic"]),
        coalesce_per_id,
        panicky,
    );
    let after: HookHandler = Arc::new(move |_ctx: &mut HookContext, _events: &[MutationEvent]| {
        ac.fetch_add(1, Ordering::SeqCst);
        Ok(HookOutcome::Done)
    });
    let after_reg = HookRegistration::new(
        "after",
        MutationMatcher::any().with_labels(["After"]),
        coalesce_per_id,
        after,
    );

    let store = shared_store();
    let dispatcher = HookDispatcher::start(
        Arc::clone(&store),
        vec![panic_reg, after_reg],
        fast_config(),
    );
    store
        .lock()
        .unwrap()
        .attach_hook_emitter(dispatcher.emitter());

    store
        .lock()
        .unwrap()
        .upsert_node(NodeRecord::new("p1", ["Panic"], json!({})))
        .unwrap();
    assert!(dispatcher.quiesce(Duration::from_secs(5)));

    // Mutex not poisoned: lock still succeeds and the store is consistent.
    store
        .lock()
        .unwrap()
        .upsert_node(NodeRecord::new("p2", ["X"], json!({})))
        .unwrap();
    assert!(store.lock().unwrap().get_node("p2").unwrap().is_some());

    // The worker survived the panic and keeps dispatching.
    store
        .lock()
        .unwrap()
        .upsert_node(NodeRecord::new("a1", ["After"], json!({})))
        .unwrap();
    assert!(dispatcher.quiesce(Duration::from_secs(5)));
    assert_eq!(after_calls.load(Ordering::SeqCst), 1);
}

/// Acceptance 5: a handler that writes a property feeding its own matcher
/// converges (bounded by max_depth) instead of looping forever.
#[test]
fn self_feeding_hook_converges_at_max_depth() {
    let runs = Arc::new(AtomicUsize::new(0));
    let r = Arc::clone(&runs);

    // Always writes a fresh value, so every generation would re-trigger if not
    // for the depth guard.
    let loopy: HookHandler = Arc::new(move |ctx: &mut HookContext, events: &[MutationEvent]| {
        r.fetch_add(1, Ordering::SeqCst);
        for event in events {
            let tick = ctx
                .store
                .get_node(&event.id)
                .ok()
                .flatten()
                .and_then(|node| node.properties.get("tick").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            ctx.store.upsert_node(NodeRecord::new(
                &event.id,
                ["Loopy"],
                json!({ "tick": tick + 1 }),
            ))?;
        }
        Ok(HookOutcome::Done)
    });
    let reg = HookRegistration::new(
        "loopy",
        MutationMatcher::any()
            .with_kinds([MutationKind::NodeUpserted])
            .with_labels(["Loopy"]),
        coalesce_per_id,
        loopy,
    );

    let store = shared_store();
    let config = HookDispatcherConfig {
        debounce: Duration::from_millis(20),
        idle_poll: Duration::from_millis(10),
        max_depth: 3,
        ..Default::default()
    };
    let dispatcher = HookDispatcher::start(Arc::clone(&store), vec![reg], config);
    store
        .lock()
        .unwrap()
        .attach_hook_emitter(dispatcher.emitter());

    store
        .lock()
        .unwrap()
        .upsert_node(NodeRecord::new("loop1", ["Loopy"], json!({ "tick": 0 })))
        .unwrap();

    assert!(dispatcher.quiesce(Duration::from_secs(5)), "converged");
    let n = runs.load(Ordering::SeqCst);
    // Foreground write is depth 0 -> handler gen1 (writes depth1) -> gen2
    // (depth2) -> gen3 (depth3) -> depth3 event dropped at the guard. Bounded.
    assert!(n >= 1 && n <= 3, "bounded by max_depth, got {n}");

    let tick = store
        .lock()
        .unwrap()
        .get_node("loop1")
        .unwrap()
        .unwrap()
        .properties["tick"]
        .as_u64()
        .unwrap();
    assert!(tick >= 1 && tick <= 3, "tick bounded, got {tick}");
}

/// Acceptance 4 (mechanic): the writer's emit is non-blocking and off the
/// critical path. A write returns immediately even when the matched handler is
/// slow, because emit is a try-send and the worker runs the handler later.
#[test]
fn write_does_not_block_on_a_slow_handler() {
    let done = Arc::new(AtomicBool::new(false));
    let d = Arc::clone(&done);

    let slow: HookHandler = Arc::new(move |_ctx: &mut HookContext, _events: &[MutationEvent]| {
        thread::sleep(Duration::from_millis(300));
        d.store(true, Ordering::SeqCst);
        Ok(HookOutcome::Done)
    });
    let reg = HookRegistration::new(
        "slow",
        MutationMatcher::any().with_labels(["Slow"]),
        coalesce_per_id,
        slow,
    );

    let store = shared_store();
    let config = HookDispatcherConfig {
        debounce: Duration::from_millis(5),
        idle_poll: Duration::from_millis(5),
        ..Default::default()
    };
    let dispatcher = HookDispatcher::start(Arc::clone(&store), vec![reg], config);
    store
        .lock()
        .unwrap()
        .attach_hook_emitter(dispatcher.emitter());

    {
        let mut guard = store.lock().unwrap();
        let t0 = Instant::now();
        guard
            .upsert_node(NodeRecord::new("s1", ["Slow"], json!({})))
            .unwrap();
        let elapsed = t0.elapsed();
        // The write returned without waiting on the 300ms handler. (The worker
        // also cannot have started: it needs this very lock to run a handler.)
        assert!(
            elapsed < Duration::from_millis(100),
            "write blocked on hook? took {elapsed:?}"
        );
        assert!(
            !done.load(Ordering::SeqCst),
            "handler ran on the writer path"
        );
    }

    assert!(dispatcher.quiesce(Duration::from_secs(2)));
    assert!(
        done.load(Ordering::SeqCst),
        "handler eventually ran off-path"
    );
}

/// A store with no emitter attached behaves exactly as before: writes succeed,
/// nothing is dispatched. Proves the seam is strictly opt-in.
#[test]
fn no_emitter_attached_is_a_noop() {
    let mut store = RedCoreGraphStore::memory();
    assert!(!store.has_hook_emitter());
    store
        .upsert_node(NodeRecord::new("n1", ["X"], json!({ "a": 1 })))
        .unwrap();
    assert!(store.get_node("n1").unwrap().is_some());
}
