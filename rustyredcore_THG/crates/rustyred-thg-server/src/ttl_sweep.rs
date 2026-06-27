//! TTL background sweep loop (TTL-04).
//!
//! Periodically iterates every materialized RedCore tenant and calls
//! `purge_expired_nodes()` on each. Each per-tenant purge writes a
//! `NodeDelete` AOF op for every expired node, so the cleanup is
//! durable across process restarts.
//!
//! Tuning knobs:
//!   * sweep interval: env var `RUSTYRED_THG_TTL_SWEEP_MS`, default 1000ms.
//!     Lower = tighter expiration precision but more CPU. Higher = lower
//!     overhead but expired nodes can linger up to one interval past
//!     their TTL before being purged (reads are filtered immediately
//!     either way; this is a reclamation-precision knob only).
//!
//! Lifecycle: the loop runs until `TtlSweepState::shutdown()` is
//! called, then drains its current tick and exits. main.rs wires
//! SIGTERM/SIGINT to shutdown via tokio::signal.
//!
//! Observability counters (exposed at /v1/diagnostics/config):
//!   * `ttl_active_count`: total TTL-bearing live nodes across all tenants.
//!   * `swept_total`: cumulative count of nodes purged since start.
//!   * `last_sweep_at_ms`: Unix milliseconds of the last completed tick.
//!   * `sweep_duration_p99_ms`: p99 of the last 100 tick durations.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::time::{interval, MissedTickBehavior};

use crate::state::AppState;

/// Cap on retained sweep durations. 100 ticks at 1000ms = 100s of
/// history, which is plenty for a p99 estimate without unbounded memory.
const SWEEP_DURATION_RING_CAP: usize = 100;

/// Observability + lifecycle state for the TTL sweep loop. Cloned into
/// the spawned task; main.rs holds the original to wire shutdown.
#[derive(Debug, Default)]
pub struct TtlSweepState {
    /// Cumulative nodes purged across all tenants since process start.
    swept_total: AtomicU64,
    /// Unix milliseconds of the last completed sweep tick (0 = never run).
    last_sweep_at_ms: AtomicU64,
    /// Ring buffer of per-tick durations in milliseconds. Capped at
    /// SWEEP_DURATION_RING_CAP entries. Mutex because writes happen
    /// from the sweep task while reads happen from the diagnostics
    /// HTTP handler.
    sweep_durations_ms: Mutex<VecDeque<u64>>,
    /// Set to true by `shutdown()`. The sweep loop checks this at the
    /// top of every tick and self-exits if set.
    cancel: AtomicBool,
}

impl TtlSweepState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Signal the sweep loop to stop. Idempotent. The loop drains its
    /// current tick before exiting, so callers don't need to await
    /// completion explicitly for correctness — but main.rs joins the
    /// JoinHandle anyway so the process doesn't exit mid-AOF write.
    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn swept_total(&self) -> u64 {
        self.swept_total.load(Ordering::Relaxed)
    }

    pub fn last_sweep_at_ms(&self) -> u64 {
        self.last_sweep_at_ms.load(Ordering::Relaxed)
    }

    /// p99 of the last 100 tick durations in milliseconds. Returns 0
    /// if no ticks have run yet.
    pub fn sweep_duration_p99_ms(&self) -> u64 {
        let Ok(durations) = self.sweep_durations_ms.lock() else {
            // Lock poisoned: treat as "no data" rather than panicking
            // in a diagnostics handler.
            return 0;
        };
        if durations.is_empty() {
            return 0;
        }
        let mut sorted: Vec<u64> = durations.iter().copied().collect();
        sorted.sort_unstable();
        // p99 index for N samples: ceil(0.99 * N) - 1, clamped to last
        // element. For small N this collapses to the max, which is
        // the conservative "tail" answer.
        let n = sorted.len();
        let idx = ((n as f64 * 0.99).ceil() as usize)
            .saturating_sub(1)
            .min(n - 1);
        sorted[idx]
    }

    fn record_tick(&self, purged: u64, duration_ms: u64) {
        self.swept_total.fetch_add(purged, Ordering::Relaxed);
        self.last_sweep_at_ms.store(now_ms_u64(), Ordering::Relaxed);
        if let Ok(mut durations) = self.sweep_durations_ms.lock() {
            if durations.len() == SWEEP_DURATION_RING_CAP {
                durations.pop_front();
            }
            durations.push_back(duration_ms);
        }
    }

    fn should_stop(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Spawn the TTL sweep loop. Returns a JoinHandle so the caller can
/// await clean shutdown. The loop runs forever until
/// `state.ttl_sweep.shutdown()` is called.
pub fn spawn_sweep_loop(state: AppState, interval_ms: u64) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        run_sweep_loop(state, interval_ms).await;
    })
}

