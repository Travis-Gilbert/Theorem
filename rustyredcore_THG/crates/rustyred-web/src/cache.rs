//! Valkey/Redis cache-aside accelerator (Spec 3, valkey-deploy-handoff).
//!
//! A pure accelerator and availability cushion -- never a source of truth.
//! Every cached value MUST be recomputable from RustyRed, and the API shape
//! enforces it: [`Cache::cache_aside`] always takes a `compute` closure, so a
//! miss, a backend outage, or a corrupt entry all fall through to recompute.
//! Two consequences fall out for free:
//!   * the cache can never return a wrong answer (a poisoned/incompatible entry
//!     is treated as a miss), and
//!   * a Valkey blip degrades to "recompute from RustyRed", not a failed
//!     request -- the availability cushion the handoff calls the main win.
//!
//! Roles (handoff): cache PPR results keyed by `(seed-set hash, params)`,
//! `context_pack` output, embedding lookups, and -- per the rankers/rerankers
//! extension -- reranked orderings keyed by `(query, candidate-set, model,
//! params)` with the model version in the key so a model bump invalidates.
//!
//! The live Valkey backend is behind the `valkey-cache` feature (pulls
//! `redis`); the in-memory backend, key schemes, stats, and the cache-aside
//! logic compile and test without it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Advisory backend error. The cache swallows these and recomputes, so a
/// request never fails because the cache is down.
#[derive(Debug, Clone)]
pub struct CacheError(pub String);

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cache backend error: {}", self.0)
    }
}

impl std::error::Error for CacheError {}

/// A byte-oriented cache store. Implementations are accelerators only: callers
/// route through [`Cache`], which never lets a backend failure surface.
#[async_trait]
pub trait CacheBackend: Send + Sync {
    async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError>;
    async fn set_bytes(&self, key: &str, value: &[u8], ttl: Duration) -> Result<(), CacheError>;
    async fn ping(&self) -> Result<(), CacheError>;
}

/// Live hit/miss/error counters for cache observability (handoff acceptance:
/// "a cache-hit counter incrementing").
#[derive(Debug, Default)]
pub struct CacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    errors: AtomicU64,
}

