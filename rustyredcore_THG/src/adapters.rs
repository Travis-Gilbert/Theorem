use std::env;
use std::path::PathBuf;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};
use rustyred_thg_adapters as adapters;
use rustyred_thg_core::{
    sanitize_tenant_segment, RedCoreDurability, RedCoreGraphStore, RedCoreOptions, ThgError,
};

pyo3::create_exception!(theseus_native, ThgErrorPy, PyException);

#[derive(Debug, Clone)]
struct AdapterStoreConfig {
    // Store root selected from environment; tenant data lives under `<base>/tenants/<tenant>`.
    base_data_dir: PathBuf,
    // Fallback tenant when tenant-aware discovery finds no tenant directories.
    default_tenant: String,
}

impl AdapterStoreConfig {
    fn from_env() -> Self {
        Self {
            // Current harness/runtime env names are checked first, then
            // legacy adapter and product-prefixed names.
            base_data_dir: first_non_empty_env_value(&[
                "THEOREM_HARNESS_DATA_DIR",
                "THEOREM_DATA_DIR",
                "RUSTYRED_THG_ADAPTER_DATA_DIR",
                "RUSTY_RED_DATA_DIR",
                "RUSTYRED_THG_PRODUCT_DATA_DIR",
            ])
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data/rusty-red")),
            default_tenant: first_non_empty_env_value(&[
                "THEOREM_TENANT_SLUG",
                "RUSTYRED_THG_TENANT_SLUG",
                "THEOREM_TENANT_ID",
                "THEOREM_HARNESS_TENANT_SLUG",
                "THEOREM_AGENT_TENANT_SLUG",
                "RUSTYRED_THG_ADAPTER_DEFAULT_TENANT",
                "RUSTY_RED_MCP_DEFAULT_TENANT",
                "RUSTYRED_THG_MCP_DEFAULT_TENANT",
            ])
            .unwrap_or_else(|| "default".to_string()),
        }
    }

    fn tenant_data_dir(&self, tenant_id: &str) -> PathBuf {
        self.base_data_dir
            .join("tenants")
            .join(sanitize_tenant_segment(tenant_id))
    }

    fn discover_tenants(&self) -> Result<Vec<String>, ThgError> {
        let tenants_root = self.base_data_dir.join("tenants");
        let mut tenants = Vec::new();
        let entries = std::fs::read_dir(&tenants_root).map_err(|error| {
            ThgError::new(
                "adapter_tenant_discovery_failed",
                format!(
                    "unable to read adapter tenants at '{}': {}",
                    tenants_root.display(),
                    error
                ),
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                ThgError::new(
                    "adapter_tenant_discovery_failed",
                    format!(
                        "unable to read adapter tenant entry under '{}': {}",
                        tenants_root.display(),
                        error
                    ),
                )
            })?;
            let file_type = entry.file_type().map_err(|error| {
                ThgError::new(
                    "adapter_tenant_discovery_failed",
                    format!(
                        "unable to inspect adapter tenant entry '{}' under '{}': {}",
                        entry.file_name().to_string_lossy(),
                        tenants_root.display(),
                        error
                    ),
                )
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let tenant = entry.file_name().into_string().map_err(|raw| {
                ThgError::new(
                    "adapter_tenant_discovery_failed",
                    format!(
                        "adapter tenant directory name is not valid UTF-8 under '{}': {:?}",
                        tenants_root.display(),
                        raw
                    ),
                )
            })?;
            tenants.push(tenant);
        }
        tenants.sort_unstable();
        if tenants.is_empty() {
            tenants.push(self.default_tenant.clone());
        }
        Ok(tenants)
    }

    fn open_store_for_tenant(&self, tenant_id: &str) -> Result<RedCoreGraphStore, ThgError> {
        let tenant_data_dir = self.tenant_data_dir(tenant_id);
        RedCoreGraphStore::open(
            tenant_data_dir.clone(),
            RedCoreOptions {
                durability: RedCoreDurability::AofEverysec,
                snapshot_interval_writes: 1_000,
                strict_acid: false,
            },
        )
        .map_err(|error| {
            let code = if error.code.trim().is_empty() {
                "adapter_graph_store_open_failed"
            } else {
                error.code.as_str()
            };
            ThgError::new(
                code,
                format!(
                    "unable to open adapter store for tenant '{}' at '{}': {}",
                    tenant_id,
                    tenant_data_dir.display(),
                    error.message
                ),
            )
        })
    }
}

fn first_non_empty_env_value(keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .next()
}

#[pyclass(module = "theseus_native.adapters", get_all, set_all)]
#[derive(Clone)]
pub struct LoraAdapter {
    pub adapter_id: String,
    pub tenant_id: String,
    pub base_model_sha: String,
    pub rank: u32,
    pub target_modules: Vec<String>,
    pub s3_uri: String,
    pub training_object_ids: Vec<i64>,
    pub version: u32,
    pub fitness: f32,
    pub created_at_ms: i64,
    pub manifest_version: u32,
}

#[pymethods]
impl LoraAdapter {
    #[new]
    #[pyo3(signature = (
        adapter_id,
        tenant_id,
        base_model_sha,
        rank,
        target_modules,
        s3_uri,
        training_object_ids,
        version,
        fitness = 0.5,
        created_at_ms = 0,
        manifest_version = 1
    ))]
    fn new(
        adapter_id: String,
        tenant_id: String,
        base_model_sha: String,
        rank: u32,
        target_modules: Vec<String>,
        s3_uri: String,
        training_object_ids: Vec<i64>,
        version: u32,
        fitness: f32,
        created_at_ms: i64,
        manifest_version: u32,
    ) -> Self {
        Self {
            adapter_id,
            tenant_id,
            base_model_sha,
            rank,
            target_modules,
            s3_uri,
            training_object_ids,
            version,
            fitness,
            created_at_ms,
            manifest_version,
        }
    }
}

