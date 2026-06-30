//! Repository-connect acceptance: CommonPlace's repo-connect GraphQL mutation
//! lands real files in the workspace mirror and builds the downstream File index.

use std::process::Command;
use std::sync::{Arc, Mutex};

use async_graphql::{Request, Variables};
use commonplace_api::{
    build_schema_with_model_and_repository_connector, in_memory_store, ApiKeyRegistry, ApiKeyToken,
    EngineRepositoryConnector, GitCredentialResolverRef, NoModel, RepositoryConnectorRef,
};
use rustyred_embedded::{EmbeddedConfig, Engine};
use rustyred_thg_code::{GitCredential, GitCredentialResolver};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn connect_repository_lands_real_files_and_indexes_file_nodes() {
    let checkout = TempDir::new().expect("checkout tempdir");
    write(checkout.path().join("README.md"), "# fixture\n");
    write(
        checkout.path().join("src/lib.rs"),
        "pub fn answer() -> u8 { 42 }\n",
    );
    write(checkout.path().join("target/debug/app"), "artifact\n");
    write(checkout.path().join(".git/config"), "[core]\n");

    let workspace = TempDir::new().expect("workspace tempdir");
    let engine_dir = TempDir::new().expect("engine tempdir");
    let connector: RepositoryConnectorRef = Arc::new(EngineRepositoryConnector::new(
        engine_dir.path(),
        Some(workspace.path().to_path_buf()),
        "repos/commonplace",
    ));
    let key = "valid-key";
    let registry = Arc::new(ApiKeyRegistry::new().with_key(key, "instance"));
    let schema = build_schema_with_model_and_repository_connector(
        in_memory_store(),
        registry,
        Arc::new(NoModel),
        Some(connector),
    );

    let mutation = r#"
        mutation($input: RepositoryConnectInputGql!) {
            connectRepository(input: $input) {
                root
                filesMirrored
                filesIndexed
                filesSkipped
                bytesIndexed
                paths
            }
        }
    "#;
    let response = schema
        .execute(
            Request::new(mutation)
                .variables(Variables::from_json(json!({
                    "input": {
                        "repoPath": checkout.path().display().to_string(),
                        "prefix": "repos/demo"
                    }
                })))
                .data(ApiKeyToken(key.to_string())),
        )
        .await;
    assert!(
        response.errors.is_empty(),
        "connectRepository errors: {:?}",
        response.errors
    );
    let data = response.data.into_json().expect("response JSON");
    let receipt = &data["connectRepository"];
    assert_eq!(receipt["filesMirrored"], 2);
    assert_eq!(receipt["filesIndexed"], 2);
    assert!(
        receipt["filesSkipped"].as_i64().unwrap() >= 1,
        "filtered artifact paths are counted as skipped"
    );
    assert_eq!(
        receipt["paths"],
        json!(["repos/demo/README.md", "repos/demo/src/lib.rs"])
    );

    assert!(workspace.path().join("README.md").is_file());
    assert!(workspace.path().join("src/lib.rs").is_file());
    assert!(!workspace.path().join("target/debug/app").exists());
    assert!(!workspace.path().join(".git/config").exists());

    let engine =
        Engine::open(engine_dir.path(), EmbeddedConfig::default()).expect("reopen mirror engine");
    assert_eq!(
        engine
            .fs_read("repos/demo/src/lib.rs")
            .expect("fs_read")
            .as_deref(),
        Some(&b"pub fn answer() -> u8 { 42 }\n"[..])
    );
    let node = engine
        .query("query{ graphNode(id:\"file:repos/demo/src/lib.rs\") }")
        .expect("graph query");
    assert!(!node["graphNode"].is_null(), "File node is queryable");
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn connect_repository_uses_github_installation_credential_for_repo_url() {
    let source = TempDir::new().expect("source git repo");
    write(source.path().join("README.md"), "# private fixture\n");
    write(
        source.path().join("src/lib.rs"),
        "pub fn private_answer() -> u8 { 7 }\n",
    );
    git(source.path(), &["init"]);
    git(source.path(), &["add", "README.md", "src/lib.rs"]);
    git(
        source.path(),
        &[
            "-c",
            "user.email=repo-connect@example.invalid",
            "-c",
            "user.name=Repo Connect",
            "commit",
            "-m",
            "fixture",
        ],
    );

    let workspace = TempDir::new().expect("workspace tempdir");
    let engine_dir = TempDir::new().expect("engine tempdir");
    let resolver = Arc::new(RecordingGitCredentialResolver::default());
    let credential_resolver: GitCredentialResolverRef = resolver.clone();
    let connector: RepositoryConnectorRef =
        Arc::new(EngineRepositoryConnector::new_with_credential_resolver(
            engine_dir.path(),
            Some(workspace.path().to_path_buf()),
            "repos/commonplace",
            Some(credential_resolver),
        ));
    let key = "valid-key";
    let registry = Arc::new(ApiKeyRegistry::new().with_key(key, "instance"));
    let schema = build_schema_with_model_and_repository_connector(
        in_memory_store(),
        registry,
        Arc::new(NoModel),
        Some(connector),
    );

    let repo_url = format!("file://{}", source.path().display());
    let response = schema
        .execute(
            Request::new(
                r#"
                mutation($input: RepositoryConnectInputGql!) {
                    connectRepository(input: $input) {
                        filesMirrored
                        filesIndexed
                        paths
                    }
                }
                "#,
            )
            .variables(Variables::from_json(json!({
                "input": {
                    "repoUrl": repo_url,
                    "githubInstallationId": 42,
                    "prefix": "repos/private"
                }
            })))
            .data(ApiKeyToken(key.to_string())),
        )
        .await;

    assert!(
        response.errors.is_empty(),
        "connectRepository errors: {:?}",
        response.errors
    );
    let data = response.data.into_json().expect("response JSON");
    assert_eq!(data["connectRepository"]["filesMirrored"], 2);
    assert_eq!(data["connectRepository"]["filesIndexed"], 2);
    assert_eq!(
        data["connectRepository"]["paths"],
        json!(["repos/private/README.md", "repos/private/src/lib.rs"])
    );
    assert!(workspace.path().join("README.md").is_file());
    assert!(workspace.path().join("src/lib.rs").is_file());
    let calls = resolver.calls.lock().expect("resolver calls");
    assert_eq!(calls.as_slice(), &[(repo_url, Some(42))]);
}

#[tokio::test]
async fn connect_repository_rejects_env_credential_ref_from_api() {
    let workspace = TempDir::new().expect("workspace tempdir");
    let engine_dir = TempDir::new().expect("engine tempdir");
    let connector: RepositoryConnectorRef = Arc::new(EngineRepositoryConnector::new(
        engine_dir.path(),
        Some(workspace.path().to_path_buf()),
        "repos/commonplace",
    ));
    let key = "valid-key";
    let registry = Arc::new(ApiKeyRegistry::new().with_key(key, "instance"));
    let schema = build_schema_with_model_and_repository_connector(
        in_memory_store(),
        registry,
        Arc::new(NoModel),
        Some(connector),
    );
    let response = schema
        .execute(
            Request::new(
                r#"mutation {
                    connectRepository(input: {
                        repoUrl: "https://github.com/private/repo.git",
                        credentialRef: "env:HOME"
                    }) { root }
                }"#,
            )
            .data(ApiKeyToken(key.to_string())),
        )
        .await;
    assert!(
        response.errors.iter().any(|error| error.message.contains(
            "unsupported credentialRef; expected server:default or github-installation:ID"
        )),
        "expected env credentialRef rejection, got {:?}",
        response.errors
    );
}

