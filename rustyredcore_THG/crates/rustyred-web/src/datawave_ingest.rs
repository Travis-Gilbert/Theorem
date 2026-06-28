//! DATAWAVE bridge for RustyWeb crawl output.
//!
//! This turns fetched crawl pages into `rustyred-thg-datawave` records so web
//! scrape output lands in the same field-fact corpus as source intake,
//! reconstructed binaries, and repository records.

use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::{
    execute_query, GraphSnapshot, GraphStore, GraphStoreError, GraphStoreResult, JoinPredicate,
    NodeRecord, Predicate, Projection, QueryIr, QueryRelation, RelationalStore, ScalarValue,
};
use rustyred_thg_datawave::{
    DatawaveError, DatawaveIngest, EdgeDef, FieldConfig, FieldType, IndexPolicy, IngestStats,
    JsonHelper, MaterializeConfig, RawRecord, FIELD_FACT_LABEL,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{CrawlGraph, EDGE_HAS_SNAPSHOT, EDGE_RESULTED_IN, LABEL_PAGE};

pub const RUSTYWEB_PAGE_DATA_TYPE: &str = "rustyweb_page";
const DEFAULT_RUSTYWEB_QUERY_LIMIT: u64 = 20;
const MAX_RUSTYWEB_QUERY_LIMIT: u64 = 100;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CrawlDatawaveIngestReport {
    pub data_type: String,
    pub records_seen: usize,
    pub ingested: usize,
    pub deduped: usize,
    pub skipped: usize,
    pub fields_written: usize,
    pub edges_written: usize,
    pub dictionary_nodes_written: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RustyWebDatawaveQueryPredicate {
    pub field: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RustyWebDatawaveQueryInput {
    pub predicates: Vec<RustyWebDatawaveQueryPredicate>,
    pub limit: usize,
}

pub fn rustyweb_datawave_field_config() -> FieldConfig {
    FieldConfig::new()
        .with_default_type(FieldType::LcText)
        .with_default_policy(IndexPolicy::INDEXED)
        .with_field("page_id", FieldType::NoOp, IndexPolicy::INDEXED)
        .with_field("snapshot_id", FieldType::NoOp, IndexPolicy::INDEXED)
        .with_field("run_id", FieldType::NoOp, IndexPolicy::INDEXED)
        .with_field("url", FieldType::Text, IndexPolicy::INDEXED.with_reverse())
        .with_field(
            "canonical_url",
            FieldType::Text,
            IndexPolicy::INDEXED.with_reverse(),
        )
        .with_field("domain", FieldType::LcText, IndexPolicy::INDEXED)
        .with_field("status", FieldType::Number, IndexPolicy::INDEXED)
        .with_field("content_type", FieldType::LcText, IndexPolicy::INDEXED)
        .with_field("content_hash", FieldType::NoOp, IndexPolicy::INDEXED)
        .with_field("source_class", FieldType::LcText, IndexPolicy::INDEXED)
        .with_field("page_state", FieldType::LcText, IndexPolicy::INDEXED)
        .with_field("namespace", FieldType::LcText, IndexPolicy::INDEXED)
        .with_field("body_bytes", FieldType::Number, IndexPolicy::INDEXED)
        .with_field("byte_len", FieldType::Number, IndexPolicy::INDEXED)
        .with_field(
            "text",
            FieldType::LcText,
            IndexPolicy::INDEXED.with_tokenized(),
        )
}

pub fn rustyweb_datawave_ingest(tenant_id: Option<String>) -> DatawaveIngest {
    let mut config = MaterializeConfig::new(tenant_id).with_generation(0);
    config.source = "rustyred-web:datawave".to_string();
    let mut ingest = DatawaveIngest::new(config);
    ingest.register(Box::new(JsonHelper::new(
        RUSTYWEB_PAGE_DATA_TYPE,
        rustyweb_datawave_field_config(),
    )));
    ingest
        .with_edge(EdgeDef::new("PAGE_ON_DOMAIN", "url", "domain"))
        .with_edge(EdgeDef::new("PAGE_HAS_CONTENT", "url", "content_hash"))
        .with_edge(EdgeDef::new("PAGE_IN_RUN", "url", "run_id"));
    ingest
}

pub fn datawave_records_from_crawl_graph(graph: &CrawlGraph) -> Vec<RawRecord> {
    let nodes = graph.nodes();
    let edges = graph.edges();
    let node_by_id = nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let mut attempt_by_page: BTreeMap<String, &NodeRecord> = BTreeMap::new();
    let mut snapshot_by_page: BTreeMap<String, &NodeRecord> = BTreeMap::new();

    for edge in &edges {
        if edge.edge_type == EDGE_RESULTED_IN {
            if let Some(attempt) = node_by_id.get(&edge.from_id) {
                attempt_by_page.insert(edge.to_id.clone(), *attempt);
            }
        } else if edge.edge_type == EDGE_HAS_SNAPSHOT {
            if let Some(snapshot) = node_by_id.get(&edge.to_id) {
                snapshot_by_page.insert(edge.from_id.clone(), *snapshot);
            }
        }
    }

    let mut records = Vec::new();
    for page in nodes.iter().filter(|node| has_label(node, LABEL_PAGE)) {
        let page_state = property_string(&page.properties, "page_state").unwrap_or_default();
        if page_state != "fetched" {
            continue;
        }
        let Some(url) = property_string(&page.properties, "url")
            .or_else(|| property_string(&page.properties, "canonical_url"))
        else {
            continue;
        };
        let attempt = attempt_by_page.get(&page.id).copied();
        let snapshot = snapshot_by_page.get(&page.id).copied();
        let body = json!({
            "page_id": page.id.clone(),
            "snapshot_id": snapshot.map(|node| node.id.clone()),
            "run_id": graph.run_id.clone(),
            "namespace": graph.namespace.clone(),
            "url": url.clone(),
            "canonical_url": property_string(&page.properties, "canonical_url"),
            "domain": property_string(&page.properties, "domain"),
            "page_state": page_state.clone(),
            "source_class": property_string(&page.properties, "source_class"),
            "status": attempt.and_then(|node| property_string(&node.properties, "status")),
            "content_type": attempt.and_then(|node| property_string(&node.properties, "content_type")),
            "body_bytes": attempt.and_then(|node| property_string(&node.properties, "body_bytes")),
            "content_hash": snapshot.and_then(|node| property_string(&node.properties, "content_hash")),
            "byte_len": snapshot.and_then(|node| property_string(&node.properties, "byte_len")),
            "text": snapshot.and_then(|node| property_string(&node.properties, "text")),
        });
        records.push(
            RawRecord::json(RUSTYWEB_PAGE_DATA_TYPE, body, 0).with_external_id(page.id.clone()),
        );
    }
    records
}

pub fn ingest_crawl_graph_as_datawave<S: GraphStore>(
    store: &mut S,
    graph: &CrawlGraph,
    tenant_id: Option<String>,
) -> Result<CrawlDatawaveIngestReport, DatawaveError> {
    let ingest = rustyweb_datawave_ingest(tenant_id);
    let records = datawave_records_from_crawl_graph(graph);
    let mut stats = IngestStats::new();
    let mut report = CrawlDatawaveIngestReport {
        data_type: RUSTYWEB_PAGE_DATA_TYPE.to_string(),
        records_seen: records.len(),
        ..CrawlDatawaveIngestReport::default()
    };

    for record in &records {
        match ingest.ingest_record(store, record, &mut stats) {
            Ok(outcome) => {
                report.ingested += 1;
                report.fields_written += outcome.fields_written;
                report.edges_written += outcome.edges_written;
                if outcome.deduped {
                    report.deduped += 1;
                }
            }
            Err(error) => {
                report.skipped += 1;
                report.errors.push(error.to_string());
            }
        }
    }
    report.dictionary_nodes_written = ingest.write_dictionary(store, &stats)?;
    Ok(report)
}

pub fn query_rustyweb_datawave<S: GraphStore>(
    store: &S,
    tenant_id: Option<&str>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let snapshot = store.graph_snapshot()?;
    query_rustyweb_datawave_snapshot(&snapshot, tenant_id, arguments)
}

pub fn query_rustyweb_datawave_snapshot(
    snapshot: &GraphSnapshot,
    tenant_id: Option<&str>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let input = RustyWebDatawaveQueryInput::from_value(arguments, tenant_id)?;
    let relational_store = RelationalStore::from_graph_snapshot(snapshot)?;
    let query = build_rustyweb_query(tenant_id, &input)?;
    let result = execute_query(&relational_store, query)?;
    let nodes_by_id = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();

    let mut event_ids = BTreeSet::new();
    let mut row_ids_by_event = serde_json::Map::new();
    for row in &result.rows {
        let Some(event_id) = row.get("f0.event_id").map(scalar_string) else {
            continue;
        };
        if let Some(row_id) = row.get("f0.id").map(scalar_string) {
            row_ids_by_event
                .entry(event_id.clone())
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .expect("inserted array")
                .push(json!(row_id));
        }
        event_ids.insert(event_id);
    }

    let mut pages_by_id = BTreeMap::new();
    let mut events = Vec::new();
    for event_id in event_ids {
        let event = nodes_by_id
            .get(&event_id)
            .map(|node| json!({ "id": node.id, "properties": node.properties }));
        let field_fact_nodes = field_facts_for_event(snapshot, tenant_id, &event_id);
        let pages = page_nodes_for_field_facts(&nodes_by_id, &field_fact_nodes)
            .into_iter()
            .map(|node| {
                let value = json!({ "id": node.id, "properties": node.properties });
                pages_by_id
                    .entry(node.id.clone())
                    .or_insert_with(|| value.clone());
                value
            })
            .collect::<Vec<_>>();
        let field_facts = field_fact_nodes
            .into_iter()
            .map(|node| json!({ "id": node.id, "properties": node.properties }))
            .collect::<Vec<_>>();
        let matching_row_ids = row_ids_by_event
            .remove(&event_id)
            .unwrap_or_else(|| json!([]));
        events.push(json!({
            "event_id": event_id,
            "event": event,
            "matching_row_ids": matching_row_ids,
            "field_facts": field_facts,
            "pages": pages,
        }));
    }
    let pages = pages_by_id.into_values().collect::<Vec<_>>();
    let page_count = pages.len();

    Ok(json!({
        "tenant_id": tenant_id,
        "data_type": RUSTYWEB_PAGE_DATA_TYPE,
        "predicate_count": input.predicates.len(),
        "predicates": input
            .predicates
            .iter()
            .map(|predicate| json!({
                "field": predicate.field,
                "normalized_value": predicate.value,
                "vf": format!("{}={}", predicate.field, predicate.value),
            }))
            .collect::<Vec<_>>(),
        "row_count": result.rows.len(),
        "event_count": events.len(),
        "events": events,
        "page_count": page_count,
        "pages": pages,
        "trace": result.trace,
    }))
}

impl RustyWebDatawaveQueryInput {
    fn from_value(arguments: Value, tenant_id: Option<&str>) -> GraphStoreResult<Self> {
        let mut predicates = Vec::new();
        if let Some(values) = arguments
            .get("predicates")
            .or_else(|| arguments.get("filters"))
            .and_then(Value::as_array)
        {
            for value in values {
                if let Some(predicate) = rustyweb_query_predicate_from_value(value, tenant_id)? {
                    predicates.push(predicate);
                }
            }
        }
        if let (Some(field), Some(value)) = (
            arguments
                .get("field")
                .or_else(|| arguments.get("fld"))
                .and_then(Value::as_str),
            arguments
                .get("value")
                .or_else(|| arguments.get("raw_value")),
        ) {
            if let Some(predicate) = rustyweb_query_predicate_from_parts(
                field,
                query_value_to_raw_string(value)?,
                tenant_id,
            )? {
                predicates.push(predicate);
            }
        }
        if predicates.is_empty() {
            return Err(GraphStoreError::new(
                "missing_rustyweb_query_predicate",
                "web.query requires at least one non-tenant field/value predicate",
            ));
        }
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_RUSTYWEB_QUERY_LIMIT)
            .clamp(1, MAX_RUSTYWEB_QUERY_LIMIT) as usize;
        Ok(Self { predicates, limit })
    }
}

fn rustyweb_query_predicate_from_value(
    value: &Value,
    tenant_id: Option<&str>,
) -> GraphStoreResult<Option<RustyWebDatawaveQueryPredicate>> {
    let object = value.as_object().ok_or_else(|| {
        GraphStoreError::new(
            "invalid_rustyweb_query_predicate",
            "each web.query predicate must be an object",
        )
    })?;
    let field = object
        .get("field")
        .or_else(|| object.get("fld"))
        .or_else(|| object.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            GraphStoreError::new(
                "invalid_rustyweb_query_predicate",
                "each web.query predicate requires field and value",
            )
        })?;
    let value = object
        .get("value")
        .or_else(|| object.get("raw_value"))
        .or_else(|| object.get("rawValue"))
        .ok_or_else(|| {
            GraphStoreError::new(
                "invalid_rustyweb_query_predicate",
                "each web.query predicate requires field and value",
            )
        })?;
    rustyweb_query_predicate_from_parts(field, query_value_to_raw_string(value)?, tenant_id)
}

fn rustyweb_query_predicate_from_parts(
    field: &str,
    raw_value: String,
    tenant_id: Option<&str>,
) -> GraphStoreResult<Option<RustyWebDatawaveQueryPredicate>> {
    let field = field.trim();
    if field.is_empty() {
        return Err(GraphStoreError::new(
            "invalid_rustyweb_query_predicate",
            "web.query predicate field must be non-empty",
        ));
    }
    if field.eq_ignore_ascii_case("tenant_id") || field.eq_ignore_ascii_case("tenantId") {
        if tenant_id == Some(raw_value.as_str()) {
            return Ok(None);
        }
        return Err(GraphStoreError::new(
            "tenant_scope_mismatch",
            "web.query tenant_id predicates must match the active tenant",
        ));
    }

    let config = rustyweb_datawave_field_config();
    let field = config.resolve_alias(field).to_string();
    let normalized = config
        .field_type(&field)
        .normalize(&raw_value)
        .map_err(|error| {
            GraphStoreError::new(
                "invalid_rustyweb_query_value",
                format!("failed to normalize {field}={raw_value:?}: {error}"),
            )
        })?;
    Ok(Some(RustyWebDatawaveQueryPredicate {
        field,
        value: normalized,
    }))
}

fn query_value_to_raw_string(value: &Value) -> GraphStoreResult<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        _ => Err(GraphStoreError::new(
            "invalid_rustyweb_query_value",
            "web.query values must be string, number, or boolean scalars",
        )),
    }
}

