use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use rustyred_thg_core::OrderedIndex;

use super::model::UrlFingerprint;
use super::FrontierResult;

const DEFAULT_FRONTIER_SCAN_LIMIT: usize = 256;

#[async_trait]
pub trait FrontierQueue: Send + Sync {
    async fn remember_domain(&self, fp: &UrlFingerprint, domain: &str) -> FrontierResult<()> {
        let _ = (fp, domain);
        Ok(())
    }

    async fn push(&self, fp: &UrlFingerprint, priority: f64) -> FrontierResult<()>;

    async fn pop_eligible(
        &self,
        is_domain_ready: &(dyn for<'a> Fn(&'a str) -> bool + Sync),
    ) -> FrontierResult<Option<UrlFingerprint>>;

    async fn requeue(&self, fp: &UrlFingerprint, priority: f64) -> FrontierResult<()>;

    async fn len(&self) -> FrontierResult<u64>;
}

#[derive(Debug)]
pub struct MemoryFrontierQueue {
    state: Mutex<MemoryQueueState>,
    scan_limit: usize,
}

#[derive(Debug)]
struct MemoryQueueState {
    ordered: OrderedIndex,
    fingerprints: BTreeMap<String, UrlFingerprint>,
    domains: BTreeMap<String, String>,
}

impl Default for MemoryQueueState {
    fn default() -> Self {
        Self {
            ordered: OrderedIndex::transient(),
            fingerprints: BTreeMap::new(),
            domains: BTreeMap::new(),
        }
    }
}

impl MemoryFrontierQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_scan_limit(mut self, scan_limit: usize) -> Self {
        self.scan_limit = scan_limit.max(1);
        self
    }
}

impl Default for MemoryFrontierQueue {
    fn default() -> Self {
        Self {
            state: Mutex::new(MemoryQueueState::default()),
            scan_limit: DEFAULT_FRONTIER_SCAN_LIMIT,
        }
    }
}

#[async_trait]
impl FrontierQueue for MemoryFrontierQueue {
    async fn remember_domain(&self, fp: &UrlFingerprint, domain: &str) -> FrontierResult<()> {
        let mut state = self.state.lock().expect("memory frontier queue poisoned");
        state.domains.insert(fp.to_hex(), domain.to_string());
        Ok(())
    }

    async fn push(&self, fp: &UrlFingerprint, priority: f64) -> FrontierResult<()> {
        let mut state = self.state.lock().expect("memory frontier queue poisoned");
        let fp_hex = fp.to_hex();
        state.ordered.zadd(fp_hex.as_bytes().to_vec(), priority)?;
        state.fingerprints.insert(fp_hex, *fp);
        Ok(())
    }

    async fn pop_eligible(
        &self,
        is_domain_ready: &(dyn for<'a> Fn(&'a str) -> bool + Sync),
    ) -> FrontierResult<Option<UrlFingerprint>> {
        let mut state = self.state.lock().expect("memory frontier queue poisoned");
        let mut best_key = None;
        for (member, _) in state.ordered.entries_desc(self.scan_limit) {
            let Ok(fp_hex) = String::from_utf8(member) else {
                continue;
            };
            let domain = state.domains.get(&fp_hex).cloned();
            let ready = domain
                .as_deref()
                .is_some_and(|domain| is_domain_ready(domain));
            if ready {
                best_key = Some(fp_hex);
                break;
            }
        }
        let Some(fp_hex) = best_key else {
            return Ok(None);
        };
        state.ordered.zrem(fp_hex.as_bytes());
        state.domains.remove(&fp_hex);
        Ok(state.fingerprints.remove(&fp_hex))
    }

    async fn requeue(&self, fp: &UrlFingerprint, priority: f64) -> FrontierResult<()> {
        self.push(fp, priority).await
    }

    async fn len(&self) -> FrontierResult<u64> {
        let state = self.state.lock().expect("memory frontier queue poisoned");
        Ok(state.ordered.zcard() as u64)
    }
}

#[cfg(feature = "redis-frontier")]
#[derive(Clone, Debug)]
pub struct RedisFrontierQueue {
    client: redis::Client,
    key: String,
    domain_key: String,
    scan_limit: usize,
}

#[cfg(feature = "redis-frontier")]
impl RedisFrontierQueue {
    pub fn new(redis_url: &str, tenant: &str, queue: &str) -> FrontierResult<Self> {
        let client = redis::Client::open(redis_url)?;
        Ok(Self {
            client,
            key: format!("frontier:{tenant}:{queue}"),
            domain_key: format!("frontier:{tenant}:{queue}:domains"),
            scan_limit: DEFAULT_FRONTIER_SCAN_LIMIT,
        })
    }

    pub fn with_scan_limit(mut self, scan_limit: usize) -> Self {
        self.scan_limit = scan_limit.max(1);
        self
    }

    async fn connection(&self) -> FrontierResult<redis::aio::MultiplexedConnection> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(Into::into)
    }
}

#[cfg(feature = "redis-frontier")]
#[async_trait]
impl FrontierQueue for RedisFrontierQueue {
    async fn remember_domain(&self, fp: &UrlFingerprint, domain: &str) -> FrontierResult<()> {
        use redis::AsyncCommands;

        let mut connection = self.connection().await?;
        let _: () = connection
            .hset(&self.domain_key, fp.to_hex(), domain)
            .await?;
        Ok(())
    }