#[tokio::test]
async fn connect_repository_requires_resolver_for_github_installation_id() {
    let engine_dir = TempDir::new().expect("engine tempdir");
    let workspace = TempDir::new().expect("workspace");
    let connector: RepositoryConnectorRef = Arc::new(EngineRepositoryConnector::new(
        engine_dir.path(),
        Some(workspace.path().to_path_buf()),
        "repos/commonplace",
    ));
    let key = "valid-key";
    let registry = Arc::new(ApiKeyRegistry::new().with_key(key, "instance"));
    let schema = build_schema_with_model_and_repository_connector(
        in_memory_store(),
        registry,
        Arc::new(NoModel),
        Some(connector),
    );
    let response = schema
        .execute(
            Request::new(
                r#"
                mutation {
                    connectRepository(input: {
                        repoUrl: "https://github.com/private/repo.git",
                        githubInstallationId: 42
                    }) { root }
                }
                "#,
            )
            .data(ApiKeyToken(key.to_string())),
        )
        .await;
    assert!(
        response.errors.iter().any(|error| error
            .message
            .contains("no repository credential resolver configured")),
        "expected resolver error, got {:?}",
        response.errors
    );
}

#[tokio::test]
async fn connect_repository_reports_not_configured_without_connector() {
    let key = "valid-key";
    let registry = Arc::new(ApiKeyRegistry::new().with_key(key, "instance"));
    let schema = build_schema_with_model_and_repository_connector(
        in_memory_store(),
        registry,
        Arc::new(NoModel),
        None,
    );
    let response = schema
        .execute(
            Request::new(
                r#"mutation {
                    connectRepository(input: { repoPath: "/tmp/nope" }) { root }
                }"#,
            )
            .data(ApiKeyToken(key.to_string())),
        )
        .await;
    assert!(
        response.errors.iter().any(|error| error
            .message
            .contains("repository connector is not configured")),
        "expected not-configured error, got {:?}",
        response.errors
    );
}

#[derive(Default)]
struct RecordingGitCredentialResolver {
    calls: Mutex<Vec<(String, Option<u64>)>>,
}

impl GitCredentialResolver for RecordingGitCredentialResolver {
    fn resolve(&self, repo_url: &str) -> Option<GitCredential> {
        self.calls
            .lock()
            .expect("resolver calls")
            .push((repo_url.to_string(), None));
        Some(GitCredential::BearerToken("test-token".to_string()))
    }

    fn resolve_installation(&self, repo_url: &str, installation_id: u64) -> Option<GitCredential> {
        self.calls
            .lock()
            .expect("resolver calls")
            .push((repo_url.to_string(), Some(installation_id)));
        Some(GitCredential::BearerToken("test-token".to_string()))
    }
}

fn git(repo: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(path: std::path::PathBuf, body: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, body).expect("write fixture");
}
