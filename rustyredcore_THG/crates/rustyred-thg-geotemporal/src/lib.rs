use std::collections::{BTreeSet, HashMap};

use rustyred_thg_core::{NodeRecord, SpatialDesignation, SpatialIndex, TimeInterval};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type NodeId = String;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TenantSpatialKey {
    tenant_id: String,
    designation: SpatialDesignation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpatialPredicate {
    RadiusKm {
        lat: f64,
        lon: f64,
        radius_km: f64,
    },
    Bbox {
        min_lat: f64,
        min_lon: f64,
        max_lat: f64,
        max_lon: f64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GeoTemporalQuery {
    pub tenant_id: String,
    pub designation: SpatialDesignation,
    pub spatial: SpatialPredicate,
    pub at_ms: Option<i64>,
    pub interval: Option<TimeInterval>,
    pub required_label: Option<String>,
    pub limit: usize,
}

#[derive(Default)]
pub struct GeoTemporalIndex {
    indexes: HashMap<TenantSpatialKey, SpatialIndex>,
    intervals: HashMap<(String, NodeId), TimeInterval>,
    labels: HashMap<(String, NodeId), BTreeSet<String>>,
}

impl GeoTemporalIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_node(
        &mut self,
        tenant_id: &str,
        designation: SpatialDesignation,
        node: &NodeRecord,
    ) -> Result<(), GeoTemporalError> {
        validate_tenant(tenant_id)?;
        let lat = numeric_property(&node.properties, &designation.lat_property)
            .ok_or_else(|| GeoTemporalError::MissingProperty(designation.lat_property.clone()))?;
        let lon = numeric_property(&node.properties, &designation.lon_property)
            .ok_or_else(|| GeoTemporalError::MissingProperty(designation.lon_property.clone()))?;

        let key = TenantSpatialKey {
            tenant_id: tenant_id.to_string(),
            designation: designation.clone(),
        };
        self.indexes
            .entry(key)
            .or_insert_with(|| SpatialIndex::for_designation(designation))
            .upsert(&node.id, lat, lon)
            .map_err(|err| GeoTemporalError::Spatial(format!("{err:?}")))?;

        if let Some(interval) = rustyred_thg_core::graph_store::node_time_interval(node) {
            self.intervals
                .insert((tenant_id.to_string(), node.id.clone()), interval);
        }

        self.labels.insert(
            (tenant_id.to_string(), node.id.clone()),
            node.labels.iter().cloned().collect(),
        );
        Ok(())
    }

    pub fn execute(&self, query: GeoTemporalQuery) -> Result<Vec<NodeId>, GeoTemporalError> {
        validate_tenant(&query.tenant_id)?;
        let key = TenantSpatialKey {
            tenant_id: query.tenant_id.clone(),
            designation: query.designation.clone(),
        };
        let Some(index) = self.indexes.get(&key) else {
            return Ok(Vec::new());
        };

        let mut candidates = match query.spatial {
            SpatialPredicate::RadiusKm {
                lat,
                lon,
                radius_km,
            } => index
                .radius_search(lat, lon, radius_km)
                .map_err(|err| GeoTemporalError::Spatial(format!("{err:?}")))?,
            SpatialPredicate::Bbox {
                min_lat,
                min_lon,
                max_lat,
                max_lon,
            } => index.bbox_search(min_lat, min_lon, max_lat, max_lon),
        };
        candidates.sort();
        candidates.dedup();

        let mut results: Vec<NodeId> = Vec::new();
        for node_id in candidates {
            if !self.label_matches(&query.tenant_id, &node_id, query.required_label.as_deref()) {
                continue;
            }
            if !self.time_matches(&query.tenant_id, &node_id, query.at_ms, query.interval) {
                continue;
            }
            results.push(node_id);
            if query.limit > 0 && results.len() >= query.limit {
                break;
            }
        }

        Ok(results)
    }

    fn label_matches(&self, tenant_id: &str, node_id: &str, required_label: Option<&str>) -> bool {
        let Some(required_label) = required_label else {
            return true;
        };
        self.labels
            .get(&(tenant_id.to_string(), node_id.to_string()))
            .map(|labels| labels.contains(required_label))
            .unwrap_or(false)
    }

    fn time_matches(
        &self,
        tenant_id: &str,
        node_id: &str,
        at_ms: Option<i64>,
        interval: Option<TimeInterval>,
    ) -> bool {
        let Some(node_interval) = self
            .intervals
            .get(&(tenant_id.to_string(), node_id.to_string()))
            .copied()
        else {
            return at_ms.is_none() && interval.is_none();
        };
        if let Some(at_ms) = at_ms {
            return node_interval.contains_ms(at_ms);
        }
        if let Some(interval) = interval {
            return node_interval.overlaps(interval);
        }
        true
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum GeoTemporalError {
    MissingTenant,
    MissingProperty(String),
    Spatial(String),
}

impl std::fmt::Display for GeoTemporalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingTenant => write!(f, "tenant_id is required"),
            Self::MissingProperty(property) => write!(f, "{property} is required"),
            Self::Spatial(message) => write!(f, "spatial index error: {message}"),
        }
    }
}

impl std::error::Error for GeoTemporalError {}

fn validate_tenant(tenant_id: &str) -> Result<(), GeoTemporalError> {
    if tenant_id.trim().is_empty() {
        return Err(GeoTemporalError::MissingTenant);
    }
    Ok(())
}

fn numeric_property(properties: &Value, key: &str) -> Option<f64> {
    properties.get(key).and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|raw| raw.parse::<f64>().ok()))
    })
}