    async fn push(&self, fp: &UrlFingerprint, priority: f64) -> FrontierResult<()> {
        use redis::AsyncCommands;

        let mut connection = self.connection().await?;
        let _: () = connection.zadd(&self.key, fp.to_hex(), priority).await?;
        Ok(())
    }

    async fn pop_eligible(
        &self,
        is_domain_ready: &(dyn for<'a> Fn(&'a str) -> bool + Sync),
    ) -> FrontierResult<Option<UrlFingerprint>> {
        use redis::AsyncCommands;

        let mut connection = self.connection().await?;
        let candidates: Vec<String> = connection
            .zrevrange(&self.key, 0, (self.scan_limit as isize) - 1)
            .await?;
        let mut ready_domains = Vec::new();
        for fp_hex in candidates {
            let domain: Option<String> = connection.hget(&self.domain_key, fp_hex).await?;
            if let Some(domain) = domain.filter(|domain| is_domain_ready(domain)) {
                if !ready_domains.iter().any(|seen| seen == &domain) {
                    ready_domains.push(domain);
                }
            }
        }
        if ready_domains.is_empty() {
            return Ok(None);
        }

        let script = redis::Script::new(
            r#"
local members = redis.call('ZREVRANGE', KEYS[1], 0, tonumber(ARGV[1]) - 1)
for _, member in ipairs(members) do
  local domain = redis.call('HGET', KEYS[2], member) or ''
  for i = 2, #ARGV do
    if domain == ARGV[i] then
      if redis.call('ZREM', KEYS[1], member) == 1 then
        redis.call('HDEL', KEYS[2], member)
        return member
      end
    end
  end
end
return nil
"#,
        );
        let mut invocation = script.prepare_invoke();
        invocation
            .key(&self.key)
            .key(&self.domain_key)
            .arg(self.scan_limit);
        for domain in ready_domains {
            invocation.arg(domain);
        }
        let fp_hex: Option<String> = invocation.invoke_async(&mut connection).await?;
        Ok(fp_hex.and_then(|raw| UrlFingerprint::from_hex(&raw)))
    }

    async fn requeue(&self, fp: &UrlFingerprint, priority: f64) -> FrontierResult<()> {
        self.push(fp, priority).await
    }

    async fn len(&self) -> FrontierResult<u64> {
        use redis::AsyncCommands;

        let mut connection = self.connection().await?;
        let len: u64 = connection.zcard(&self.key).await?;
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontier::model::{fingerprint, UrlFingerprint};

    fn fixed_fp(byte: u8) -> UrlFingerprint {
        UrlFingerprint([byte; 32])
    }

    #[tokio::test]
    async fn memory_queue_pops_highest_ready_priority() {
        let queue = MemoryFrontierQueue::new();
        let low = fingerprint("GET", "https://a.example/", b"");
        let high = fingerprint("GET", "https://b.example/", b"");
        queue.remember_domain(&low, "a.example").await.unwrap();
        queue.remember_domain(&high, "b.example").await.unwrap();
        queue.push(&low, 1.0).await.unwrap();
        queue.push(&high, 10.0).await.unwrap();

        let popped = queue
            .pop_eligible(&|domain| domain == "a.example")
            .await
            .unwrap();
        assert_eq!(popped, Some(low));
        assert_eq!(queue.len().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn memory_queue_equal_priority_uses_descending_member_tie_break() {
        let queue = MemoryFrontierQueue::new();
        let lower = fixed_fp(0x11);
        let higher = fixed_fp(0x22);
        queue
            .remember_domain(&lower, "lower.example")
            .await
            .unwrap();
        queue
            .remember_domain(&higher, "higher.example")
            .await
            .unwrap();
        queue.push(&lower, 1.0).await.unwrap();
        queue.push(&higher, 1.0).await.unwrap();

        let popped = queue.pop_eligible(&|_| true).await.unwrap();
        assert_eq!(popped, Some(higher));
    }

    #[tokio::test]
    async fn memory_queue_scan_limit_bounds_candidate_window() {
        let blocked_high = fixed_fp(0x33);
        let blocked_mid = fixed_fp(0x22);
        let ready_low = fixed_fp(0x11);

        let queue = MemoryFrontierQueue::new().with_scan_limit(2);
        for (fp, domain) in [
            (blocked_high, "blocked-high.example"),
            (blocked_mid, "blocked-mid.example"),
            (ready_low, "ready.example"),
        ] {
            queue.remember_domain(&fp, domain).await.unwrap();
            queue.push(&fp, 1.0).await.unwrap();
        }

        let popped = queue
            .pop_eligible(&|domain| domain == "ready.example")
            .await
            .unwrap();
        assert_eq!(popped, None);
        assert_eq!(queue.len().await.unwrap(), 3);

        let queue = MemoryFrontierQueue::new().with_scan_limit(3);
        for (fp, domain) in [
            (blocked_high, "blocked-high.example"),
            (blocked_mid, "blocked-mid.example"),
            (ready_low, "ready.example"),
        ] {
            queue.remember_domain(&fp, domain).await.unwrap();
            queue.push(&fp, 1.0).await.unwrap();
        }

        let popped = queue
            .pop_eligible(&|domain| domain == "ready.example")
            .await
            .unwrap();
        assert_eq!(popped, Some(ready_low));
    }
}
