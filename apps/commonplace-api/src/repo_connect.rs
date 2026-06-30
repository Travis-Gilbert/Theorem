//! Repository-connect bridge for CommonPlace.
//!
//! CommonPlace owns the user action, but the code mirror/index implementation
//! lives in `rustyred-workspace`. This module keeps that boundary explicit: the
//! GraphQL schema depends on a small trait, and runtime wiring can provide the
//! Engine-backed implementation when a mirror engine directory is configured.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rustyred_embedded::{EmbeddedConfig, Engine};
use rustyred_thg_code::{GitCredential, GitCredentialResolver};
use rustyred_workspace::{
    connect_commonplace_repo_mirror, import_repo_url_mirror, MirrorAuditTarget,
    MirrorImportOptions, WorkspaceMirrorAuditMonitor, DEFAULT_MIRROR_AUDIT_INTERVAL,
};

pub type GitCredentialResolverRef = Arc<dyn GitCredentialResolver>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepositoryConnectInput {
    pub repo_path: Option<PathBuf>,
    pub repo_url: Option<String>,
    pub credential_ref: Option<String>,
    pub github_installation_id: Option<u64>,
    pub workspace_root: Option<PathBuf>,
    pub prefix: Option<String>,
    pub max_file_bytes: Option<u64>,
    pub max_total_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryConnectReceipt {
    pub root: PathBuf,
    pub files_mirrored: usize,
    pub files_indexed: usize,
    pub files_removed: usize,
    pub files_skipped: usize,
    pub bytes_mirrored: u64,
    pub bytes_indexed: u64,
    pub paths: Vec<String>,
    pub clone_ms: u64,
}

pub trait RepositoryConnector: Send + Sync {
    fn connect_repository(
        &self,
        input: RepositoryConnectInput,
    ) -> Result<RepositoryConnectReceipt, String>;

    fn audit_prometheus(&self) -> Option<String> {
        None
    }
}

pub type RepositoryConnectorRef = Arc<dyn RepositoryConnector>;

pub struct EngineRepositoryConnector {
    engine_dir: PathBuf,
    engine_lock: Arc<Mutex<()>>,
    default_workspace_root: Option<PathBuf>,
    default_prefix: String,
    credential_resolver: Option<GitCredentialResolverRef>,
    audit_monitor: Option<Arc<WorkspaceMirrorAuditMonitor>>,
}

impl EngineRepositoryConnector {
    pub fn new(
        engine_dir: impl Into<PathBuf>,
        default_workspace_root: Option<PathBuf>,
        default_prefix: impl Into<String>,
    ) -> Self {
        Self {
            engine_dir: engine_dir.into(),
            engine_lock: Arc::new(Mutex::new(())),
            default_workspace_root,
            default_prefix: default_prefix.into(),
            credential_resolver: None,
            audit_monitor: None,
        }
    }

    pub fn new_with_integrations(
        engine_dir: impl Into<PathBuf>,
        default_workspace_root: Option<PathBuf>,
        default_prefix: impl Into<String>,
        credential_resolver: Option<GitCredentialResolverRef>,
        audit_monitor: Option<Arc<WorkspaceMirrorAuditMonitor>>,
    ) -> Self {
        Self {
            engine_dir: engine_dir.into(),
            engine_lock: Arc::new(Mutex::new(())),
            default_workspace_root,
            default_prefix: default_prefix.into(),
            credential_resolver,
            audit_monitor,
        }
    }

    pub fn new_with_credential_resolver(
        engine_dir: impl Into<PathBuf>,
        default_workspace_root: Option<PathBuf>,
        default_prefix: impl Into<String>,
        credential_resolver: Option<GitCredentialResolverRef>,
    ) -> Self {
        Self::new_with_integrations(
            engine_dir,
            default_workspace_root,
            default_prefix,
            credential_resolver,
            None,
        )
    }

    pub fn with_credential_resolver(
        mut self,
        credential_resolver: GitCredentialResolverRef,
    ) -> Self {
        self.credential_resolver = Some(credential_resolver);
        self
    }

    pub fn with_audit_monitor(mut self, audit_monitor: Arc<WorkspaceMirrorAuditMonitor>) -> Self {
        self.audit_monitor = Some(audit_monitor);
        self
    }

    pub fn open(
        engine_dir: impl Into<PathBuf>,
        default_workspace_root: Option<PathBuf>,
        default_prefix: impl Into<String>,
    ) -> Result<Self, String> {
        let engine_dir = engine_dir.into();
        let config = EmbeddedConfig::load_for_dir(&engine_dir)
            .map_err(|error| format!("load mirror engine config: {error}"))?;
        let _engine = Engine::open(&engine_dir, config)
            .map_err(|error| format!("open mirror engine: {error}"))?;
        Ok(Self::new(
            engine_dir,
            default_workspace_root,
            default_prefix,
        ))
    }

    pub fn engine_dir(&self) -> &PathBuf {
        &self.engine_dir
    }

    fn resolve_repo_credential(
        &self,
        repo_url: &str,
        credential_ref: Option<&str>,
        github_installation_id: Option<u64>,
    ) -> Result<Option<GitCredential>, String> {
        let credential_ref = credential_ref
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let mut installation_id = github_installation_id;

        if let Some(credential_ref) = credential_ref {
            if let Some(raw_id) = credential_ref.strip_prefix("github-installation:") {
                let parsed = raw_id.trim().parse::<u64>().map_err(|error| {
                    format!("credentialRef github-installation requires a numeric id: {error}")
                })?;
                if let Some(existing) = installation_id {
                    if existing != parsed {
                        return Err(
                            "credentialRef github-installation conflicts with githubInstallationId"
                                .to_string(),
                        );
                    }
                }
                installation_id = Some(parsed);
            } else if credential_ref == "server:default" || credential_ref == "default" {
                let resolver = self.credential_resolver.as_ref().ok_or_else(|| {
                    "no repository credential resolver configured for server default credential"
                        .to_string()
                })?;
                return resolver
                    .resolve(repo_url)
                    .ok_or_else(|| {
                        "credential resolver returned no server default credential".to_string()
                    })
                    .map(Some);
            } else {
                return Err(
                    "unsupported credentialRef; expected server:default or github-installation:ID"
                        .to_string(),
                );
            }
        }

        if let Some(installation_id) = installation_id {
            let resolver = self.credential_resolver.as_ref().ok_or_else(|| {
                format!(
                    "no repository credential resolver configured for GitHub installation {installation_id}"
                )
            })?;
            return resolver
                .resolve_installation(repo_url, installation_id)
                .ok_or_else(|| {
                    format!(
                        "credential resolver returned no credential for GitHub installation {installation_id}"
                    )
                })
                .map(Some);
        }

        Ok(self
            .credential_resolver
            .as_ref()
            .and_then(|resolver| resolver.resolve(repo_url)))
    }
}

impl RepositoryConnector for EngineRepositoryConnector {
    fn connect_repository(
        &self,
        input: RepositoryConnectInput,
    ) -> Result<RepositoryConnectReceipt, String> {
        let RepositoryConnectInput {
            repo_path,
            repo_url,
            credential_ref,
            github_installation_id,
            workspace_root,
            prefix,
            max_file_bytes,
            max_total_bytes,
        } = input;

        let has_path = repo_path.is_some();
        let has_url = repo_url.as_ref().is_some_and(|url| !url.trim().is_empty());
        match (has_path, has_url) {
            (true, true) => {
                return Err("provide repo_path or repo_url, not both".to_string());
            }
            (false, false) => {
                return Err("provide repo_path or repo_url".to_string());
            }
            _ => {}
        }
        if has_path
            && (credential_ref
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty())
                || github_installation_id.is_some())
        {
            return Err("credentials apply only to repo_url".to_string());
        }

        let options = MirrorImportOptions {
            prefix: prefix
                .filter(|prefix| !prefix.trim().is_empty())
                .unwrap_or_else(|| self.default_prefix.clone()),
            workspace_root: workspace_root.or_else(|| self.default_workspace_root.clone()),
            max_file_bytes: max_file_bytes
                .unwrap_or_else(|| MirrorImportOptions::default().max_file_bytes),
            max_total_bytes: max_total_bytes
                .unwrap_or_else(|| MirrorImportOptions::default().max_total_bytes),
        };
        let target_options = options.clone();
        let receipt = {
            let _engine_guard = self
                .engine_lock
                .lock()
                .map_err(|_| "repository mirror engine lock poisoned".to_string())?;
            let config = EmbeddedConfig::load_for_dir(&self.engine_dir)
                .map_err(|error| format!("load mirror engine config: {error}"))?;
            let engine = Engine::open(&self.engine_dir, config)
                .map_err(|error| format!("open mirror engine: {error}"))?;
            if let Some(repo_path) = repo_path {
                connect_commonplace_repo_mirror(&engine, repo_path, options)
            } else {
                let repo_url = repo_url.expect("checked repo_url above");
                let repo_url = repo_url.trim();
                let credential = self.resolve_repo_credential(
                    repo_url,
                    credential_ref.as_deref(),
                    github_installation_id,
                )?;
                import_repo_url_mirror(&engine, repo_url, options, credential.as_ref())
            }
            .map_err(|error| error.to_string())?
        };
        if let Some(monitor) = &self.audit_monitor {
            monitor.add_target(MirrorAuditTarget::new(receipt.root.clone(), target_options));
            monitor.run_once();
        }

        Ok(RepositoryConnectReceipt {
            root: receipt.root,
            files_mirrored: receipt.files_mirrored,
            files_indexed: receipt.index.files_indexed,
            files_removed: receipt.index.files_removed,
            files_skipped: receipt.files_skipped,
            bytes_mirrored: receipt.bytes_mirrored,
            bytes_indexed: receipt.index.bytes_indexed,
            paths: receipt.paths,
            clone_ms: receipt.clone_ms,
        })
    }

    fn audit_prometheus(&self) -> Option<String> {
        self.audit_monitor
            .as_ref()
            .map(|monitor| monitor.latest_prometheus())
    }
}