/// The sweep loop body. Public so tokio::test cases can drive it
/// directly without going through tokio::spawn.
pub async fn run_sweep_loop(state: AppState, interval_ms: u64) {
    let interval_ms = interval_ms.max(1);
    // Skew the first tick to interval_ms so we don't immediately sweep
    // an empty store at startup. `MissedTickBehavior::Delay` means if
    // a tick gets behind (e.g., a tenant purge took longer than the
    // interval), the next tick resets relative to that completion
    // rather than firing in a burst to "catch up" — important because
    // catching up would mean every backlogged tick re-locks the same
    // tenant writers in quick succession.
    let mut ticker = interval(Duration::from_millis(interval_ms));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // Burn the immediate first tick that `interval()` always fires.
    ticker.tick().await;

    loop {
        ticker.tick().await;
        if state.ttl_sweep.should_stop() {
            tracing::info!("ttl_sweep loop received shutdown signal, exiting");
            break;
        }
        let start = Instant::now();
        let sweep_state = state.clone();
        let purged =
            match tokio::task::spawn_blocking(move || sweep_all_tenants(&sweep_state)).await {
                Ok(purged) => purged,
                Err(err) => {
                    tracing::warn!(error = %err, "ttl_sweep blocking task failed");
                    0
                }
            };
        let duration_ms = start.elapsed().as_millis() as u64;
        state.ttl_sweep.record_tick(purged, duration_ms);
        if purged > 0 {
            tracing::debug!(
                purged,
                duration_ms,
                swept_total = state.ttl_sweep.swept_total(),
                "ttl_sweep tick complete"
            );
        }
    }
}

/// Iterate every materialized tenant, call purge_expired_nodes on each.
/// Errors are logged but DO NOT stop the loop — one tenant's IO error
/// must not block sweep for other tenants. Returns the total count
/// purged across all tenants this tick.
fn sweep_all_tenants(state: &AppState) -> u64 {
    let tenants = match state.iter_redcore_tenants() {
        Ok(tenants) => tenants,
        Err(err) => {
            tracing::error!(error = %err.message, "ttl_sweep: tenant enumeration failed");
            return 0;
        }
    };
    let mut total: u64 = 0;
    for (tenant_id, executor) in tenants {
        match executor.purge_expired_nodes() {
            Ok(purged) => {
                total = total.saturating_add(purged as u64);
            }
            Err(err) => {
                // Per-tenant error: skip and continue. Most likely
                // cause is AOF disk write failure (full disk, permission
                // change). Operators will see the structured log.
                tracing::warn!(
                    tenant = %tenant_id,
                    error = %err.message,
                    code = %err.code,
                    "ttl_sweep: purge failed for tenant"
                );
            }
        }
    }
    total
}

