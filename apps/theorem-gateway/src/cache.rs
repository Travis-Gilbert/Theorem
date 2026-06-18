//! Per-IP rate limiting and recomputable response caching.
//!
//! The gateway stores nothing durable. The only state it keeps is accelerator
//! state: rate-limit counters (so a single browser cannot hammer the
//! model/ingest paths) and an optional cache of recomputable read responses.
//! Both are backed by Valkey when `VALKEY_URL` is set and by in-process memory
//! otherwise — matching the theorem-grpc cache-aside convention.
//!
//! The rate limiter is a token bucket: `burst` capacity, refilled at
//! `per_minute` tokens/min. In-memory it is a `Mutex`-guarded map; on Valkey it
//! is an atomic Lua script (so the bucket is correct even across replicas).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use redis::aio::MultiplexedConnection;

const RATE_LIMIT_PREFIX: &str = "theorem-gateway:rl:";
const CACHE_PREFIX: &str = "theorem-gateway:cache:";

/// Atomic token-bucket refill+consume. KEYS[1] = bucket key. ARGV = capacity,
/// refill_per_sec, now_ms, ttl_sec. Returns 1 (allowed) or 0 (denied).
const TOKEN_BUCKET_LUA: &str = r#"
local key = KEYS[1]
local capacity = tonumber(ARGV[1])
local refill_per_sec = tonumber(ARGV[2])
local now_ms = tonumber(ARGV[3])
local ttl = tonumber(ARGV[4])
local data = redis.call('HMGET', key, 'tokens', 'ts')
local tokens = tonumber(data[1])
local ts = tonumber(data[2])
if tokens == nil then
  tokens = capacity
  ts = now_ms
end
local elapsed = math.max(0, now_ms - ts) / 1000.0
tokens = math.min(capacity, tokens + elapsed * refill_per_sec)
local allowed = 0
if tokens >= 1.0 then
  tokens = tokens - 1.0
  allowed = 1
end
redis.call('HMSET', key, 'tokens', tokens, 'ts', now_ms)
redis.call('EXPIRE', key, ttl)
return allowed
"#;

struct InMemoryBucket {
    tokens: f64,
    last_refill: Instant,
}

enum LimiterBackend {
    InMemory(Mutex<HashMap<String, InMemoryBucket>>),
    Valkey(MultiplexedConnection),
}

/// Per-IP token-bucket rate limiter for the side-effecting / model resolvers.
pub struct RateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    ttl: Duration,
    backend: LimiterBackend,
}

impl RateLimiter {
    pub fn new(burst: u32, per_minute: u32, valkey: Option<MultiplexedConnection>) -> Self {
        let capacity = burst.max(1) as f64;
        let refill_per_sec = (per_minute.max(1) as f64) / 60.0;
        // TTL long enough for a fully drained bucket to refill, so idle keys
        // expire instead of accumulating forever.
        let ttl_secs = ((capacity / refill_per_sec).ceil() as u64).max(60);
        let backend = match valkey {
            Some(conn) => LimiterBackend::Valkey(conn),
            None => LimiterBackend::InMemory(Mutex::new(HashMap::new())),
        };
        Self {
            capacity,
            refill_per_sec,
            ttl: Duration::from_secs(ttl_secs),
            backend,
        }
    }

    /// Returns `true` if a token was available for `key` (the client IP) and was
    /// consumed; `false` if the bucket is empty (request should be refused).
    /// Fails open: if Valkey errors, the request is allowed (availability over a
    /// hard denial on infra failure) and the error is logged.
    pub async fn check(&self, key: &str) -> bool {
        match &self.backend {
            LimiterBackend::InMemory(map) => self.check_in_memory(map, key),
            LimiterBackend::Valkey(conn) => self.check_valkey(conn.clone(), key).await,
        }
    }

