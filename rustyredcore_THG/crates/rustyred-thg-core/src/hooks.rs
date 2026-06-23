//! Graph-level mutation hooks: reactive compute that fires *inside* the store
//! after a mutation is durably committed.
//!
//! `Operation` (in [`crate::plugin`]) is RustyRed's stored-procedure equivalent:
//! compute invoked against the store on demand. `Hook` is the reactive twin:
//! compute that fires when the graph itself mutates. This module wires the
//! `Hook` capability that [`crate::plugin::PluginCapabilityKind::Hook`] only
//! declared.
//!
//! ## Hard guarantees (from the hook primitive spec)
//!
//! 1. **Post-commit, off the writer's critical path.** Triggers emit only after
//!    a mutation is durably committed, onto this bounded queue. The producer's
//!    [`HookEmitter::try_emit`] never blocks and never takes the writer's lock,
//!    so writer throughput is unchanged with hooks enabled.
//! 2. **Idempotent + coalesced.** Same-`(kind, id)` events merge in place in the
//!    bounded queue; the worker then groups by `(registration, tenant,
//!    coalesce_key)` so an ingest of 700 symbols coalesces into one handler call
//!    per repo, not 700. Handlers must therefore be safe to repeat.
//! 3. **Fail open.** A handler that returns `Err` *or panics* is logged and
//!    skipped. It never aborts the triggering write, never stalls the queue,
//!    and (via `catch_unwind`) never poisons the store mutex.
//! 4. **Handlers may write derived structure back.** A handler gets
//!    `&mut RedCoreGraphStore`. Those writes emit their own events, tagged one
//!    generation deeper, and the dispatcher refuses to dispatch past
//!    [`HookDispatcherConfig::max_depth`], so a self-feeding hook converges
//!    instead of looping.
//!
//! The substrate stays tokio-free: the dispatcher is a plain `std::thread`
//! worker draining a `Mutex`/`Condvar`-guarded coalescing buffer, matching the
//! `std::sync::{mpsc, Mutex}` + `std::thread` primitives `graph_store.rs`
//! already uses.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::graph_store::{GraphStoreError, RedCoreGraphStore};

/// The kind of mutation that produced an event. For edges, the event's
/// `labels` carries `[edge_type]`, so matchers filter node labels and edge
/// types through one field.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MutationKind {
    NodeUpserted,
    NodeDeleted,
    EdgeUpserted,
    EdgeDeleted,
}

impl MutationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MutationKind::NodeUpserted => "node_upserted",
            MutationKind::NodeDeleted => "node_deleted",
            MutationKind::EdgeUpserted => "edge_upserted",
            MutationKind::EdgeDeleted => "edge_deleted",
        }
    }

    /// Coarse value ranking used by the bounded-queue drop policy: structure
    /// that *creates* graph (upserts) outranks structure that removes it, so a
    /// full queue sheds the lowest-value class first.
    fn drop_rank(self) -> u8 {
        match self {
            MutationKind::NodeUpserted => 3,
            MutationKind::EdgeUpserted => 2,
            MutationKind::EdgeDeleted => 1,
            MutationKind::NodeDeleted => 0,
        }
    }
}

/// A single durable graph mutation, emitted post-commit by the store.
#[derive(Clone, Debug)]
pub struct MutationEvent {
    pub kind: MutationKind,
    pub tenant: String,
    /// Node id, or edge id for edge events.
    pub id: String,
    /// Node labels, or `[edge_type]` for edges.
    pub labels: Vec<String>,
    /// Property keys that changed on this commit.
    pub changed_props: Vec<String>,
    pub committed_at_ms: u64,
    /// Loop-guard generation of the write that produced this event. `0` is a
    /// foreground (non-hook) write; a handler reacting to generation `g`
    /// produces writes stamped `g + 1`.
    pub depth: u32,
}

impl MutationEvent {
    pub fn new(
        kind: MutationKind,
        tenant: impl Into<String>,
        id: impl Into<String>,
        labels: Vec<String>,
        changed_props: Vec<String>,
        committed_at_ms: u64,
        depth: u32,
    ) -> Self {
        Self {
            kind,
            tenant: tenant.into(),
            id: id.into(),
            labels,
            changed_props,
            committed_at_ms,
            depth,
        }
    }