fn build_rustyweb_query(
    tenant_id: Option<&str>,
    input: &RustyWebDatawaveQueryInput,
) -> GraphStoreResult<QueryIr> {
    if input.predicates.is_empty() {
        return Err(GraphStoreError::new(
            "missing_rustyweb_query_predicate",
            "web.query requires at least one predicate",
        ));
    }
    let mut event_predicates = vec![Predicate::Equals {
        column: "data_type".to_string(),
        value: ScalarValue::String(RUSTYWEB_PAGE_DATA_TYPE.to_string()),
    }];
    if let Some(tenant_id) = tenant_id {
        event_predicates.push(Predicate::Equals {
            column: "tenant_id".to_string(),
            value: ScalarValue::String(tenant_id.to_string()),
        });
    }

    let mut relations = vec![QueryRelation {
        alias: "e".to_string(),
        relation: "ingestevent".to_string(),
        predicates: event_predicates,
    }];
    relations.extend(
        input
            .predicates
            .iter()
            .enumerate()
            .map(|(index, predicate)| {
                let mut predicates = vec![Predicate::Equals {
                    column: "vf".to_string(),
                    value: ScalarValue::String(format!("{}={}", predicate.field, predicate.value)),
                }];
                if let Some(tenant_id) = tenant_id {
                    predicates.push(Predicate::Equals {
                        column: "tenant_id".to_string(),
                        value: ScalarValue::String(tenant_id.to_string()),
                    });
                }
                QueryRelation {
                    alias: format!("f{index}"),
                    relation: "fieldfact".to_string(),
                    predicates,
                }
            }),
    );

    let mut joins = vec![JoinPredicate {
        left_alias: "f0".to_string(),
        left_column: "event_id".to_string(),
        right_alias: "e".to_string(),
        right_column: "id".to_string(),
    }];
    joins.extend((1..input.predicates.len()).map(|index| JoinPredicate {
        left_alias: "f0".to_string(),
        left_column: "event_id".to_string(),
        right_alias: format!("f{index}"),
        right_column: "event_id".to_string(),
    }));

    Ok(QueryIr {
        relations,
        joins,
        projection: vec![
            Projection {
                alias: "f0".to_string(),
                column: "event_id".to_string(),
            },
            Projection {
                alias: "f0".to_string(),
                column: "id".to_string(),
            },
            Projection {
                alias: "e".to_string(),
                column: "id".to_string(),
            },
        ],
        limit: Some(input.limit),
        ..QueryIr::default()
    })
}