fn now_ms_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, StorageMode};
    use rustyred_thg_core::{NodeRecord, TTL_PROPERTY};
    use serde_json::json;

    fn test_config() -> Config {
        let mut config = Config::default_for_tests();
        config.storage_mode = StorageMode::Memory;
        config
    }

    #[tokio::test]
    async fn sweep_loop_purges_expired_node_within_one_tick() {
        let config = test_config();
        let state = AppState::new(config);

        // Write a node with a TTL already in the past, then materialize
        // the tenant via a normal write path so it lands in
        // redcore_stores (the lazy-cache the sweep iterates).
        let executor = match state
            .tenant_graph_store("test")
            .expect("tenant store available")
        {
            crate::state::TenantGraphStore::RedCore(executor) => executor,
            _ => panic!("expected RedCore tenant store; saw a non-RedCore variant (legacy Redis mode is unsupported for TTL by design)"),
        };
        let past_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            - 1_000;
        executor
            .upsert_node(NodeRecord::new(
                "node:expired",
                ["MemoryAtom"],
                json!({ "title": "stale", TTL_PROPERTY: past_ms }),
            ))
            .unwrap();
        assert!(
            executor.get_node("node:expired").unwrap().is_none(),
            "expired node should be filtered from get_node BEFORE sweep too (read filter)"
        );
        assert_eq!(
            executor.ttl_active_count().unwrap(),
            1,
            "expired node still in ttl_index until swept"
        );

        // Run sweep on a tight 50ms interval so the test finishes
        // quickly. interval() skips its immediate first fire because
        // run_sweep_loop calls .tick() once at startup before entering
        // the loop body.
        let state_clone = state.clone();
        let handle = spawn_sweep_loop(state_clone, 50);

        // Let ~3 ticks elapse so we're robust to scheduler jitter.
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Tell the loop to stop, then await it to confirm clean exit.
        state.ttl_sweep.shutdown();
        // Give the loop one more tick to observe the cancel flag.
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();

        assert_eq!(
            executor.ttl_active_count().unwrap(),
            0,
            "ttl_index should be empty after sweep purged the expired node"
        );
        assert!(
            state.ttl_sweep.swept_total() >= 1,
            "swept_total counter should reflect at least one purge"
        );
        assert!(
            state.ttl_sweep.last_sweep_at_ms() > 0,
            "last_sweep_at_ms should advance after first tick"
        );
    }

    #[tokio::test]
    async fn sweep_loop_exits_cleanly_when_shutdown_signaled() {
        let config = test_config();
        let state = AppState::new(config);
        let handle = spawn_sweep_loop(state.clone(), 50);
        // Give it time to enter the loop.
        tokio::time::sleep(Duration::from_millis(100)).await;
        state.ttl_sweep.shutdown();
        // Wait long enough for the loop to observe the flag at the
        // next tick boundary, then await the handle. If the loop
        // doesn't honor cancel, this hangs and the test times out.
        let exit_result = tokio::time::timeout(Duration::from_millis(500), handle).await;
        assert!(
            exit_result.is_ok(),
            "sweep loop should exit within 500ms of shutdown signal"
        );
    }

    #[tokio::test]
    async fn sweep_loop_records_zero_purged_when_nothing_expired() {
        let config = test_config();
        let state = AppState::new(config);
        // Materialize a tenant with a single live (future-TTL) node so
        // there's something to iterate but nothing to purge.
        let executor = match state.tenant_graph_store("test").unwrap() {
            crate::state::TenantGraphStore::RedCore(executor) => executor,
            _ => panic!("expected RedCore tenant store"),
        };
        let future_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            + 60_000;
        executor
            .upsert_node(NodeRecord::new(
                "node:fresh",
                ["MemoryAtom"],
                json!({ "title": "live", TTL_PROPERTY: future_ms }),
            ))
            .unwrap();
        let handle = spawn_sweep_loop(state.clone(), 50);
        tokio::time::sleep(Duration::from_millis(200)).await;
        state.ttl_sweep.shutdown();
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();

        // Counters should show the loop ran (last_sweep_at_ms > 0) but
        // purged nothing.
        assert_eq!(state.ttl_sweep.swept_total(), 0);
        assert!(state.ttl_sweep.last_sweep_at_ms() > 0);
        assert!(executor.get_node("node:fresh").unwrap().is_some());
    }
}