    fn coalesce_identity(&self) -> (MutationKind, String) {
        (self.kind, self.id.clone())
    }
}

/// Declarative filter on which mutations a hook fires for. Every populated
/// field is a constraint; `None` means "any". Multiple constraints AND together.
#[derive(Clone, Debug, Default)]
pub struct MutationMatcher {
    /// Exact tenant ids; `None` = any tenant.
    pub tenants: Option<Vec<String>>,
    /// Any-of node labels (or edge types); `None` = any.
    pub labels: Option<Vec<String>>,
    /// Any-of mutation kinds; `None` = any.
    pub kinds: Option<Vec<MutationKind>>,
    /// Fire only if one of these property keys changed; `None` = any.
    pub changed_props_any: Option<Vec<String>>,
}

impl MutationMatcher {
    /// Matches every mutation.
    pub fn any() -> Self {
        Self::default()
    }

    pub fn with_tenants(mut self, tenants: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tenants = Some(tenants.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_labels(mut self, labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.labels = Some(labels.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_kinds(mut self, kinds: impl IntoIterator<Item = MutationKind>) -> Self {
        self.kinds = Some(kinds.into_iter().collect());
        self
    }

    pub fn with_changed_props_any(
        mut self,
        props: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.changed_props_any = Some(props.into_iter().map(Into::into).collect());
        self
    }

    /// True when `event` satisfies every populated constraint.
    pub fn matches(&self, event: &MutationEvent) -> bool {
        if let Some(tenants) = &self.tenants {
            if !tenants.iter().any(|t| t == &event.tenant) {
                return false;
            }
        }
        if let Some(kinds) = &self.kinds {
            if !kinds.contains(&event.kind) {
                return false;
            }
        }
        if let Some(labels) = &self.labels {
            if !labels
                .iter()
                .any(|wanted| event.labels.iter().any(|l| l == wanted))
            {
                return false;
            }
        }
        if let Some(props) = &self.changed_props_any {
            if !props
                .iter()
                .any(|wanted| event.changed_props.iter().any(|p| p == wanted))
            {
                return false;
            }
        }
        true
    }
}

/// Handler-facing context. The handle is `&mut RedCoreGraphStore` so a hook can
/// materialize centrality, embeddings, impact, or new nodes/edges back into the
/// graph. `depth` is the loop-guard generation this handler runs at; writes it
/// makes are emitted at `depth` and stop dispatching past `max_depth`.
pub struct HookContext<'a> {
    pub store: &'a mut RedCoreGraphStore,
    pub tenant: &'a str,
    pub depth: u32,
}

/// Outcome of a handler invocation. `Wrote` is for observability only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HookOutcome {
    Done,
    Wrote { mutations: usize },
}

/// Error a handler may return. Fails the handler open (logged + skipped), never
/// the triggering write. `From<GraphStoreError>` lets handlers `?` on writes.
#[derive(Clone, Debug)]
pub struct HookError {
    pub message: String,
}

impl HookError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for HookError {}

impl From<GraphStoreError> for HookError {
    fn from(err: GraphStoreError) -> Self {
        Self::new(format!("{}: {}", err.code, err.message))
    }
}

impl From<String> for HookError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for HookError {
    fn from(message: &str) -> Self {
        Self::new(message.to_string())
    }
}

/// Maps an event to its coalescing key. Events with the same key (under one
/// registration, in one drain window) collapse to a single handler call. Return
/// `None` to coalesce per-event-id (the default the dispatcher applies).
pub type CoalesceKeyFn = fn(&MutationEvent) -> Option<String>;

/// The handler closure. Takes the coalesced *batch* of events, not a single
/// event, so an ingest storm becomes one call.
pub type HookHandler =
    Arc<dyn Fn(&mut HookContext, &[MutationEvent]) -> Result<HookOutcome, HookError> + Send + Sync>;

/// One registered hook: a name, a matcher, a coalescing key, and the handler.
#[derive(Clone)]
pub struct HookRegistration {
    pub name: String,
    pub on: MutationMatcher,
    pub coalesce_key: CoalesceKeyFn,
    pub handler: HookHandler,
}

impl HookRegistration {
    pub fn new(
        name: impl Into<String>,
        on: MutationMatcher,
        coalesce_key: CoalesceKeyFn,
        handler: HookHandler,
    ) -> Self {
        Self {
            name: name.into(),
            on,
            coalesce_key,
            handler,
        }
    }
}

impl fmt::Debug for HookRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookRegistration")
            .field("name", &self.name)
            .field("on", &self.on)
            .finish_non_exhaustive()
    }
}

