//! Gateway configuration, read from the environment once at startup.
//!
//! Every browser-boundary control (CORS origins, the public ingest allowlist,
//! rate-limit shape) and every upstream address (theorem-grpc, GL-Fusion,
//! Valkey) is resolved here so the rest of the crate consumes typed config
//! rather than re-reading `std::env` ad hoc. `GLFUSION_TOKEN` and the internal
//! gRPC/Valkey addresses live here and never leave the server.

use std::time::Duration;

/// Default internal address for theorem-grpc on Railway private networking.
pub const DEFAULT_THEOREM_GRPC_URL: &str = "http://theorem-grpc.railway.internal:8080";

/// Fixed tenant the gateway uses for every code-graph operation. The public
/// demo is single-tenant: ingest and search must land on the same tenant so a
/// repo ingested through `ingestCodebase` is queryable through `searchCode`.
pub const DEFAULT_TENANT_ID: &str = "gateway-public";

#[derive(Clone, Debug)]
pub struct GatewayConfig {
    /// Listen port (Railway injects `PORT`).
    pub port: u16,
    /// Upstream theorem-grpc address (`THEOREM_GRPC_URL`).
    pub grpc_url: String,
    /// Tenant id used for all code-graph ops (`GATEWAY_TENANT_ID`).
    pub tenant_id: String,
    /// GL-Fusion model endpoint (`GLFUSION_URL`); `None` => askAgent returns the
    /// assembled graph context with an honest "model not configured" answer.
    pub glfusion_url: Option<String>,
    /// Optional bearer token for GL-Fusion (`GLFUSION_TOKEN`). Server-side only.
    pub glfusion_token: Option<String>,
    /// Reported model name when GL-Fusion does not echo one (`GLFUSION_MODEL`).
    pub glfusion_model: String,
    /// Browser origins permitted to call `/graphql` (`CORS_ALLOW_ORIGINS`,
    /// comma-separated). Empty => permissive (dev only) with a startup warning.
    pub cors_allow_origins: Vec<String>,
    /// Optional Valkey/Redis URL for rate-limit counters + response cache.
    pub valkey_url: Option<String>,
    /// TTL for recomputable cached responses (`VALKEY_CACHE_TTL_SECONDS`).
    pub valkey_cache_ttl: Duration,
    /// Repo-URL prefixes the public `ingestCodebase` mutation accepts
    /// (`PUBLIC_INGEST_ALLOWLIST`, comma-separated).
    pub ingest_allowlist: Vec<String>,
    /// Token-bucket capacity (burst) for the rate-limited mutations.
    pub rate_limit_burst: u32,
    /// Token-bucket refill rate (tokens per minute) per IP.
    pub rate_limit_per_minute: u32,
    /// How long `ingestCodebase` polls the async job before returning the ack
    /// (`GATEWAY_INGEST_WAIT_SECONDS`). 0 => return immediately on submit.
    pub ingest_wait: Duration,
    /// Optional public base URL for absolute scene URLs (`GATEWAY_PUBLIC_URL`).
    /// When unset, `SceneRef.url` is the relative `/scene/{id}` path.
    pub public_url: Option<String>,
    /// Max compiled scenes held in the in-memory store (`GATEWAY_SCENE_CACHE_SIZE`).
    pub scene_cache_size: usize,
}