#[cfg(test)]
mod tests {
    use rustyred_thg_core::{GraphStore, InMemoryGraphStore};
    use serde_json::json;

    use super::*;

    fn designation() -> SpatialDesignation {
        SpatialDesignation {
            label: "BuildingPresence".to_string(),
            lat_property: "lat".to_string(),
            lon_property: "lon".to_string(),
            resolution: 11,
        }
    }

    fn building(
        id: &str,
        lat: f64,
        lon: f64,
        start: i64,
        end: i64,
        anchors: &[&str],
    ) -> NodeRecord {
        NodeRecord::new(
            id,
            ["BuildingPresence"],
            json!({
                "lat": lat,
                "lon": lon,
                "t_start_ms": start,
                "t_end_ms": end,
                "photo_anchor_ids": anchors,
            }),
        )
    }

    #[test]
    fn graph_store_reads_standard_node_interval() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(building("b1", 43.019, -83.699, 0, 10, &[]))
            .unwrap();

        assert_eq!(
            store.get_node_interval("b1"),
            Some(TimeInterval {
                start_ms: Some(0),
                end_ms: Some(10)
            })
        );
    }

    #[test]
    fn geotemporal_query_returns_flint_1925_block_nodes_only() {
        let mut index = GeoTemporalIndex::new();
        for i in 0..5 {
            let anchors = if i < 2 { vec!["photo:ct"] } else { Vec::new() };
            index
                .upsert_node(
                    "flint",
                    designation(),
                    &building(
                        &format!("building:ct:{i}"),
                        43.019 + (i as f64 * 0.0001),
                        -83.699,
                        -1_420_070_400_000,
                        -946_684_800_000,
                        &anchors,
                    ),
                )
                .unwrap();
        }
        index
            .upsert_node(
                "test-city",
                designation(),
                &building(
                    "building:test:1",
                    43.019,
                    -83.699,
                    -1_420_070_400_000,
                    -946_684_800_000,
                    &[],
                ),
            )
            .unwrap();

        let results = index
            .execute(GeoTemporalQuery {
                tenant_id: "flint".to_string(),
                designation: designation(),
                spatial: SpatialPredicate::RadiusKm {
                    lat: 43.019,
                    lon: -83.699,
                    radius_km: 0.5,
                },
                at_ms: Some(-1_420_070_400_000),
                interval: None,
                required_label: Some("BuildingPresence".to_string()),
                limit: 10,
            })
            .unwrap();

        assert_eq!(results.len(), 5);
        assert!(results.iter().all(|id| id.starts_with("building:ct:")));
    }

    #[test]
    fn outside_time_slice_returns_empty_result() {
        let mut index = GeoTemporalIndex::new();
        index
            .upsert_node(
                "flint",
                designation(),
                &building(
                    "building:ct:outside",
                    43.019,
                    -83.699,
                    -1_420_070_400_000,
                    -946_684_800_000,
                    &[],
                ),
            )
            .unwrap();

        let results = index
            .execute(GeoTemporalQuery {
                tenant_id: "flint".to_string(),
                designation: designation(),
                spatial: SpatialPredicate::RadiusKm {
                    lat: 43.019,
                    lon: -83.699,
                    radius_km: 0.5,
                },
                at_ms: Some(1_735_689_600_000),
                interval: None,
                required_label: Some("BuildingPresence".to_string()),
                limit: 10,
            })
            .unwrap();

        assert!(results.is_empty());
    }
}