#[pyclass(module = "theseus_native.adapters", get_all)]
#[derive(Clone)]
pub struct AdapterRef {
    pub adapter: LoraAdapter,
    pub score: f32,
}

#[pyfunction]
#[pyo3(signature = (
    tenant_id,
    seed_node_ids,
    k,
    *,
    base_model_sha = None,
    include_superseded = false,
    min_fitness = Some(0.3),
    ppr_damping = 0.85,
    ppr_max_iter = 30,
    shared_weight = Some(0.5)
))]
pub fn find_adapters(
    tenant_id: String,
    seed_node_ids: Vec<String>,
    k: u32,
    base_model_sha: Option<String>,
    include_superseded: bool,
    min_fitness: Option<f32>,
    ppr_damping: f32,
    ppr_max_iter: u32,
    shared_weight: Option<f32>,
) -> PyResult<Vec<AdapterRef>> {
    let store = open_store_for_tenant(&tenant_id)?;
    let request = adapters::AdapterFindRequest {
        tenant_id,
        seed_node_ids,
        k,
        base_model_sha,
        include_superseded,
        min_fitness,
        ppr_damping,
        ppr_max_iter,
        shared_weight,
    };
    adapters::find_adapters_for(&store, &request)
        .map(|items| items.into_iter().map(AdapterRef::from).collect())
        .map_err(py_thg_error)
}

#[pyfunction]
#[pyo3(signature = (adapter, derived_from_adapter_id = None))]
pub fn upsert_adapter(
    py: Python<'_>,
    adapter: LoraAdapter,
    derived_from_adapter_id: Option<String>,
) -> PyResult<PyObject> {
    let mut store = open_store_for_tenant(&adapter.tenant_id)?;
    let result = adapters::upsert_adapter(
        &mut store,
        adapter.into(),
        derived_from_adapter_id.as_deref(),
        Some("theseus_native"),
    )
    .map_err(py_thg_error)?;
    let dict = PyDict::new_bound(py);
    dict.set_item("node_id", result.node_id)?;
    dict.set_item("edge_count", result.edge_count)?;
    dict.set_item("graph_version", result.transaction.graph_version)?;
    dict.set_item("writes", result.transaction.writes.len())?;
    Ok(dict.into_py(py))
}

#[pyfunction]
pub fn get_adapter(adapter_id: String) -> PyResult<Option<LoraAdapter>> {
    for tenant in discover_tenants()? {
        let store = open_store_for_tenant(&tenant)?;
        if let Some(adapter) =
            adapters::find_adapter_by_id(&store, &adapter_id).map_err(py_thg_error)?
        {
            return Ok(Some(adapter.into()));
        }
    }
    Ok(None)
}

#[pyfunction]
#[pyo3(signature = (
    tenant_id,
    *,
    base_model_sha = None,
    min_fitness = None,
    include_superseded = false
))]
pub fn list_adapters(
    tenant_id: String,
    base_model_sha: Option<String>,
    min_fitness: Option<f32>,
    include_superseded: bool,
) -> PyResult<Vec<LoraAdapter>> {
    let store = open_store_for_tenant(&tenant_id)?;
    adapters::list_adapters(
        &store,
        adapters::AdapterListRequest {
            tenant_id,
            base_model_sha,
            min_fitness,
            include_superseded,
        },
    )
    .map(|items| items.into_iter().map(LoraAdapter::from).collect())
    .map_err(py_thg_error)
}