/// Default coalescing key: coalesce per event id.
pub fn coalesce_per_id(_event: &MutationEvent) -> Option<String> {
    None
}

/// Abstracts how the worker obtains exclusive access to the store to run
/// handlers. Decouples the dispatcher from the embedder's lock topology: a bare
/// `Arc<Mutex<RedCoreGraphStore>>` (the blanket impl below) or a server tenant
/// executor that owns its own writer mutex can both satisfy it.
///
/// `with_store_mut` returns `false` when the store is unavailable (poisoned or
/// gone). The worker treats that as "skip this dispatch generation" and never
/// unwraps a lock — a handler panic elsewhere must not be able to wedge hooks.
pub trait HookStoreAccess: Send + Sync {
    fn with_store_mut(&self, f: &mut dyn FnMut(&mut RedCoreGraphStore)) -> bool;
}

impl HookStoreAccess for Arc<Mutex<RedCoreGraphStore>> {
    fn with_store_mut(&self, f: &mut dyn FnMut(&mut RedCoreGraphStore)) -> bool {
        match self.lock() {
            Ok(mut guard) => {
                f(&mut guard);
                true
            }
            // Poisoned: a prior panic left the mutex tainted. Fail open rather
            // than unwrap-and-panic, per the spec's mutex-poisoning caution.
            Err(_poisoned) => false,
        }
    }
}

/// Tuning for the dispatcher. Defaults match the spec's stated examples.
#[derive(Clone, Debug)]
pub struct HookDispatcherConfig {
    /// Max distinct coalescing identities buffered before the drop policy fires.
    pub queue_capacity: usize,
    /// Debounce window: after the first event of a cycle, wait this long for the
    /// storm to coalesce before draining. `Duration::ZERO` drains immediately.
    pub debounce: Duration,
    /// Idle poll interval the worker waits on when the queue is empty (also the
    /// shutdown-responsiveness granularity).
    pub idle_poll: Duration,
    /// Refuse to dispatch events at or beyond this generation. Bounds hook-
    /// induced write chains so a self-feeding hook converges.
    pub max_depth: u32,
}

impl Default for HookDispatcherConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 8192,
            debounce: Duration::from_millis(250),
            idle_poll: Duration::from_millis(50),
            max_depth: 3,
        }
    }
}

/// A buffered, coalesced event plus how many source events it represents (for
/// the quiesce barrier accounting).
#[derive(Clone, Debug)]
struct QueuedEvent {
    event: MutationEvent,
    coalesced_count: u64,
}

#[derive(Default)]
struct QueueInner {
    /// Arrival order of coalescing identities (deterministic drain order).
    order: Vec<(MutationKind, String)>,
    by_key: HashMap<(MutationKind, String), QueuedEvent>,
}