impl GatewayConfig {
    pub fn from_env() -> Self {
        let port = env_string("PORT")
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(50080);
        let grpc_url = env_string("THEOREM_GRPC_URL")
            .filter(|s| !s.trim().is_empty())
            .map(|s| normalize_grpc_url(&s))
            .unwrap_or_else(|| DEFAULT_THEOREM_GRPC_URL.to_string());
        let tenant_id = env_string("GATEWAY_TENANT_ID")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_TENANT_ID.to_string());
        let glfusion_url = env_string("GLFUSION_URL").filter(|s| !s.trim().is_empty());
        let glfusion_token = env_string("GLFUSION_TOKEN").filter(|s| !s.trim().is_empty());
        let glfusion_model = env_string("GLFUSION_MODEL")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "theseus-gemma-31b-glfusion".to_string());
        let cors_allow_origins = parse_csv(env_string("CORS_ALLOW_ORIGINS").as_deref());
        let valkey_url = env_string("VALKEY_URL").filter(|s| !s.trim().is_empty());
        let valkey_cache_ttl = Duration::from_secs(
            env_string("VALKEY_CACHE_TTL_SECONDS")
                .and_then(|raw| raw.parse().ok())
                .unwrap_or(60),
        );
        let ingest_allowlist = parse_csv(env_string("PUBLIC_INGEST_ALLOWLIST").as_deref());
        let rate_limit_burst = env_string("GATEWAY_RATE_LIMIT_BURST")
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(10);
        let rate_limit_per_minute = env_string("GATEWAY_RATE_LIMIT_PER_MINUTE")
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(20);
        let ingest_wait = Duration::from_secs(
            env_string("GATEWAY_INGEST_WAIT_SECONDS")
                .and_then(|raw| raw.parse().ok())
                .unwrap_or(180),
        );
        let public_url = env_string("GATEWAY_PUBLIC_URL").filter(|s| !s.trim().is_empty());
        let scene_cache_size = env_string("GATEWAY_SCENE_CACHE_SIZE")
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(256);

        Self {
            port,
            grpc_url,
            tenant_id,
            glfusion_url,
            glfusion_token,
            glfusion_model,
            cors_allow_origins,
            valkey_url,
            valkey_cache_ttl,
            ingest_allowlist,
            rate_limit_burst,
            rate_limit_per_minute,
            ingest_wait,
            public_url,
            scene_cache_size,
        }
    }

    /// The URL the browser embeds for a compiled scene. Absolute when
    /// `GATEWAY_PUBLIC_URL` is set, otherwise the relative `/scene/{id}` path.
    pub fn scene_url(&self, scene_id: &str) -> String {
        match &self.public_url {
            Some(base) => format!("{}/scene/{}", base.trim_end_matches('/'), scene_id),
            None => format!("/scene/{scene_id}"),
        }
    }

    /// AC5: the public `ingestCodebase`/`reindexCodebase` mutations only accept
    /// repo URLs whose normalized form starts with an allowlisted prefix. An
    /// empty allowlist refuses everything (fail closed) — the safe default for
    /// a public browser boundary. The check is pure: it never dials gRPC.
    pub fn ingest_url_allowed(&self, repo_url: &str) -> bool {
        let candidate = repo_url.trim();
        if candidate.is_empty() {
            return false;
        }
        self.ingest_allowlist
            .iter()
            .any(|prefix| candidate.starts_with(prefix.as_str()))
    }
}

fn env_string(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Split a comma-separated env value into trimmed, non-empty entries.
pub fn parse_csv(raw: Option<&str>) -> Vec<String> {
    match raw {
        None => Vec::new(),
        Some(value) => value
            .split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
    }
}

/// tonic needs a scheme-qualified endpoint. Bare `host:port` values (the shape
/// Railway often hands out) are normalized to `http://host:port`, matching the
/// theorem-grpc app-affordance URL normalization.
pub fn normalize_grpc_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_allowlist(prefixes: &[&str]) -> GatewayConfig {
        let mut cfg = GatewayConfig {
            port: 50080,
            grpc_url: DEFAULT_THEOREM_GRPC_URL.to_string(),
            tenant_id: DEFAULT_TENANT_ID.to_string(),
            glfusion_url: None,
            glfusion_token: None,
            glfusion_model: "m".to_string(),
            cors_allow_origins: Vec::new(),
            valkey_url: None,
            valkey_cache_ttl: Duration::from_secs(60),
            ingest_allowlist: Vec::new(),
            rate_limit_burst: 10,
            rate_limit_per_minute: 20,
            ingest_wait: Duration::from_secs(0),
            public_url: None,
            scene_cache_size: 256,
        };
        cfg.ingest_allowlist = prefixes.iter().map(|p| p.to_string()).collect();
        cfg
    }

    #[test]
    fn scene_url_relative_then_absolute() {
        let mut cfg = cfg_with_allowlist(&[]);
        assert_eq!(cfg.scene_url("abc"), "/scene/abc");
        cfg.public_url = Some("https://gw.example/".to_string());
        assert_eq!(cfg.scene_url("abc"), "https://gw.example/scene/abc");
    }

    #[test]
    fn allowlisted_prefix_is_accepted() {
        let cfg = cfg_with_allowlist(&["https://github.com/Travis-Gilbert/"]);
        assert!(cfg.ingest_url_allowed("https://github.com/Travis-Gilbert/RustyRed-Graph-Database"));
    }

    #[test]
    fn non_allowlisted_url_is_refused() {
        let cfg = cfg_with_allowlist(&["https://github.com/Travis-Gilbert/"]);
        assert!(!cfg.ingest_url_allowed("https://github.com/someone-else/evil"));
    }

    #[test]
    fn empty_allowlist_fails_closed() {
        let cfg = cfg_with_allowlist(&[]);
        assert!(!cfg.ingest_url_allowed("https://github.com/Travis-Gilbert/RustyRed-Graph-Database"));
        assert!(!cfg.ingest_url_allowed(""));
    }

    #[test]
    fn parse_csv_trims_and_drops_empties() {
        assert_eq!(
            parse_csv(Some("a, b ,,c,")),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
        assert!(parse_csv(None).is_empty());
    }

    #[test]
    fn normalize_grpc_url_adds_scheme() {
        assert_eq!(
            normalize_grpc_url("theorem-grpc.railway.internal:8080"),
            "http://theorem-grpc.railway.internal:8080"
        );
        assert_eq!(normalize_grpc_url("https://x:1"), "https://x:1");
    }
}