impl CacheStats {
    fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }
    fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }
    fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }
    pub fn snapshot(&self) -> CacheStatsSnapshot {
        CacheStatsSnapshot {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStatsSnapshot {
    pub hits: u64,
    pub misses: u64,
    pub errors: u64,
}

impl CacheStatsSnapshot {
    /// Hit rate over hits + misses (errors excluded; they fell through to
    /// recompute but are not lookups against a populated cache). 0.0 when no
    /// lookups have happened.
    pub fn hit_rate(&self) -> f64 {
        let lookups = self.hits + self.misses;
        if lookups == 0 {
            0.0
        } else {
            self.hits as f64 / lookups as f64
        }
    }
}

/// Cache-aside accelerator over a [`CacheBackend`]. Clone-cheap (shared backend
/// + stats via `Arc`).
#[derive(Clone)]
pub struct Cache {
    backend: Arc<dyn CacheBackend>,
    stats: Arc<CacheStats>,
    enabled: bool,
}

impl Cache {
    /// Build a cache over a live backend.
    pub fn new(backend: Arc<dyn CacheBackend>) -> Self {
        Self {
            backend,
            stats: Arc::new(CacheStats::default()),
            enabled: true,
        }
    }

    /// A disabled cache: every [`Cache::cache_aside`] computes. This is the
    /// graceful no-op used when `VALKEY_URL` is unset, so callers never branch
    /// on whether caching is configured.
    pub fn disabled() -> Self {
        Self {
            backend: Arc::new(NoopBackend),
            stats: Arc::new(CacheStats::default()),
            enabled: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn stats(&self) -> CacheStatsSnapshot {
        self.stats.snapshot()
    }

    /// Round-trip the backend (handoff acceptance #1: a `PING` round-trips).
    pub async fn ping(&self) -> Result<(), CacheError> {
        self.backend.ping().await
    }

    /// Cache-aside: return the cached value for `key` if present and decodable,
    /// otherwise run `compute`, populate the cache best-effort, and return the
    /// computed value. The expensive `compute` is skipped only on a genuine hit;
    /// a backend error or a decode failure both recompute (and count an error),
    /// so the cache is strictly an accelerator.
    pub async fn cache_aside<T, F, Fut>(&self, key: &str, ttl: Duration, compute: F) -> T
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        if self.enabled {
            match self.backend.get_bytes(key).await {
                Ok(Some(bytes)) => match serde_json::from_slice::<T>(&bytes) {
                    Ok(value) => {
                        self.stats.record_hit();
                        return value;
                    }
                    // Poisoned / schema-incompatible entry: treat as a miss and
                    // recompute rather than ever returning a wrong answer.
                    Err(_) => self.stats.record_error(),
                },
                Ok(None) => self.stats.record_miss(),
                // Backend outage: the availability cushion -- recompute from
                // RustyRed instead of failing the request.
                Err(_) => self.stats.record_error(),
            }
        }

        let value = compute().await;

        if self.enabled {
            if let Ok(bytes) = serde_json::to_vec(&value) {
                // Best-effort populate; a set failure must not fail the request.
                let _ = self.backend.set_bytes(key, &bytes, ttl).await;
            }
        }
        value
    }

    /// Lower-level lookup for batch/partial cache-aside (e.g. embedding lookups
    /// where only the misses in a batch get recomputed, in one call). Returns
    /// `None` on miss, backend error, or decode failure -- all counted -- so the
    /// caller recomputes; a hit is counted.
    pub async fn get_cached<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        if !self.enabled {
            return None;
        }
        match self.backend.get_bytes(key).await {
            Ok(Some(bytes)) => match serde_json::from_slice(&bytes) {
                Ok(value) => {
                    self.stats.record_hit();
                    Some(value)
                }
                Err(_) => {
                    self.stats.record_error();
                    None
                }
            },
            Ok(None) => {
                self.stats.record_miss();
                None
            }
            Err(_) => {
                self.stats.record_error();
                None
            }
        }
    }

    /// Lower-level best-effort populate for batch/partial cache-aside. A set
    /// failure is swallowed (the cache is an accelerator).
    pub async fn put_cached<T: Serialize>(&self, key: &str, value: &T, ttl: Duration) {
        if !self.enabled {
            return;
        }
        if let Ok(bytes) = serde_json::to_vec(value) {
            let _ = self.backend.set_bytes(key, &bytes, ttl).await;
        }
    }
}

/// No-op backend backing a [`Cache::disabled`].
struct NoopBackend;

#[async_trait]
impl CacheBackend for NoopBackend {
    async fn get_bytes(&self, _key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        Ok(None)
    }
    async fn set_bytes(&self, _key: &str, _value: &[u8], _ttl: Duration) -> Result<(), CacheError> {
        Ok(())
    }
    async fn ping(&self) -> Result<(), CacheError> {
        Ok(())
    }
}

/// In-process cache backend with TTL. Useful as a test double and as a
/// single-process fallback. Honors expiry so freshness behaves like the live
/// backend.
#[derive(Default)]
pub struct InMemoryBackend {
    map: Mutex<HashMap<String, (Vec<u8>, Option<Instant>)>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CacheBackend for InMemoryBackend {
    async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        let mut map = self.map.lock().expect("in-memory cache poisoned");
        match map.get(key) {
            Some((_, Some(expiry))) if *expiry <= Instant::now() => {
                map.remove(key);
                Ok(None)
            }
            Some((bytes, _)) => Ok(Some(bytes.clone())),
            None => Ok(None),
        }
    }
    async fn set_bytes(&self, key: &str, value: &[u8], ttl: Duration) -> Result<(), CacheError> {
        let expiry = if ttl.is_zero() {
            None
        } else {
            Instant::now().checked_add(ttl)
        };
        self.map
            .lock()
            .expect("in-memory cache poisoned")
            .insert(key.to_string(), (value.to_vec(), expiry));
        Ok(())
    }
    async fn ping(&self) -> Result<(), CacheError> {
        Ok(())
    }
}

/// Namespaced, content-addressed cache keys for the handoff roles. Each scheme
/// blake3-hashes its variable inputs so keys stay short and collision-resistant,
/// and keeps the role + invalidation dimension (model version) in the clear so
/// keyspaces are inspectable and a model bump cleanly invalidates.
pub mod keys {
    fn digest(parts: &[&str]) -> String {
        let mut hasher = blake3::Hasher::new();
        for part in parts {
            hasher.update(part.as_bytes());
            hasher.update(&[0x1f]); // unit separator so ("a","b") != ("ab","")
        }
        hasher.finalize().to_hex()[..32].to_string()
    }

    /// PPR result keyed by the seed-set fingerprint and the params (alpha, topk,
    /// edge filters, ...). Recomputable from RustyRed PPR.
    pub fn ppr(seed_set: &str, params: &str) -> String {
        format!("ppr:{}", digest(&[seed_set, params]))
    }

    /// `context_pack` output keyed by the pack request fingerprint.
    pub fn context_pack(request: &str) -> String {
        format!("ctxpack:{}", digest(&[request]))
    }

    /// Embedding lookup keyed by model + text.
    pub fn embedding(model: &str, text: &str) -> String {
        format!("emb:{model}:{}", digest(&[text]))
    }

    /// Reranked ordering keyed by model VERSION + query + candidate-set
    /// fingerprint + params. The model version is in the clear so bumping the
    /// reranker invalidates every prior entry for it.
    pub fn rerank(model_version: &str, query: &str, candidate_set: &str, params: &str) -> String {
        format!(
            "rerank:{model_version}:{}",
            digest(&[query, candidate_set, params])
        )
    }
}

/// A [`crate::embedding::TextEmbedder`] wrapped with per-text cache-aside -- the
/// handoff's "embedding lookups" role. Each input text is cached individually
/// keyed by `(model_id, text)`, so only the cache misses in a batch reach the
/// inner embedder and a text repeated across batches is served from Valkey.
/// Order-preserving; transparent (implements the same trait), so it drops in
/// wherever a `TextEmbedder` is used. When the cache is disabled it is a pure
/// pass-through to the inner embedder.
pub struct CachedTextEmbedder<E: crate::embedding::TextEmbedder> {
    inner: E,
    cache: Cache,
    ttl: Duration,
}

impl<E: crate::embedding::TextEmbedder> CachedTextEmbedder<E> {
    pub fn new(inner: E, cache: Cache, ttl: Duration) -> Self {
        Self { inner, cache, ttl }
    }
}

impl<E: crate::embedding::TextEmbedder> crate::embedding::TextEmbedder for CachedTextEmbedder<E> {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn dimension(&self) -> usize {
        self.inner.dimension()
    }
    fn property(&self) -> &str {
        self.inner.property()
    }
    fn metric(&self) -> &str {
        self.inner.metric()
    }
    fn normalized(&self) -> bool {
        self.inner.normalized()
    }
    fn embed<'a>(
        &'a self,
        inputs: &'a [String],
    ) -> futures_util::future::BoxFuture<'a, Result<Vec<Vec<f32>>, crate::embedding::EmbeddingError>>
    {
        Box::pin(async move {
            let model_id = self.inner.model_id();
            let mut results: Vec<Option<Vec<f32>>> = vec![None; inputs.len()];
            let mut miss_indices: Vec<usize> = Vec::new();
            let mut miss_inputs: Vec<String> = Vec::new();

            for (idx, text) in inputs.iter().enumerate() {
                let key = keys::embedding(model_id, text);
                match self.cache.get_cached::<Vec<f32>>(&key).await {
                    Some(vector) => results[idx] = Some(vector),
                    None => {
                        miss_indices.push(idx);
                        miss_inputs.push(text.clone());
                    }
                }
            }

            if !miss_inputs.is_empty() {
                // One batched call for just the misses.
                let embedded = self.inner.embed(&miss_inputs).await?;
                for (offset, vector) in embedded.into_iter().enumerate() {
                    let idx = miss_indices[offset];
                    let key = keys::embedding(model_id, &inputs[idx]);
                    self.cache.put_cached(&key, &vector, self.ttl).await;
                    results[idx] = Some(vector);
                }
            }

            Ok(results
                .into_iter()
                .map(|slot| slot.unwrap_or_default())
                .collect())
        })
    }
}

#[cfg(feature = "valkey-cache")]
mod valkey {
    use super::{CacheBackend, CacheError};
    use async_trait::async_trait;
    use std::time::Duration;

    /// Live Valkey/Redis backend over the async multiplexed connection. Reads
    /// `VALKEY_URL`; wire-compatible with Valkey unchanged.
    pub struct ValkeyBackend {
        client: redis::Client,
    }

    impl ValkeyBackend {
        pub fn from_url(url: &str) -> Result<Self, CacheError> {
            redis::Client::open(url)
                .map(|client| Self { client })
                .map_err(|error| CacheError(error.to_string()))
        }

        async fn connection(&self) -> Result<redis::aio::MultiplexedConnection, CacheError> {
            self.client
                .get_multiplexed_async_connection()
                .await
                .map_err(|error| CacheError(error.to_string()))
        }
    }

    #[async_trait]
    impl CacheBackend for ValkeyBackend {
        async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
            use redis::AsyncCommands;
            let mut connection = self.connection().await?;
            connection
                .get(key)
                .await
                .map_err(|error| CacheError(error.to_string()))
        }
        async fn set_bytes(
            &self,
            key: &str,
            value: &[u8],
            ttl: Duration,
        ) -> Result<(), CacheError> {
            use redis::AsyncCommands;
            let mut connection = self.connection().await?;
            // SET key value EX <ttl>. A TTL floor of 1s keeps Valkey from
            // rejecting a zero expiry; zero-ttl values are not meant for Valkey.
            let seconds = ttl.as_secs().max(1);
            connection
                .set_ex::<_, _, ()>(key, value, seconds)
                .await
                .map_err(|error| CacheError(error.to_string()))
        }
        async fn ping(&self) -> Result<(), CacheError> {
            let mut connection = self.connection().await?;
            redis::cmd("PING")
                .query_async::<()>(&mut connection)
                .await
                .map_err(|error| CacheError(error.to_string()))
        }
    }
}