fn scalar_string(value: &ScalarValue) -> String {
    match value {
        ScalarValue::String(value) => value.clone(),
        ScalarValue::I64(value) => value.to_string(),
        ScalarValue::F64(value) => value.to_string(),
        ScalarValue::Bool(value) => value.to_string(),
    }
}

fn field_facts_for_event<'a>(
    snapshot: &'a GraphSnapshot,
    tenant_id: Option<&str>,
    event_id: &str,
) -> Vec<&'a NodeRecord> {
    snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .filter(|node| has_label(node, FIELD_FACT_LABEL))
        .filter(|node| node.properties.get("event_id").and_then(Value::as_str) == Some(event_id))
        .filter(|node| match tenant_id {
            Some(tenant_id) => {
                node.properties.get("tenant_id").and_then(Value::as_str) == Some(tenant_id)
            }
            None => true,
        })
        .take(MAX_RUSTYWEB_QUERY_LIMIT as usize)
        .collect()
}

fn page_nodes_for_field_facts<'a>(
    nodes_by_id: &BTreeMap<String, &'a NodeRecord>,
    field_facts: &[&NodeRecord],
) -> Vec<&'a NodeRecord> {
    field_facts
        .iter()
        .filter(|node| node.properties.get("fld").and_then(Value::as_str) == Some("page_id"))
        .filter_map(|node| {
            node.properties
                .get("raw_value")
                .or_else(|| node.properties.get("nv"))
                .and_then(Value::as_str)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|page_id| nodes_by_id.get(page_id).copied())
        .collect()
}

