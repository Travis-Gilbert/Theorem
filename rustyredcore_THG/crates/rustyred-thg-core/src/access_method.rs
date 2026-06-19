use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::graph_store::{Direction, GraphStoreError, GraphStoreResult};
use crate::ordered::{OrderedIndex, OrderedMode};

pub type RelationId = String;
pub type ColumnId = String;
pub type RowId = String;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", untagged)]
pub enum ScalarValue {
    String(String),
    I64(i64),
    F64(f64),
    Bool(bool),
}

impl ScalarValue {
    pub fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::String(value) => Some(Self::String(value.clone())),
            Value::Number(value) => value
                .as_i64()
                .map(Self::I64)
                .or_else(|| value.as_f64().map(Self::F64)),
            Value::Bool(value) => Some(Self::Bool(*value)),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::I64(value) => Some(*value as f64),
            Self::F64(value) if value.is_finite() => Some(*value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I64(value) => Some(*value),
            Self::F64(value) if value.is_finite() => Some(*value as i64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }
}

impl Eq for ScalarValue {}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ScalarBound {
    Unbounded,
    Included(ScalarValue),
    Excluded(ScalarValue),
}

impl ScalarBound {
    pub fn included(value: ScalarValue) -> Self {
        Self::Included(value)
    }

    pub fn unbounded() -> Self {
        Self::Unbounded
    }

    fn numeric_floor(&self) -> Option<f64> {
        match self {
            Self::Unbounded => Some(f64::NEG_INFINITY),
            Self::Included(value) | Self::Excluded(value) => value.as_f64(),
        }
    }

    fn numeric_ceil(&self) -> Option<f64> {
        match self {
            Self::Unbounded => Some(f64::INFINITY),
            Self::Included(value) | Self::Excluded(value) => value.as_f64(),
        }
    }

