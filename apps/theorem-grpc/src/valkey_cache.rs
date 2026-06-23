use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use prost::Message;
use redis::Commands;
use serde::Serialize;
use serde_json::Value;

use rustyred_thg_core::stable_hash;

const VALKEY_URL_ENV: &str = "VALKEY_URL";
const VALKEY_CACHE_TTL_SECONDS_ENV: &str = "VALKEY_CACHE_TTL_SECONDS";
const VALKEY_KEY_PREFIX_ENV: &str = "VALKEY_KEY_PREFIX";
const DEFAULT_TTL_SECONDS: u64 = 60;

#[derive(Clone)]
pub struct ValkeyCache {
    client: Option<redis::Client>,
    key_prefix: String,
    ttl: Duration,
    metrics: Arc<ValkeyCacheMetricsInner>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ValkeyCacheMetrics {
    pub hits: u64,
    pub misses: u64,
    pub writes: u64,
    pub errors: u64,
}

#[derive(Default)]
struct ValkeyCacheMetricsInner {
    hits: AtomicU64,
    misses: AtomicU64,
    writes: AtomicU64,
    errors: AtomicU64,
}

impl ValkeyCache {
    pub fn from_env() -> Self {
        let client = std::env::var(VALKEY_URL_ENV)
            .ok()
            .filter(|url| !url.trim().is_empty())
            .and_then(|url| redis::Client::open(url).ok());
        let ttl_seconds = std::env::var(VALKEY_CACHE_TTL_SECONDS_ENV)
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|ttl| *ttl > 0)
            .unwrap_or(DEFAULT_TTL_SECONDS);
        let key_prefix = std::env::var(VALKEY_KEY_PREFIX_ENV)
            .ok()
            .filter(|prefix| !prefix.trim().is_empty())
            .unwrap_or_else(|| "theorem-grpc".to_string());
        Self {
            client,
            key_prefix,
            ttl: Duration::from_secs(ttl_seconds),
            metrics: Arc::new(ValkeyCacheMetricsInner::default()),
        }
    }

    pub fn disabled() -> Self {
        Self {
            client: None,
            key_prefix: "theorem-grpc".to_string(),
            ttl: Duration::from_secs(DEFAULT_TTL_SECONDS),
            metrics: Arc::new(ValkeyCacheMetricsInner::default()),
        }
    }

    pub fn cache_key(&self, namespace: &str, input: impl Serialize) -> String {
        let encoded = serde_json::to_value(input).unwrap_or(Value::Null);
        format!(
            "{}:{}:{}",
            self.key_prefix,
            namespace,
            stable_hash(&encoded)
        )
    }

    pub fn get_proto<M>(&self, key: &str) -> Option<M>
    where
        M: Message + Default,
    {
        let Some(client) = &self.client else {
            return None;
        };
        let result = client
            .get_connection()
            .and_then(|mut connection| connection.get::<_, Option<Vec<u8>>>(key));
        match result {
            Ok(Some(bytes)) => match M::decode(bytes.as_slice()) {
                Ok(message) => {
                    self.metrics.hits.fetch_add(1, Ordering::Relaxed);
                    Some(message)
                }
                Err(_) => {
                    self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                    None
                }
            },
            Ok(None) => {
                self.metrics.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
            Err(_) => {
                self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    pub fn put_proto<M>(&self, key: &str, message: &M)
    where
        M: Message,
    {
        let Some(client) = &self.client else {
            return;
        };
        let bytes = message.encode_to_vec();
        let result = client.get_connection().and_then(|mut connection| {
            connection.set_ex::<_, _, ()>(key, bytes, self.ttl.as_secs())
        });
        match result {
            Ok(()) => {
                self.metrics.writes.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.metrics.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn get_json(&self, key: &str) -> Option<Value> {
        let Some(client) = &self.client else {
            return None;
        };
        let result = client
            .get_connection()
            .and_then(|mut connection| connection.get::<_, Option<String>>(key));
        match result {
            Ok(Some(raw)) => match serde_json::from_str(&raw) {
                Ok(value) => {
                    self.metrics.hits.fetch_add(1, Ordering::Relaxed);
                    Some(value)
                }
                Err(_) => {
                    self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                    None
                }
            },
            Ok(None) => {
                self.metrics.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
            Err(_) => {
                self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    pub fn put_json(&self, key: &str, value: &Value) {
        let Some(client) = &self.client else {
            return;
        };
        let Ok(raw) = serde_json::to_string(value) else {
            self.metrics.errors.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let result = client
            .get_connection()
            .and_then(|mut connection| connection.set_ex::<_, _, ()>(key, raw, self.ttl.as_secs()));
        match result {
            Ok(()) => {
                self.metrics.writes.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.metrics.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn ping(&self) -> Result<Option<String>, String> {
        let Some(client) = &self.client else {
            return Ok(None);
        };
        client
            .get_connection()
            .and_then(|mut connection| redis::cmd("PING").query::<String>(&mut connection))
            .map(Some)
            .map_err(|error| error.to_string())
    }

    pub fn metrics(&self) -> ValkeyCacheMetrics {
        ValkeyCacheMetrics {
            hits: self.metrics.hits.load(Ordering::Relaxed),
            misses: self.metrics.misses.load(Ordering::Relaxed),
            writes: self.metrics.writes.load(Ordering::Relaxed),
            errors: self.metrics.errors.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_cache_is_noop_and_stable_keys_hash_inputs() {
        let cache = ValkeyCache::disabled();
        let key_a = cache.cache_key("gap_walk", serde_json::json!({ "query": "parks" }));
        let key_b = cache.cache_key("gap_walk", serde_json::json!({ "query": "parks" }));
        assert_eq!(key_a, key_b);
        assert!(key_a.starts_with("theorem-grpc:gap_walk:sha256:"));
        assert_eq!(cache.get_json(&key_a), None);
        cache.put_json(&key_a, &serde_json::json!({ "ok": true }));
        assert_eq!(cache.metrics(), ValkeyCacheMetrics::default());
    }
}