pub fn connector_from_env() -> Result<Option<RepositoryConnectorRef>, String> {
    let Some(engine_dir) = env_path("COMMONPLACE_REPO_MIRROR_ENGINE_DIR") else {
        return Ok(None);
    };
    let workspace_root = env_path("COMMONPLACE_REPO_MIRROR_WORKSPACE_ROOT");
    let prefix = std::env::var("COMMONPLACE_REPO_MIRROR_PREFIX")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "repos/commonplace".to_string());
    let connector = EngineRepositoryConnector::open(engine_dir, workspace_root, prefix)?;
    let connector = match credential_resolver_from_env()? {
        Some(resolver) => connector.with_credential_resolver(resolver),
        None => connector,
    };
    let connector = match audit_monitor_from_env(connector.engine_dir(), &connector)? {
        Some(monitor) => connector.with_audit_monitor(monitor),
        None => connector,
    };
    Ok(Some(Arc::new(connector)))
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn credential_resolver_from_env() -> Result<Option<GitCredentialResolverRef>, String> {
    if let Some(env_var) = std::env::var("COMMONPLACE_REPO_MIRROR_GIT_TOKEN_ENV")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(Arc::new(EnvGitCredentialResolver::new(env_var))));
    }

    if std::env::var("COMMONPLACE_REPO_MIRROR_GIT_TOKEN")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return Ok(Some(Arc::new(EnvGitCredentialResolver::new(
            "COMMONPLACE_REPO_MIRROR_GIT_TOKEN",
        ))));
    }

    Ok(None)
}