#[pyfunction]
#[pyo3(signature = (
    adapter_id,
    source_node_id,
    value,
    weight = 1.0,
    kind = "user_thumbs".to_string(),
    recorded_at_ms = None
))]
pub fn record_fitness(
    py: Python<'_>,
    adapter_id: String,
    source_node_id: String,
    value: f32,
    weight: f32,
    kind: String,
    recorded_at_ms: Option<i64>,
) -> PyResult<PyObject> {
    let tenant = tenant_for_adapter(&adapter_id)?;
    let mut store = open_store_for_tenant(&tenant)?;
    let result = adapters::record_fitness(
        &mut store,
        adapters::AdapterFitnessRecordRequest {
            adapter_id,
            source_node_id,
            value,
            weight,
            kind,
            recorded_at_ms,
        },
        Some("theseus_native"),
    )
    .map_err(py_thg_error)?;
    let dict = PyDict::new_bound(py);
    dict.set_item("edge_id", result.edge_id)?;
    dict.set_item("effective_fitness", result.effective_fitness)?;
    dict.set_item("graph_version", result.transaction.graph_version)?;
    Ok(dict.into_py(py))
}

#[pyfunction]
#[pyo3(signature = (old_adapter_id, new_adapter_id, archive_old = false))]
pub fn supersede_adapter(
    py: Python<'_>,
    old_adapter_id: String,
    new_adapter_id: String,
    archive_old: bool,
) -> PyResult<PyObject> {
    let tenant = tenant_for_adapter(&old_adapter_id)?;
    let mut store = open_store_for_tenant(&tenant)?;
    let result = adapters::supersede_adapter(
        &mut store,
        &old_adapter_id,
        &new_adapter_id,
        archive_old,
        Some("theseus_native"),
    )
    .map_err(py_thg_error)?;
    let dict = PyDict::new_bound(py);
    dict.set_item("edge_id", result.edge_id)?;
    dict.set_item("graph_version", result.transaction.graph_version)?;
    Ok(dict.into_py(py))
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    parent.add("ThgError", py.get_type_bound::<ThgErrorPy>())?;
    let module = PyModule::new_bound(py, "adapters")?;
    module.add_class::<LoraAdapter>()?;
    module.add_class::<AdapterRef>()?;
    module.add_function(wrap_pyfunction!(find_adapters, &module)?)?;
    module.add_function(wrap_pyfunction!(upsert_adapter, &module)?)?;
    module.add_function(wrap_pyfunction!(get_adapter, &module)?)?;
    module.add_function(wrap_pyfunction!(list_adapters, &module)?)?;
    module.add_function(wrap_pyfunction!(record_fitness, &module)?)?;
    module.add_function(wrap_pyfunction!(supersede_adapter, &module)?)?;
    module.add("ThgError", py.get_type_bound::<ThgErrorPy>())?;
    parent.add_submodule(&module)?;
    py.import_bound("sys")?
        .getattr("modules")?
        .set_item("theseus_native.adapters", module)?;
    Ok(())
}

impl From<LoraAdapter> for adapters::LoraAdapter {
    fn from(value: LoraAdapter) -> Self {
        Self {
            adapter_id: value.adapter_id,
            tenant_id: value.tenant_id,
            base_model_sha: value.base_model_sha,
            rank: value.rank,
            target_modules: value.target_modules,
            s3_uri: value.s3_uri,
            training_object_ids: value.training_object_ids,
            version: value.version,
            fitness: value.fitness,
            created_at_ms: value.created_at_ms,
            manifest_version: value.manifest_version,
        }
    }
}

impl From<adapters::LoraAdapter> for LoraAdapter {
    fn from(value: adapters::LoraAdapter) -> Self {
        Self {
            adapter_id: value.adapter_id,
            tenant_id: value.tenant_id,
            base_model_sha: value.base_model_sha,
            rank: value.rank,
            target_modules: value.target_modules,
            s3_uri: value.s3_uri,
            training_object_ids: value.training_object_ids,
            version: value.version,
            fitness: value.fitness,
            created_at_ms: value.created_at_ms,
            manifest_version: value.manifest_version,
        }
    }
}

impl From<adapters::AdapterRef> for AdapterRef {
    fn from(value: adapters::AdapterRef) -> Self {
        Self {
            adapter: value.adapter.into(),
            score: value.score,
        }
    }
}