impl QueueInner {
    fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

/// Shared state between the producer (`HookEmitter`, held by the store) and the
/// worker thread. Counters drive the quiesce barrier; `sent == processed` means
/// every accepted event has been dispatched, dropped, or skipped.
struct HookShared {
    queue: Mutex<QueueInner>,
    not_empty: Condvar,
    progress_lock: Mutex<()>,
    progress: Condvar,
    capacity: usize,
    sent: AtomicU64,
    processed: AtomicU64,
    dropped: AtomicU64,
    shutdown: AtomicBool,
}

impl HookShared {
    fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(QueueInner::default()),
            not_empty: Condvar::new(),
            progress_lock: Mutex::new(()),
            progress: Condvar::new(),
            capacity: capacity.max(1),
            sent: AtomicU64::new(0),
            processed: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
        }
    }

    /// Producer path. Non-blocking with respect to hooks: takes only the brief
    /// buffer lock, never waits on a handler. Coalesces same-identity events in
    /// place; on a full buffer sheds the lowest-value queued entry.
    fn emit(&self, event: MutationEvent) {
        // A poisoned buffer lock should never silently swallow events without a
        // trace, but it must also never panic the writer. Recover the guard.
        let mut inner = match self.queue.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        self.sent.fetch_add(1, Ordering::SeqCst);
        let identity = event.coalesce_identity();

        if let Some(existing) = inner.by_key.get_mut(&identity) {
            merge_event(&mut existing.event, event);
            existing.coalesced_count += 1;
            drop(inner);
            self.not_empty.notify_one();
            return;
        }

        if inner.by_key.len() >= self.capacity {
            if let Some(victim_key) = lowest_value_key(&inner) {
                if let Some(removed) = inner.by_key.remove(&victim_key) {
                    inner.order.retain(|k| k != &victim_key);
                    // The evicted entry's source events will never be handled;
                    // count them processed (dropped) so the barrier still closes.
                    self.processed
                        .fetch_add(removed.coalesced_count, Ordering::SeqCst);
                    self.dropped
                        .fetch_add(removed.coalesced_count, Ordering::SeqCst);
                    eprintln!(
                        "[rustyred-hook] queue full (cap={}); dropped lowest-value event {}:{} \
                         ({} coalesced)",
                        self.capacity,
                        victim_key.0.as_str(),
                        victim_key.1,
                        removed.coalesced_count
                    );
                    self.signal_progress();
                }
            }
        }

        inner.order.push(identity.clone());
        inner.by_key.insert(
            identity,
            QueuedEvent {
                event,
                coalesced_count: 1,
            },
        );
        drop(inner);
        self.not_empty.notify_one();
    }

    /// Worker path. Blocks until at least one event is present (or shutdown),
    /// then waits `debounce` for the storm to coalesce, then swaps out the whole
    /// buffer. Returns `(events_in_arrival_order, total_source_events)`.
    fn wait_and_drain(&self, config: &HookDispatcherConfig) -> Option<(Vec<MutationEvent>, u64)> {
        let mut inner = match self.queue.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        while inner.is_empty() {
            if self.shutdown.load(Ordering::SeqCst) {
                return None;
            }
            let (guard, _timeout) = match self.not_empty.wait_timeout(inner, config.idle_poll) {
                Ok(result) => result,
                Err(poisoned) => {
                    let (guard, timeout) = poisoned.into_inner();
                    (guard, timeout)
                }
            };
            inner = guard;
        }

        // Release the buffer lock during the debounce window so producers keep
        // coalescing into it; re-lock and swap afterward.
        drop(inner);
        if !config.debounce.is_zero() {
            thread::sleep(config.debounce);
        }

        let mut inner = match self.queue.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let order = std::mem::take(&mut inner.order);
        let mut by_key = std::mem::take(&mut inner.by_key);
        drop(inner);

        let mut events = Vec::with_capacity(order.len());
        let mut total = 0u64;
        for key in order {
            if let Some(queued) = by_key.remove(&key) {
                total += queued.coalesced_count;
                events.push(queued.event);
            }
        }
        Some((events, total))
    }

    fn mark_processed(&self, n: u64) {
        if n > 0 {
            self.processed.fetch_add(n, Ordering::SeqCst);
        }
        self.signal_progress();
    }

    fn signal_progress(&self) {
        let _guard = self
            .progress_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.progress.notify_all();
    }

    /// Block until every accepted event has been dispatched/dropped or the
    /// timeout elapses. Returns true when the queue fully drained.
    fn quiesce(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut guard = self
            .progress_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if self.processed.load(Ordering::SeqCst) >= self.sent.load(Ordering::SeqCst) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return self.processed.load(Ordering::SeqCst) >= self.sent.load(Ordering::SeqCst);
            }
            let remaining = deadline - now;
            let (next, _timeout) = self
                .progress
                .wait_timeout(guard, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard = next;
        }
    }
}