fn audit_monitor_from_env(
    engine_dir: &PathBuf,
    connector: &EngineRepositoryConnector,
) -> Result<Option<Arc<WorkspaceMirrorAuditMonitor>>, String> {
    let interval = audit_interval_from_env();
    let mut targets = Vec::new();
    if let Some(root) = connector.default_workspace_root.clone() {
        targets.push(MirrorAuditTarget::new(
            root.clone(),
            MirrorImportOptions {
                prefix: connector.default_prefix.clone(),
                workspace_root: Some(root),
                ..MirrorImportOptions::default()
            },
        ));
    }
    WorkspaceMirrorAuditMonitor::start_with_engine_lock(
        engine_dir.clone(),
        targets,
        interval,
        Arc::clone(&connector.engine_lock),
    )
    .map(Arc::new)
    .map(Some)
    .map_err(|error| format!("start repository mirror audit monitor: {error}"))
}

fn audit_interval_from_env() -> std::time::Duration {
    std::env::var("COMMONPLACE_REPO_MIRROR_AUDIT_INTERVAL_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or(DEFAULT_MIRROR_AUDIT_INTERVAL)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvGitCredentialResolver {
    env_var: String,
}

impl EnvGitCredentialResolver {
    pub fn new(env_var: impl Into<String>) -> Self {
        Self {
            env_var: env_var.into(),
        }
    }
}

impl GitCredentialResolver for EnvGitCredentialResolver {
    fn resolve(&self, _repo_url: &str) -> Option<GitCredential> {
        std::env::var(&self.env_var)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(GitCredential::BearerToken)
    }
}
