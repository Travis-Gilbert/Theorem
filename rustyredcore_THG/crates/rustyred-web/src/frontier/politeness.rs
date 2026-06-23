use std::collections::BTreeMap;

use rustyred_thg_core::{GraphStore, NodeQuery, RedCoreGraphStore};
use serde_json::{json, Value};

use super::model::LABEL_DOMAIN;

#[derive(Clone, Debug)]
pub struct Politeness {
    pub default_crawl_delay_ms: u32,
    pub max_in_flight_per_domain: u32,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
}

impl Default for Politeness {
    fn default() -> Self {
        Self {
            default_crawl_delay_ms: 0,
            max_in_flight_per_domain: 1,
            max_retries: 2,
            retry_backoff_ms: 1_000,
        }
    }
}

impl Politeness {
    pub fn ready_domains(&self, store: &RedCoreGraphStore, now_ms: i64) -> BTreeMap<String, bool> {
        GraphStore::query_nodes(store, NodeQuery::label(LABEL_DOMAIN))
            .into_iter()
            .filter_map(|node| {
                let host = node
                    .properties
                    .get("host")
                    .and_then(Value::as_str)
                    .unwrap_or(&node.id)
                    .to_string();
                Some((host, self.domain_node_ready(&node.properties, now_ms)))
            })
            .collect()
    }

    pub fn domain_ready(&self, store: &RedCoreGraphStore, domain: &str, now_ms: i64) -> bool {
        let ready = self.ready_domains(store, now_ms);
        ready.get(domain).copied().unwrap_or(true)
    }

    pub fn retry_priority(&self, current_priority: f64, retry_count: u32) -> f64 {
        let penalty = 10.0 * retry_count.max(1) as f64;
        current_priority - penalty
    }

    pub fn domain_defaults(&self, host: &str) -> Value {
        json!({
            "host": host,
            "last_fetched_at": 0_i64,
            "crawl_delay_ms": self.default_crawl_delay_ms,
            "in_flight_count": 0_u32,
            "budget_remaining": Value::Null,
        })
    }

    fn domain_node_ready(&self, properties: &Value, now_ms: i64) -> bool {
        let last_fetched_at = properties
            .get("last_fetched_at")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let crawl_delay_ms = properties
            .get("crawl_delay_ms")
            .and_then(Value::as_u64)
            .unwrap_or(self.default_crawl_delay_ms as u64);
        let in_flight_count = properties
            .get("in_flight_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);

        in_flight_count < self.max_in_flight_per_domain as u64
            && now_ms.saturating_sub(last_fetched_at) >= crawl_delay_ms as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{NodeRecord, RedCoreGraphStore};

    #[test]
    fn domain_ready_respects_delay_and_inflight() {
        let policy = Politeness {
            default_crawl_delay_ms: 1000,
            max_in_flight_per_domain: 1,
            ..Default::default()
        };
        let mut store = RedCoreGraphStore::memory();
        store
            .upsert_node(NodeRecord::new(
                "example.com",
                [LABEL_DOMAIN],
                json!({
                    "host": "example.com",
                    "last_fetched_at": 1_000_i64,
                    "crawl_delay_ms": 1_000_u32,
                    "in_flight_count": 0_u32,
                }),
            ))
            .unwrap();

        assert!(!policy.domain_ready(&store, "example.com", 1_500));
        assert!(policy.domain_ready(&store, "example.com", 2_000));

        store
            .upsert_node(NodeRecord::new(
                "example.com",
                [LABEL_DOMAIN],
                json!({
                    "host": "example.com",
                    "last_fetched_at": 0_i64,
                    "crawl_delay_ms": 0_u32,
                    "in_flight_count": 1_u32,
                }),
            ))
            .unwrap();
        assert!(!policy.domain_ready(&store, "example.com", 2_000));
    }
}