    fn check_in_memory(&self, map: &Mutex<HashMap<String, InMemoryBucket>>, key: &str) -> bool {
        let now = Instant::now();
        let mut guard = match map.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let bucket = guard.entry(key.to_string()).or_insert(InMemoryBucket {
            tokens: self.capacity,
            last_refill: now,
        });
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        bucket.last_refill = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    async fn check_valkey(&self, mut conn: MultiplexedConnection, key: &str) -> bool {
        // A monotonic-ish millisecond clock for the refill math. SystemTime is
        // fine here: the bucket only needs elapsed deltas, not wall accuracy.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let full_key = format!("{RATE_LIMIT_PREFIX}{key}");
        let script = redis::Script::new(TOKEN_BUCKET_LUA);
        let result: redis::RedisResult<i64> = script
            .key(full_key)
            .arg(self.capacity)
            .arg(self.refill_per_sec)
            .arg(now_ms)
            .arg(self.ttl.as_secs())
            .invoke_async(&mut conn)
            .await;
        match result {
            Ok(allowed) => allowed == 1,
            Err(error) => {
                tracing::warn!("GATEWAY_RATELIMIT_VALKEY_ERROR fail-open: {error}");
                true
            }
        }
    }
}

/// Optional cache for recomputable read responses (search, gapWalk, searchCode).
/// `None` when `VALKEY_URL` is unset: the gateway simply recomputes every time.
#[derive(Clone)]
pub struct ResponseCache {
    conn: Option<MultiplexedConnection>,
    ttl: Duration,
}

impl ResponseCache {
    pub fn new(conn: Option<MultiplexedConnection>, ttl: Duration) -> Self {
        Self { conn, ttl }
    }

    pub fn enabled(&self) -> bool {
        self.conn.is_some()
    }

    /// Fetch a cached JSON payload for `key`, if present and cache is enabled.
    pub async fn get(&self, key: &str) -> Option<String> {
        let mut conn = self.conn.clone()?;
        let full_key = format!("{CACHE_PREFIX}{key}");
        match redis::cmd("GET")
            .arg(&full_key)
            .query_async::<Option<String>>(&mut conn)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!("GATEWAY_CACHE_GET_ERROR {error}");
                None
            }
        }
    }

    /// Store a JSON payload for `key` with the configured TTL. Best-effort: a
    /// cache write failure never fails the request.
    pub async fn set(&self, key: &str, value: &str) {
        let Some(mut conn) = self.conn.clone() else {
            return;
        };
        let full_key = format!("{CACHE_PREFIX}{key}");
        let outcome: redis::RedisResult<()> = redis::cmd("SET")
            .arg(&full_key)
            .arg(value)
            .arg("EX")
            .arg(self.ttl.as_secs().max(1))
            .query_async(&mut conn)
            .await;
        if let Err(error) = outcome {
            tracing::warn!("GATEWAY_CACHE_SET_ERROR {error}");
        }
    }
}

/// Build a multiplexed async Valkey connection from a URL. Returns `None` (and
/// logs) on any failure so the gateway always boots — Valkey is an accelerator,
/// never a hard dependency.
pub async fn connect_valkey(url: &str) -> Option<MultiplexedConnection> {
    let client = match redis::Client::open(url) {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!("GATEWAY_VALKEY_OPEN_FAILED {error}");
            return None;
        }
    };
    match client.get_multiplexed_async_connection().await {
        Ok(conn) => {
            tracing::info!("GATEWAY_VALKEY_READY");
            Some(conn)
        }
        Err(error) => {
            tracing::warn!("GATEWAY_VALKEY_UNREACHABLE fall back to in-memory: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_bucket_allows_burst_then_denies() {
        // burst = 3, refill 1/min => first 3 pass, 4th denied within the window.
        let limiter = RateLimiter::new(3, 1, None);
        assert!(limiter.check("1.2.3.4").await);
        assert!(limiter.check("1.2.3.4").await);
        assert!(limiter.check("1.2.3.4").await);
        assert!(!limiter.check("1.2.3.4").await);
    }

    #[tokio::test]
    async fn buckets_are_per_key() {
        let limiter = RateLimiter::new(1, 1, None);
        assert!(limiter.check("a").await);
        assert!(!limiter.check("a").await);
        // A different IP has its own full bucket.
        assert!(limiter.check("b").await);
    }

    #[tokio::test]
    async fn disabled_cache_reports_disabled() {
        let cache = ResponseCache::new(None, Duration::from_secs(60));
        assert!(!cache.enabled());
        assert_eq!(cache.get("anything").await, None);
        // set is a no-op and must not panic.
        cache.set("anything", "value").await;
    }
}