fn open_store_for_tenant(tenant_id: &str) -> PyResult<RedCoreGraphStore> {
    AdapterStoreConfig::from_env()
        .open_store_for_tenant(tenant_id)
        .map_err(py_thg_error)
}

fn tenant_for_adapter(adapter_id: &str) -> PyResult<String> {
    let config = AdapterStoreConfig::from_env();
    for tenant in config.discover_tenants().map_err(py_thg_error)? {
        let store = config
            .open_store_for_tenant(&tenant)
            .map_err(py_thg_error)?;
        if adapters::find_adapter_by_id(&store, adapter_id)
            .map_err(py_thg_error)?
            .is_some()
        {
            return Ok(tenant);
        }
    }
    Err(py_thg_error(ThgError::new(
        "adapter_not_found",
        "adapter_id not found",
    )))
}

fn discover_tenants() -> PyResult<Vec<String>> {
    AdapterStoreConfig::from_env()
        .discover_tenants()
        .map_err(py_thg_error)
}

fn py_thg_error(error: ThgError) -> PyErr {
    PyErr::new::<ThgErrorPy, _>(format!("{}: {}", error.code, error.message))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{first_non_empty_env_value, AdapterStoreConfig};

    static TEST_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    struct EnvScope {
        snapshot: Vec<(String, Option<String>)>,
    }

    impl EnvScope {
        fn new(values: &[(&str, Option<&str>)]) -> Self {
            let mut seen = HashSet::new();
            let mut snapshot = Vec::new();
            for (key, value) in values {
                let key = key.to_string();
                if seen.insert(key.clone()) {
                    snapshot.push((key.clone(), env::var(&key).ok()));
                }
                match value {
                    Some(value) => env::set_var(&key, value),
                    None => env::remove_var(&key),
                }
            }
            Self { snapshot }
        }
    }

    impl Drop for EnvScope {
        fn drop(&mut self) {
            for (key, previous) in self.snapshot.drain(..) {
                match previous {
                    Some(previous) => env::set_var(&key, previous),
                    None => env::remove_var(key),
                }
            }
        }
    }

    fn set_adapter_env(values: &[(&str, Option<&str>)]) -> EnvScope {
        let mut vars = vec![
            ("THEOREM_HARNESS_DATA_DIR", None),
            ("THEOREM_DATA_DIR", None),
            ("RUSTYRED_THG_ADAPTER_DATA_DIR", None),
            ("RUSTY_RED_DATA_DIR", None),
            ("RUSTYRED_THG_PRODUCT_DATA_DIR", None),
            ("THEOREM_TENANT_SLUG", None),
            ("RUSTYRED_THG_TENANT_SLUG", None),
            ("THEOREM_TENANT_ID", None),
            ("THEOREM_HARNESS_TENANT_SLUG", None),
            ("THEOREM_AGENT_TENANT_SLUG", None),
            ("RUSTYRED_THG_ADAPTER_DEFAULT_TENANT", None),
            ("RUSTY_RED_MCP_DEFAULT_TENANT", None),
            ("RUSTYRED_THG_MCP_DEFAULT_TENANT", None),
        ];
        vars.extend_from_slice(values);
        EnvScope::new(&vars)
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}"))
    }

    #[test]
    fn adapter_store_config_prefers_current_tenant_slug_env() {
        let _guard = env_lock();
        let _scope = set_adapter_env(&[
            ("THEOREM_TENANT_SLUG", Some("CurrentTenant")),
            ("RUSTYRED_THG_TENANT_SLUG", Some("LegacyTenant")),
            ("RUSTYRED_THG_ADAPTER_DEFAULT_TENANT", Some("AdapterTenant")),
        ]);

        let config = AdapterStoreConfig::from_env();
        assert_eq!(config.default_tenant, "CurrentTenant");
    }

    #[test]
    fn adapter_store_config_prefers_legacy_tenant_fallbacks() {
        let _guard = env_lock();
        let _scope = set_adapter_env(&[
            ("RUSTYRED_THG_TENANT_SLUG", Some("LegacyTenant")),
            ("RUSTYRED_THG_ADAPTER_DEFAULT_TENANT", Some("AdapterTenant")),
            ("RUSTY_RED_MCP_DEFAULT_TENANT", Some("McpTenant")),
        ]);

        let config = AdapterStoreConfig::from_env();
        assert_eq!(config.default_tenant, "LegacyTenant");
    }

    #[test]
    fn adapter_store_config_falls_back_to_default_tenant() {
        let _guard = env_lock();
        let _scope = set_adapter_env(&[]);

        let config = AdapterStoreConfig::from_env();
        assert_eq!(config.default_tenant, "default");
    }

    #[test]
    fn adapter_store_config_prefers_current_data_dir_env() {
        let _guard = env_lock();
        let _scope = set_adapter_env(&[
            ("THEOREM_HARNESS_DATA_DIR", Some("/tmp/harness-root")),
            ("THEOREM_DATA_DIR", Some("/tmp/theorem-data")),
            ("RUSTYRED_THG_ADAPTER_DATA_DIR", Some("/tmp/legacy")),
        ]);

        let config = AdapterStoreConfig::from_env();
        assert_eq!(config.base_data_dir, PathBuf::from("/tmp/harness-root"));
    }

    #[test]
    fn adapter_store_config_falls_back_to_legacy_data_dir() {
        let _guard = env_lock();
        let _scope = set_adapter_env(&[
            ("RUSTY_RED_DATA_DIR", Some("/tmp/rusty-red-data")),
            ("RUSTYRED_THG_PRODUCT_DATA_DIR", Some("/tmp/product-data")),
        ]);

        let config = AdapterStoreConfig::from_env();
        assert_eq!(config.base_data_dir, PathBuf::from("/tmp/rusty-red-data"));
    }

    #[test]
    fn adapter_store_config_uses_default_data_dir_when_unset() {
        let _guard = env_lock();
        let _scope = set_adapter_env(&[]);
        let config = AdapterStoreConfig::from_env();
        assert_eq!(config.base_data_dir, PathBuf::from("data/rusty-red"));
    }

    #[test]
    fn adapter_store_discover_tenants_reads_subdirs_only_and_falls_back_to_default() {
        let tenants_root = unique_temp_dir("rustyredcore-tenant-discovery");
        fs::create_dir_all(tenants_root.join("tenants").join("tenant-a")).unwrap();
        fs::create_dir_all(tenants_root.join("tenants").join("tenant-b")).unwrap();
        fs::write(tenants_root.join("tenants").join("not-a-dir"), "content").unwrap();

        let config = AdapterStoreConfig {
            base_data_dir: tenants_root,
            default_tenant: "FallbackTenant".to_string(),
        };
        let discovered = config.discover_tenants().unwrap();
        let discovered: HashSet<String> = discovered.into_iter().collect();
        assert_eq!(
            discovered,
            ["tenant-a", "tenant-b"]
                .into_iter()
                .map(|tenant| tenant.to_string())
                .collect::<HashSet<_>>()
        );

        let empty_root = unique_temp_dir("rustyredcore-tenant-discovery-empty");
        fs::create_dir_all(empty_root.join("tenants")).unwrap();
        let fallback_config = AdapterStoreConfig {
            base_data_dir: empty_root,
            default_tenant: "FallbackTenant".to_string(),
        };
        assert_eq!(
            fallback_config.discover_tenants().unwrap(),
            vec!["FallbackTenant".to_string()]
        );

        let _ = fs::remove_dir_all(&config.base_data_dir);
        let _ = fs::remove_dir_all(&fallback_config.base_data_dir);
    }

    #[test]
    fn adapter_store_discover_tenants_surfaces_missing_tenants_root() {
        let root = unique_temp_dir("rustyredcore-tenant-discovery-missing");
        let config = AdapterStoreConfig {
            base_data_dir: root,
            default_tenant: "FallbackTenant".to_string(),
        };
        let error = config.discover_tenants().unwrap_err();
        assert_eq!(error.code, "adapter_tenant_discovery_failed");
        assert!(error.message.contains("unable to read adapter tenants"));

        let _ = fs::remove_dir_all(&config.base_data_dir);
    }

    #[test]
    fn first_non_empty_env_value_prefers_first_non_empty_and_ignores_blank() {
        let _guard = env_lock();
        let key_a = "RUSTYRED_THG_DATA_DIR_TEST_A";
        let key_b = "RUSTYRED_THG_DATA_DIR_TEST_B";
        let _scope = EnvScope::new(&[(key_a, Some("  ")), (key_b, Some("/tmp/actual"))]);

        let value = first_non_empty_env_value(&[key_a, key_b]).unwrap();
        assert_eq!(value, "/tmp/actual");
    }

    #[test]
    fn first_non_empty_env_value_none_when_all_missing() {
        let _guard = env_lock();
        let _scope = EnvScope::new(&[("NO_SUCH_KEY_A", None), ("NO_SUCH_KEY_B", None)]);
        let value = first_non_empty_env_value(&["NO_SUCH_KEY_A", "NO_SUCH_KEY_B"]);
        assert!(value.is_none());
    }
}