fn has_label(node: &NodeRecord, label: &str) -> bool {
    node.labels.iter().any(|candidate| candidate == label)
}

fn property_string(properties: &Value, key: &str) -> Option<String> {
    match properties.get(key)? {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use rustyred_thg_core::{InMemoryGraphStore, NodeQuery, RedCoreGraphStore};
    use rustyred_thg_datawave::{EVENT_LABEL, FIELD_FACT_LABEL};

    use super::*;
    use crate::{build_v2_fixture_crawl, CrawlRequest, FixturePage};

    #[test]
    fn crawl_graph_becomes_datawave_field_facts() {
        let request = CrawlRequest::new(
            "rw-datawave-1",
            vec!["https://example.com/index.html".to_string()],
        );
        let output = build_v2_fixture_crawl(
            request,
            &[FixturePage::html(
                "https://example.com/index.html",
                "<html><body>alpha beta<a href=\"/about\">About</a></body></html>",
            )],
        )
        .unwrap();

        let mut store = InMemoryGraphStore::default();
        let apply_report = output
            .graph
            .apply_to_store_with_datawave(&mut store, Some("tenant-a".to_string()))
            .unwrap();
        let report = apply_report.datawave;

        assert_eq!(report.records_seen, 1);
        assert_eq!(report.ingested, 1);
        assert!(report.fields_written >= 8);
        assert!(report.edges_written >= 3);

        let events = store.query_nodes(
            NodeQuery::label(EVENT_LABEL)
                .with_property("tenant_id", json!("tenant-a"))
                .with_property("data_type", json!(RUSTYWEB_PAGE_DATA_TYPE)),
        );
        assert_eq!(events.len(), 1);

        let url_facts = store.query_nodes(
            NodeQuery::label(FIELD_FACT_LABEL)
                .with_property("tenant_id", json!("tenant-a"))
                .with_property("fld", json!("url")),
        );
        assert_eq!(url_facts.len(), 1);
        assert_eq!(
            url_facts[0].properties["nv"],
            json!("https://example.com/index.html")
        );

        let text_facts = store.query_nodes(
            NodeQuery::label(FIELD_FACT_LABEL)
                .with_property("tenant_id", json!("tenant-a"))
                .with_property("fld", json!("text")),
        );
        assert_eq!(text_facts.len(), 1);
        assert!(text_facts[0].properties["nv"]
            .as_str()
            .unwrap()
            .contains("alpha beta"));
    }

    #[test]
    fn rustyweb_query_intersects_page_facts_and_hydrates_pages() {
        let request = CrawlRequest::new(
            "rw-query-1",
            vec!["https://example.com/index.html".to_string()],
        );
        let output = build_v2_fixture_crawl(
            request,
            &[FixturePage::html(
                "https://example.com/index.html",
                "<html><body>Alpha Beta<a href=\"/about\">About</a></body></html>",
            )],
        )
        .unwrap();

        let other_request = CrawlRequest::new(
            "rw-query-other",
            vec!["https://example.com/index.html".to_string()],
        );
        let other_output = build_v2_fixture_crawl(
            other_request,
            &[FixturePage::html(
                "https://example.com/index.html",
                "<html><body>Other tenant</body></html>",
            )],
        )
        .unwrap();

        let mut store = RedCoreGraphStore::memory();
        output
            .graph
            .apply_to_store_with_datawave(&mut store, Some("tenant-a".to_string()))
            .unwrap();
        other_output
            .graph
            .apply_to_store_with_datawave(&mut store, Some("tenant-b".to_string()))
            .unwrap();

        let result = query_rustyweb_datawave(
            &store,
            Some("tenant-a"),
            json!({
                "predicates": [
                    { "field": "url", "value": "https://example.com/index.html" },
                    { "field": "domain", "value": "EXAMPLE.COM" }
                ]
            }),
        )
        .unwrap();

        assert_eq!(result["tenant_id"], json!("tenant-a"));
        assert_eq!(result["data_type"], json!(RUSTYWEB_PAGE_DATA_TYPE));
        assert_eq!(result["event_count"], json!(1));
        assert_eq!(result["page_count"], json!(1));
        assert_eq!(
            result["pages"][0]["properties"]["url"],
            json!("https://example.com/index.html")
        );
        assert!(result["events"][0]["field_facts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|fact| fact["properties"]["fld"] == json!("text")
                && fact["properties"]["nv"]
                    .as_str()
                    .unwrap_or("")
                    .contains("alpha beta")));
        assert_eq!(result["trace"]["full_relation_scans"], json!(0));
        assert_eq!(result["trace"]["join_algorithm"], json!("hash_join"));
        assert_eq!(result["trace"]["used_roaring_bitmaps"], json!(true));
        let access_paths = result["trace"]["access_paths"].as_array().unwrap();
        assert!(!access_paths.is_empty());
        assert!(!access_paths
            .iter()
            .any(|path| path["method"] == json!("full_scan")));
    }
}