#[cfg(feature = "valkey-cache")]
pub use valkey::ValkeyBackend;

impl Cache {
    /// Build a cache from the `VALKEY_URL` environment variable. Returns a
    /// disabled cache (graceful no-op) when the var is unset/empty, when the
    /// `valkey-cache` feature is off, or when the client cannot be built -- so
    /// a missing or misconfigured cache never breaks the service, it just turns
    /// caching off.
    pub fn from_env() -> Self {
        #[cfg(feature = "valkey-cache")]
        {
            match std::env::var("VALKEY_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
            {
                Some(url) => match ValkeyBackend::from_url(&url) {
                    Ok(backend) => Cache::new(Arc::new(backend)),
                    Err(_) => Cache::disabled(),
                },
                None => Cache::disabled(),
            }
        }
        #[cfg(not(feature = "valkey-cache"))]
        {
            Cache::disabled()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend that always errors, to exercise the availability cushion.
    struct FailingBackend;

    #[async_trait]
    impl CacheBackend for FailingBackend {
        async fn get_bytes(&self, _key: &str) -> Result<Option<Vec<u8>>, CacheError> {
            Err(CacheError("down".into()))
        }
        async fn set_bytes(
            &self,
            _key: &str,
            _value: &[u8],
            _ttl: Duration,
        ) -> Result<(), CacheError> {
            Err(CacheError("down".into()))
        }
        async fn ping(&self) -> Result<(), CacheError> {
            Err(CacheError("down".into()))
        }
    }

    /// Acceptance #2: a repeated expensive read is served from cache on the
    /// second call, the expensive compute runs once, and the hit counter
    /// increments.
    #[tokio::test]
    async fn cache_aside_serves_second_call_and_counts_hit() {
        let cache = Cache::new(Arc::new(InMemoryBackend::new()));
        let key = keys::ppr("seed-set-1", "alpha=0.15;topk=10");
        let computes = Arc::new(AtomicU64::new(0));

        let counter = computes.clone();
        let first: Vec<u32> = cache
            .cache_aside(&key, Duration::from_secs(60), move || async move {
                counter.fetch_add(1, Ordering::Relaxed);
                vec![1, 2, 3]
            })
            .await;
        let counter = computes.clone();
        let second: Vec<u32> = cache
            .cache_aside(&key, Duration::from_secs(60), move || async move {
                counter.fetch_add(1, Ordering::Relaxed);
                vec![1, 2, 3]
            })
            .await;

        assert_eq!(first, vec![1, 2, 3]);
        assert_eq!(second, vec![1, 2, 3]);
        assert_eq!(
            computes.load(Ordering::Relaxed),
            1,
            "second call served from cache, expensive compute ran once"
        );
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.errors, 0);
        assert!((stats.hit_rate() - 0.5).abs() < f64::EPSILON);
    }

    /// Acceptance #4: a TTL expiry causes the next read to recompute (no value
    /// is stale forever).
    #[tokio::test]
    async fn ttl_expiry_recomputes() {
        let cache = Cache::new(Arc::new(InMemoryBackend::new()));
        let key = keys::context_pack("doc-1");
        let computes = Arc::new(AtomicU64::new(0));

        let counter = computes.clone();
        let _: u64 = cache
            .cache_aside(&key, Duration::from_millis(20), move || async move {
                counter.fetch_add(1, Ordering::Relaxed);
                7
            })
            .await;
        tokio::time::sleep(Duration::from_millis(45)).await;
        let counter = computes.clone();
        let _: u64 = cache
            .cache_aside(&key, Duration::from_millis(20), move || async move {
                counter.fetch_add(1, Ordering::Relaxed);
                7
            })
            .await;

        assert_eq!(
            computes.load(Ordering::Relaxed),
            2,
            "expired entry recomputes"
        );
    }

    /// Acceptance #3 (mechanism): a backend outage falls through to recompute --
    /// the request still succeeds and an error is counted.
    #[tokio::test]
    async fn backend_outage_falls_through_to_compute() {
        let cache = Cache::new(Arc::new(FailingBackend));
        let value: u32 = cache
            .cache_aside("k", Duration::from_secs(60), || async { 42 })
            .await;
        assert_eq!(value, 42, "request survives a cache outage");
        let stats = cache.stats();
        assert_eq!(stats.errors, 1);
        assert_eq!(stats.hits, 0);
    }

    /// A disabled cache always computes (the graceful VALKEY_URL-unset path).
    #[tokio::test]
    async fn disabled_cache_always_computes() {
        let cache = Cache::disabled();
        assert!(!cache.is_enabled());
        let computes = Arc::new(AtomicU64::new(0));
        for _ in 0..3 {
            let counter = computes.clone();
            let _: u64 = cache
                .cache_aside("k", Duration::from_secs(60), move || async move {
                    counter.fetch_add(1, Ordering::Relaxed);
                    1
                })
                .await;
        }
        assert_eq!(computes.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn keys_are_deterministic_namespaced_and_model_versioned() {
        assert_eq!(keys::ppr("s", "p"), keys::ppr("s", "p"));
        assert_ne!(keys::ppr("s", "p"), keys::ppr("s", "p2"));
        assert_ne!(keys::ppr("ab", ""), keys::ppr("a", "b"), "separator guards");
        assert!(keys::context_pack("x").starts_with("ctxpack:"));
        assert!(keys::embedding("bge-small", "hello").starts_with("emb:bge-small:"));
        assert!(keys::rerank("bge-v1", "q", "cands", "").starts_with("rerank:bge-v1:"));
        assert_ne!(
            keys::rerank("bge-v1", "q", "cands", ""),
            keys::rerank("bge-v2", "q", "cands", ""),
            "model version invalidates the rerank cache"
        );
    }

    #[test]
    fn from_env_without_url_is_disabled() {
        // Whatever the ambient env, with the feature off (default test build)
        // from_env yields a disabled cache.
        let cache = Cache::from_env();
        assert!(!cache.is_enabled());
    }

    /// The embedding-lookups wire: a `CachedTextEmbedder` serves repeated texts
    /// from cache and only sends genuine misses to the inner embedder.
    #[tokio::test]
    async fn cached_embedder_serves_repeats_and_only_embeds_misses() {
        use crate::embedding::{EmbeddingError, TextEmbedder};

        struct CountingEmbedder {
            embedded: Arc<AtomicU64>,
        }
        impl TextEmbedder for CountingEmbedder {
            fn model_id(&self) -> &str {
                "counting-v1"
            }
            fn dimension(&self) -> usize {
                2
            }
            fn embed<'a>(
                &'a self,
                inputs: &'a [String],
            ) -> futures_util::future::BoxFuture<'a, Result<Vec<Vec<f32>>, EmbeddingError>>
            {
                let embedded = self.embedded.clone();
                let inputs = inputs.to_vec();
                Box::pin(async move {
                    embedded.fetch_add(inputs.len() as u64, Ordering::Relaxed);
                    Ok(inputs.iter().map(|_| vec![0.0_f32, 0.0]).collect())
                })
            }
        }

        let embedded = Arc::new(AtomicU64::new(0));
        let inner = CountingEmbedder {
            embedded: embedded.clone(),
        };
        let cache = Cache::new(Arc::new(InMemoryBackend::new()));
        let cached = CachedTextEmbedder::new(inner, cache, Duration::from_secs(60));

        let first = cached
            .embed(&["a".to_string(), "b".to_string()])
            .await
            .unwrap();
        assert_eq!(first.len(), 2);
        // "a" is now cached; the second batch only embeds the new "c".
        let second = cached
            .embed(&["a".to_string(), "c".to_string()])
            .await
            .unwrap();
        assert_eq!(second.len(), 2);

        assert_eq!(
            embedded.load(Ordering::Relaxed),
            3,
            "only the 3 distinct cache-miss texts (a,b,c) were embedded, not 4"
        );
    }
}
