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
    for tenant in discover_tenants() {
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
    RedCoreGraphStore::open(
        tenant_data_dir(tenant_id),
        RedCoreOptions {
            durability: RedCoreDurability::AofEverysec,
            snapshot_interval_writes: 1_000,
            strict_acid: false,
        },
    )
    .map_err(|error| py_thg_error(ThgError::new(error.code, error.message)))
}

fn tenant_for_adapter(adapter_id: &str) -> PyResult<String> {
    for tenant in discover_tenants() {
        let store = open_store_for_tenant(&tenant)?;
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

fn discover_tenants() -> Vec<String> {
    let tenants_root = base_data_dir().join("tenants");
    let mut tenants = std::fs::read_dir(&tenants_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .and_then(|_| entry.file_name().into_string().ok())
        })
        .collect::<Vec<_>>();
    if tenants.is_empty() {
        tenants.push(default_tenant());
    }
    tenants
}

fn tenant_data_dir(tenant_id: &str) -> PathBuf {
    let safe_tenant = sanitize_tenant_segment(tenant_id);
    base_data_dir().join("tenants").join(safe_tenant)
}

fn base_data_dir() -> PathBuf {
    env::var("RUSTYRED_THG_ADAPTER_DATA_DIR")
        .or_else(|_| env::var("RUSTY_RED_DATA_DIR"))
        .or_else(|_| env::var("RUSTYRED_THG_PRODUCT_DATA_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/rusty-red"))
}

fn default_tenant() -> String {
    env::var("RUSTYRED_THG_ADAPTER_DEFAULT_TENANT").unwrap_or_else(|_| "default".to_string())
}

fn py_thg_error(error: ThgError) -> PyErr {
    PyErr::new::<ThgErrorPy, _>(format!("{}: {}", error.code, error.message))
}