    pub fn contains_numeric(&self, value: f64, lower: bool) -> bool {
        match self {
            Self::Unbounded => true,
            Self::Included(bound) => bound
                .as_f64()
                .map(|bound| {
                    if lower {
                        value >= bound
                    } else {
                        value <= bound
                    }
                })
                .unwrap_or(false),
            Self::Excluded(bound) => bound
                .as_f64()
                .map(|bound| if lower { value > bound } else { value < bound })
                .unwrap_or(false),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Predicate {
    Equals {
        column: ColumnId,
        value: ScalarValue,
    },
    Range {
        column: ColumnId,
        lo: ScalarBound,
        hi: ScalarBound,
    },
    Prefix {
        column: ColumnId,
        prefix: String,
    },
    Knn {
        column: ColumnId,
        query: Vec<f32>,
        k: usize,
    },
    GeoWithin {
        column: ColumnId,
        region: RegionRef,
    },
    TimeRange {
        column: ColumnId,
        lo_ms: i64,
        hi_ms: i64,
    },
    TextMatch {
        column: ColumnId,
        query: String,
    },
    Expand {
        from: RowId,
        edge_type: String,
        dir: Direction,
    },
}

impl Predicate {
    pub fn column(&self) -> Option<&str> {
        match self {
            Self::Equals { column, .. }
            | Self::Range { column, .. }
            | Self::Prefix { column, .. }
            | Self::Knn { column, .. }
            | Self::GeoWithin { column, .. }
            | Self::TimeRange { column, .. }
            | Self::TextMatch { column, .. } => Some(column),
            Self::Expand { .. } => None,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Equals { .. } => "equals",
            Self::Range { .. } => "range",
            Self::Prefix { .. } => "prefix",
            Self::Knn { .. } => "knn",
            Self::GeoWithin { .. } => "geo_within",
            Self::TimeRange { .. } => "time_range",
            Self::TextMatch { .. } => "text_match",
            Self::Expand { .. } => "expand",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RegionRef {
    Bbox {
        min_lat: f64,
        min_lon: f64,
        max_lat: f64,
        max_lon: f64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Cost {
    pub est_rows: f64,
    pub est_work: f64,
}

impl Cost {
    pub fn new(est_rows: f64, est_work: f64) -> Self {
        Self {
            est_rows: est_rows.max(0.0),
            est_work: est_work.max(0.0),
        }
    }

    pub fn rank_key(self) -> f64 {
        self.est_work + self.est_rows
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RowIdStream {
    pub rows: Vec<RowId>,
    pub visited: usize,
}

impl RowIdStream {
    pub fn new(rows: Vec<RowId>, visited: usize) -> Self {
        Self { rows, visited }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowChangeKind {
    Upsert,
    Delete,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RowChange {
    pub relation: RelationId,
    pub row_id: RowId,
    pub values: BTreeMap<ColumnId, ScalarValue>,
    pub kind: RowChangeKind,
}

pub type AmResult<T> = GraphStoreResult<T>;

pub trait AccessMethod: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, relation: &str, predicate: &Predicate) -> bool;
    fn cost(&self, relation: &str, predicate: &Predicate) -> Option<Cost>;
    fn scan(&self, relation: &str, predicate: &Predicate) -> AmResult<RowIdStream>;
    fn on_write(&self, change: &RowChange) -> AmResult<()>;
}

#[derive(Default)]
pub struct AccessMethodRegistry {
    methods: Vec<Box<dyn AccessMethod>>,
}

impl AccessMethodRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, method: impl AccessMethod + 'static) {
        self.methods.push(Box::new(method));
    }

    pub fn methods(&self) -> impl Iterator<Item = &dyn AccessMethod> {
        self.methods.iter().map(|method| method.as_ref())
    }

    pub fn on_write(&self, change: &RowChange) -> AmResult<()> {
        for method in &self.methods {
            method.on_write(change)?;
        }
        Ok(())
    }

    pub fn with_native_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(OrderedAccessMethod::new());
        registry.register(TimeSeriesAccessMethod::new());
        registry
    }
}

#[derive(Clone, Debug, Default)]
pub struct AccessMethodStats {
    pub scans: usize,
    pub rows_visited: usize,
}

#[derive(Clone, Debug, Default)]
pub struct OrderedAccessMethod {
    state: Arc<Mutex<OrderedState>>,
    stats: Arc<Mutex<AccessMethodStats>>,
}

#[derive(Clone, Debug, Default)]
struct OrderedState {
    numeric: BTreeMap<(RelationId, ColumnId), OrderedIndex>,
    text: BTreeMap<(RelationId, ColumnId), BTreeMap<String, BTreeSet<RowId>>>,
}

impl OrderedAccessMethod {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> AccessMethodStats {
        self.stats
            .lock()
            .map(|stats| stats.clone())
            .unwrap_or_default()
    }
}

impl AccessMethod for OrderedAccessMethod {
    fn name(&self) -> &'static str {
        "ordered"
    }

    fn supports(&self, _relation: &str, predicate: &Predicate) -> bool {
        matches!(
            predicate,
            Predicate::Equals { .. } | Predicate::Range { .. } | Predicate::Prefix { .. }
        )
    }

    fn cost(&self, relation: &str, predicate: &Predicate) -> Option<Cost> {
        if !self.supports(relation, predicate) {
            return None;
        }
        let state = self.state.lock().ok()?;
        match predicate {
            Predicate::Equals { column, value } if value.as_f64().is_some() => {
                let total = state
                    .numeric
                    .get(&(relation.to_string(), column.clone()))
                    .map(OrderedIndex::zcard)
                    .unwrap_or(0);
                Some(Cost::new(1.0, (total.max(1) as f64).log2() + 1.0))
            }
            Predicate::Equals { column, value } => {
                let rows = value
                    .as_str()
                    .and_then(|value| {
                        state
                            .text
                            .get(&(relation.to_string(), column.clone()))
                            .and_then(|index| index.get(value))
                    })
                    .map(BTreeSet::len)
                    .unwrap_or(0);
                Some(Cost::new(rows as f64, rows.max(1) as f64))
            }
            Predicate::Range { column, .. } => {
                let total = state
                    .numeric
                    .get(&(relation.to_string(), column.clone()))
                    .map(OrderedIndex::zcard)
                    .unwrap_or(0);
                Some(Cost::new(
                    (total as f64 * 0.25).max(1.0),
                    total.max(1) as f64 * 0.25,
                ))
            }
            Predicate::Prefix { column, prefix } => {
                let total = state
                    .text
                    .get(&(relation.to_string(), column.clone()))
                    .map(BTreeMap::len)
                    .unwrap_or(0);
                Some(Cost::new(
                    (total as f64 * 0.1).max(1.0),
                    prefix.len().max(1) as f64,
                ))
            }
            _ => None,
        }
    }

    fn scan(&self, relation: &str, predicate: &Predicate) -> AmResult<RowIdStream> {
        let state = self.state.lock().map_err(poisoned)?;
        let mut visited = 0usize;
        let rows = match predicate {
            Predicate::Equals { column, value } if value.as_f64().is_some() => {
                let score = value.as_f64().unwrap();
                state
                    .numeric
                    .get(&(relation.to_string(), column.clone()))
                    .map(|index| {
                        index
                            .zrange_by_score(score, score, None)
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|(member, _)| String::from_utf8(member).ok())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            }
            Predicate::Equals { column, value } => value
                .as_str()
                .and_then(|value| {
                    state
                        .text
                        .get(&(relation.to_string(), column.clone()))
                        .and_then(|index| index.get(value))
                })
                .map(|rows| rows.iter().cloned().collect())
                .unwrap_or_default(),
            Predicate::Range { column, lo, hi } => {
                let min = lo.numeric_floor().ok_or_else(|| {
                    GraphStoreError::new(
                        "invalid_range_predicate",
                        "range lower bound is not numeric",
                    )
                })?;
                let max = hi.numeric_ceil().ok_or_else(|| {
                    GraphStoreError::new(
                        "invalid_range_predicate",
                        "range upper bound is not numeric",
                    )
                })?;
                let hits = state
                    .numeric
                    .get(&(relation.to_string(), column.clone()))
                    .map(|index| index.zrange_by_score(min, max, None))
                    .transpose()?
                    .unwrap_or_default();
                visited = hits.len();
                hits.into_iter()
                    .filter(|(_, score)| {
                        lo.contains_numeric(*score, true) && hi.contains_numeric(*score, false)
                    })
                    .filter_map(|(member, _)| String::from_utf8(member).ok())
                    .collect()
            }
            Predicate::Prefix { column, prefix } => {
                let Some(index) = state.text.get(&(relation.to_string(), column.clone())) else {
                    return Ok(RowIdStream::new(Vec::new(), 0));
                };
                let mut out = Vec::new();
                for (value, row_ids) in index.range(prefix.clone()..) {
                    visited += 1;
                    if !value.starts_with(prefix) {
                        break;
                    }
                    out.extend(row_ids.iter().cloned());
                }
                out
            }
            _ => Vec::new(),
        };
        let visited = visited.max(rows.len());
        if let Ok(mut stats) = self.stats.lock() {
            stats.scans += 1;
            stats.rows_visited += visited;
        }
        Ok(RowIdStream::new(rows, visited))
    }

    fn on_write(&self, change: &RowChange) -> AmResult<()> {
        let mut state = self.state.lock().map_err(poisoned)?;
        if change.kind == RowChangeKind::Delete {
            for index in state.numeric.values_mut() {
                index.zrem(change.row_id.as_bytes());
            }
            for index in state.text.values_mut() {
                for rows in index.values_mut() {
                    rows.remove(&change.row_id);
                }
            }
            return Ok(());
        }
        for (column, value) in &change.values {
            if let Some(score) = value.as_f64() {
                state
                    .numeric
                    .entry((change.relation.clone(), column.clone()))
                    .or_insert_with(|| OrderedIndex::new(OrderedMode::Persistent))
                    .zadd(change.row_id.as_bytes().to_vec(), score)?;
            }
            if let Some(value) = value.as_str() {
                state
                    .text
                    .entry((change.relation.clone(), column.clone()))
                    .or_default()
                    .entry(value.to_string())
                    .or_default()
                    .insert(change.row_id.clone());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct TimeSeriesAccessMethod {
    state: Arc<Mutex<BTreeMap<(RelationId, ColumnId), OrderedIndex>>>,
    stats: Arc<Mutex<AccessMethodStats>>,
}

impl TimeSeriesAccessMethod {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> AccessMethodStats {
        self.stats
            .lock()
            .map(|stats| stats.clone())
            .unwrap_or_default()
    }
}

impl AccessMethod for TimeSeriesAccessMethod {
    fn name(&self) -> &'static str {
        "time_series"
    }

    fn supports(&self, _relation: &str, predicate: &Predicate) -> bool {
        matches!(predicate, Predicate::TimeRange { .. })
    }

    fn cost(&self, relation: &str, predicate: &Predicate) -> Option<Cost> {
        let Predicate::TimeRange { column, .. } = predicate else {
            return None;
        };
        let total = self
            .state
            .lock()
            .ok()?
            .get(&(relation.to_string(), column.clone()))
            .map(OrderedIndex::zcard)
            .unwrap_or(0);
        Some(Cost::new(
            (total as f64 * 0.2).max(1.0),
            total.max(1) as f64 * 0.2,
        ))
    }

    fn scan(&self, relation: &str, predicate: &Predicate) -> AmResult<RowIdStream> {
        let Predicate::TimeRange {
            column,
            lo_ms,
            hi_ms,
        } = predicate
        else {
            return Ok(RowIdStream::new(Vec::new(), 0));
        };
        let state = self.state.lock().map_err(poisoned)?;
        let hits = state
            .get(&(relation.to_string(), column.clone()))
            .map(|index| index.zrange_by_score(*lo_ms as f64, *hi_ms as f64, None))
            .transpose()?
            .unwrap_or_default();
        let rows = hits
            .iter()
            .filter_map(|(member, _)| String::from_utf8(member.clone()).ok())
            .collect::<Vec<_>>();
        if let Ok(mut stats) = self.stats.lock() {
            stats.scans += 1;
            stats.rows_visited += hits.len();
        }
        Ok(RowIdStream::new(rows, hits.len()))
    }

    fn on_write(&self, change: &RowChange) -> AmResult<()> {
        let mut state = self.state.lock().map_err(poisoned)?;
        if change.kind == RowChangeKind::Delete {
            for index in state.values_mut() {
                index.zrem(change.row_id.as_bytes());
            }
            return Ok(());
        }
        for (column, value) in &change.values {
            let Some(timestamp) = value.as_i64() else {
                continue;
            };
            state
                .entry((change.relation.clone(), column.clone()))
                .or_insert_with(|| OrderedIndex::new(OrderedMode::Persistent))
                .zadd(change.row_id.as_bytes().to_vec(), timestamp as f64)?;
        }
        Ok(())
    }
}

fn poisoned<T>(_: T) -> GraphStoreError {
    GraphStoreError::new(
        "access_method_poisoned",
        "access method mutex was poisoned".to_string(),
    )
}