/// Producer handle the store holds. Cloneable and cheap; emitting is a brief
/// lock + notify with no handler involvement, so writes never block on hooks.
#[derive(Clone)]
pub struct HookEmitter {
    shared: Arc<HookShared>,
}

impl HookEmitter {
    /// Enqueue a post-commit mutation event. Always returns immediately.
    pub fn try_emit(&self, event: MutationEvent) {
        self.shared.emit(event);
    }
}

impl fmt::Debug for HookEmitter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookEmitter")
            .field("sent", &self.shared.sent.load(Ordering::SeqCst))
            .field("processed", &self.shared.processed.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

/// Point-in-time dispatcher counters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HookDispatcherStats {
    pub sent: u64,
    pub processed: u64,
    pub dropped: u64,
}

/// The dispatcher: owns the worker thread, hands out an [`HookEmitter`] for the
/// store, and tears the worker down on drop.
pub struct HookDispatcher {
    shared: Arc<HookShared>,
    worker: Option<JoinHandle<()>>,
}

impl HookDispatcher {
    /// Start the worker. `store` is any handle that can lend `&mut
    /// RedCoreGraphStore` (typically `Arc<Mutex<RedCoreGraphStore>>`, shared
    /// with the embedder's writer). Registrations are usually collected from
    /// the plugin registry via [`crate::plugin::PluginRegistry::hooks`].
    pub fn start<S: HookStoreAccess + 'static>(
        store: S,
        registrations: Vec<HookRegistration>,
        config: HookDispatcherConfig,
    ) -> Self {
        let shared = Arc::new(HookShared::new(config.queue_capacity));
        let store: Arc<dyn HookStoreAccess> = Arc::new(store);
        let worker_shared = Arc::clone(&shared);
        let registrations = Arc::new(registrations);

        let worker = thread::Builder::new()
            .name("rustyred-hook-dispatcher".to_string())
            .spawn(move || {
                run_worker(worker_shared, store, registrations, config);
            })
            .expect("spawn rustyred-hook-dispatcher worker");

        Self {
            shared,
            worker: Some(worker),
        }
    }

    /// A producer handle to install on the store via
    /// [`RedCoreGraphStore::attach_hook_emitter`].
    pub fn emitter(&self) -> HookEmitter {
        HookEmitter {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Block until the queue fully drains or the timeout elapses. Primarily for
    /// tests and deterministic shutdown; returns true on full drain.
    pub fn quiesce(&self, timeout: Duration) -> bool {
        self.shared.quiesce(timeout)
    }

    pub fn stats(&self) -> HookDispatcherStats {
        HookDispatcherStats {
            sent: self.shared.sent.load(Ordering::SeqCst),
            processed: self.shared.processed.load(Ordering::SeqCst),
            dropped: self.shared.dropped.load(Ordering::SeqCst),
        }
    }
}

impl Drop for HookDispatcher {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        self.shared.not_empty.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl fmt::Debug for HookDispatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookDispatcher")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

fn run_worker(
    shared: Arc<HookShared>,
    store: Arc<dyn HookStoreAccess>,
    registrations: Arc<Vec<HookRegistration>>,
    config: HookDispatcherConfig,
) {
    loop {
        let Some((events, total)) = shared.wait_and_drain(&config) else {
            return; // shutdown with an empty queue
        };
        dispatch_batch(store.as_ref(), &registrations, &config, events);
        shared.mark_processed(total);
    }
}

/// Group `events` by `(registration, tenant, coalesce_key)`, then run each group
/// once under a single store lock. Over-depth events are dropped (counted as
/// processed via the batch total). Handler errors and panics are isolated.
fn dispatch_batch(
    store: &dyn HookStoreAccess,
    registrations: &[HookRegistration],
    config: &HookDispatcherConfig,
    events: Vec<MutationEvent>,
) {
    // Group key: (registration index, tenant, coalesce key). Tenant is folded
    // in so two tenants sharing a coalesce slug (e.g. a repo id) never merge.
    type GroupKey = (usize, String, String);
    let mut groups: HashMap<GroupKey, Vec<MutationEvent>> = HashMap::new();
    let mut order: Vec<GroupKey> = Vec::new();

    for event in &events {
        if event.depth >= config.max_depth {
            // Loop guard: a write this deep in a hook chain stops here.
            continue;
        }
        for (idx, reg) in registrations.iter().enumerate() {
            if !reg.on.matches(event) {
                continue;
            }
            let coalesce = (reg.coalesce_key)(event).unwrap_or_else(|| event.id.clone());
            let key = (idx, event.tenant.clone(), coalesce);
            groups
                .entry(key.clone())
                .or_insert_with(|| {
                    order.push(key.clone());
                    Vec::new()
                })
                .push(event.clone());
        }
    }

    if groups.is_empty() {
        return;
    }

    let ran = store.with_store_mut(&mut |store| {
        for key in &order {
            let Some(group) = groups.get(key) else {
                continue;
            };
            let reg = &registrations[key.0];
            let tenant = key.1.clone();
            // Generation of this dispatch: one deeper than the deepest event it
            // reacts to. The store stamps the handler's writes with this depth.
            let run_depth = group.iter().map(|e| e.depth).max().unwrap_or(0) + 1;

            store.set_hook_emit_depth(run_depth);
            let result = {
                let mut ctx = HookContext {
                    store,
                    tenant: &tenant,
                    depth: run_depth,
                };
                // catch_unwind: a panicking handler must not poison the store
                // mutex (it is held by `with_store_mut` right now). AssertUnwindSafe
                // is sound because store writes are transactional — a panic leaves
                // the store at its last committed batch.
                catch_unwind(AssertUnwindSafe(|| (reg.handler)(&mut ctx, group)))
            };
            // Always clear the emit depth before the next group or lock release,
            // even if the handler errored or panicked.
            store.set_hook_emit_depth(0);

            match result {
                Ok(Ok(_outcome)) => {}
                Ok(Err(err)) => {
                    eprintln!("[rustyred-hook] handler '{}' error: {err}", reg.name);
                }
                Err(_panic) => {
                    eprintln!("[rustyred-hook] handler '{}' panicked; skipped", reg.name);
                }
            }
        }
    });

    if !ran {
        eprintln!(
            "[rustyred-hook] store unavailable (poisoned/gone); skipped {} hook group(s)",
            order.len()
        );
    }
}

/// Merge an incoming event into a coalesced entry: union changed props + labels,
/// take the latest commit timestamp and the deepest generation.
fn merge_event(existing: &mut MutationEvent, incoming: MutationEvent) {
    let mut props: BTreeSet<String> = existing.changed_props.drain(..).collect();
    props.extend(incoming.changed_props);
    existing.changed_props = props.into_iter().collect();

    if existing.labels != incoming.labels {
        let mut labels: BTreeSet<String> = existing.labels.drain(..).collect();
        labels.extend(incoming.labels);
        existing.labels = labels.into_iter().collect();
    }

    existing.committed_at_ms = existing.committed_at_ms.max(incoming.committed_at_ms);
    existing.depth = existing.depth.max(incoming.depth);
}

/// Pick the lowest-value queued identity to evict on a full buffer: lowest
/// mutation-kind rank, then oldest by arrival order.
fn lowest_value_key(inner: &QueueInner) -> Option<(MutationKind, String)> {
    // `order` is arrival order; the first entry at the minimum rank is the
    // oldest-lowest-value, which is what we shed.
    let mut best: Option<(u8, &(MutationKind, String))> = None;
    for key in &inner.order {
        if !inner.by_key.contains_key(key) {
            continue;
        }
        let rank = key.0.drop_rank();
        match best {
            Some((best_rank, _)) if rank >= best_rank => {}
            _ => best = Some((rank, key)),
        }
    }
    best.map(|(_, key)| key.clone())
}

/// Build the per-(kind,id) change summary the store emits. Diffs property keys
/// between the prior and next record so `changed_props` carries exactly the keys
/// that moved on this commit. A brand-new record reports all of its keys.
pub(crate) fn changed_property_keys(prior: Option<&Value>, next: &Value) -> Vec<String> {
    let empty = serde_json::Map::new();
    let prior_obj = prior.and_then(Value::as_object).unwrap_or(&empty);
    let next_obj = next.as_object().unwrap_or(&empty);

    let mut changed: BTreeMap<String, ()> = BTreeMap::new();
    for (key, value) in next_obj {
        if prior_obj.get(key) != Some(value) {
            changed.insert(key.clone(), ());
        }
    }
    for key in prior_obj.keys() {
        if !next_obj.contains_key(key) {
            changed.insert(key.clone(), ());
        }
    }
    changed.into_keys().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(
        kind: MutationKind,
        id: &str,
        labels: &[&str],
        props: &[&str],
        depth: u32,
    ) -> MutationEvent {
        MutationEvent::new(
            kind,
            "tenant-a",
            id,
            labels.iter().map(|s| s.to_string()).collect(),
            props.iter().map(|s| s.to_string()).collect(),
            0,
            depth,
        )
    }

    #[test]
    fn matcher_constraints_and_together() {
        let m = MutationMatcher::any()
            .with_kinds([MutationKind::NodeUpserted])
            .with_labels(["CodeSymbol"])
            .with_changed_props_any(["signature"]);
        assert!(m.matches(&ev(
            MutationKind::NodeUpserted,
            "n1",
            &["CodeSymbol"],
            &["signature"],
            0
        )));
        // wrong kind
        assert!(!m.matches(&ev(
            MutationKind::NodeDeleted,
            "n1",
            &["CodeSymbol"],
            &["signature"],
            0
        )));
        // wrong label
        assert!(!m.matches(&ev(
            MutationKind::NodeUpserted,
            "n1",
            &["CodeFile"],
            &["signature"],
            0
        )));
        // prop not in changed set
        assert!(!m.matches(&ev(
            MutationKind::NodeUpserted,
            "n1",
            &["CodeSymbol"],
            &["doc"],
            0
        )));
    }

    #[test]
    fn matcher_edge_type_matches_via_labels() {
        let m = MutationMatcher::any()
            .with_kinds([MutationKind::EdgeUpserted])
            .with_labels(["links_to"]);
        assert!(m.matches(&ev(MutationKind::EdgeUpserted, "e1", &["links_to"], &[], 0)));
        assert!(!m.matches(&ev(
            MutationKind::EdgeUpserted,
            "e1",
            &["CALLS_SYMBOL"],
            &[],
            0
        )));
    }

    #[test]
    fn changed_property_keys_reports_added_changed_removed() {
        let prior = serde_json::json!({"a": 1, "b": 2, "c": 3});
        let next = serde_json::json!({"a": 1, "b": 99, "d": 4});
        let keys = changed_property_keys(Some(&prior), &next);
        // b changed, d added, c removed; a unchanged.
        assert_eq!(
            keys,
            vec!["b".to_string(), "c".to_string(), "d".to_string()]
        );
    }

    #[test]
    fn changed_property_keys_new_record_reports_all() {
        let next = serde_json::json!({"signature": "fn f()", "doc": "x"});
        let keys = changed_property_keys(None, &next);
        assert_eq!(keys, vec!["doc".to_string(), "signature".to_string()]);
    }

    #[test]
    fn merge_event_unions_props_and_takes_max_depth() {
        let mut a = ev(
            MutationKind::NodeUpserted,
            "n1",
            &["CodeSymbol"],
            &["signature"],
            0,
        );
        a.committed_at_ms = 10;
        let mut b = ev(
            MutationKind::NodeUpserted,
            "n1",
            &["CodeSymbol"],
            &["doc"],
            1,
        );
        b.committed_at_ms = 20;
        merge_event(&mut a, b);
        assert_eq!(
            a.changed_props,
            vec!["doc".to_string(), "signature".to_string()]
        );
        assert_eq!(a.depth, 1);
        assert_eq!(a.committed_at_ms, 20);
    }
}
