use std::collections::{BTreeMap, BTreeSet, HashMap};

use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};

use crate::access_method::{
    AccessMethod, ColumnId, Cost, Predicate, RelationId, RowId, ScalarValue,
};
use crate::graph_store::{GraphStoreError, GraphStoreResult};
use crate::relational::{RelationalRow, RelationalStore};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryRelation {
    pub alias: String,
    pub relation: RelationId,
    #[serde(default)]
    pub predicates: Vec<Predicate>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JoinPredicate {
    pub left_alias: String,
    pub left_column: ColumnId,
    pub right_alias: String,
    pub right_column: ColumnId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Projection {
    pub alias: String,
    pub column: ColumnId,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct QueryIr {
    pub relations: Vec<QueryRelation>,
    #[serde(default)]
    pub joins: Vec<JoinPredicate>,
    #[serde(default)]
    pub projection: Vec<Projection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AccessPathTrace {
    pub relation: RelationId,
    pub alias: String,
    pub predicate: String,
    pub method: String,
    pub est_rows: f64,
    pub est_work: f64,
    pub returned_rows: usize,
    pub visited_rows: usize,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PlanTrace {
    pub access_paths: Vec<AccessPathTrace>,
    pub full_relation_scans: usize,
    pub bitmap_intersections: usize,
    pub used_roaring_bitmaps: bool,
    pub join_algorithm: Option<String>,
    pub joined_rows: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryResult {
    pub rows: Vec<BTreeMap<String, ScalarValue>>,
    pub trace: PlanTrace,
}

pub fn execute_query(store: &RelationalStore, query: QueryIr) -> GraphStoreResult<QueryResult> {
    if query.relations.is_empty() {
        return Err(GraphStoreError::new(
            "empty_relational_query",
            "query requires at least one relation",
        ));
    }

    let mut trace = PlanTrace::default();
    let mut per_alias: BTreeMap<String, Vec<RelationalRow>> = BTreeMap::new();
    for relation_query in &query.relations {
        let relation = store.relation(&relation_query.relation).ok_or_else(|| {
            GraphStoreError::new(
                "unknown_relation",
                format!("unknown relation {}", relation_query.relation),
            )
        })?;
        let row_ids = candidate_rows(store, relation_query, &mut trace)?;
        let rows = row_ids
            .into_iter()
            .filter_map(|id| relation.get(&id).cloned())
            .filter(|row| {
                relation_query
                    .predicates
                    .iter()
                    .all(|predicate| row_matches(row, predicate))
            })
            .collect::<Vec<_>>();
        per_alias.insert(relation_query.alias.clone(), rows);
    }

    let joined = execute_joins(&per_alias, &query.joins, &mut trace)?;
    let mut out = Vec::new();
    for rowset in joined {
        out.push(project_rowset(&rowset, &query.projection));
        if query.limit.is_some_and(|limit| out.len() >= limit) {
            break;
        }
    }
    trace.joined_rows = out.len();
    Ok(QueryResult { rows: out, trace })
}

fn candidate_rows(
    store: &RelationalStore,
    relation_query: &QueryRelation,
    trace: &mut PlanTrace,
) -> GraphStoreResult<Vec<RowId>> {
    let relation = store.relation(&relation_query.relation).ok_or_else(|| {
        GraphStoreError::new(
            "unknown_relation",
            format!("unknown relation {}", relation_query.relation),
        )
    })?;
    if relation_query.predicates.is_empty() {
        trace.full_relation_scans += 1;
        return Ok(relation.row_ids());
    }
    let row_ord = relation
        .row_ids()
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, index as u32))
        .collect::<BTreeMap<_, _>>();
    let ord_row = row_ord
        .iter()
        .map(|(row, ord)| (*ord, row.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut bitmaps = Vec::new();
    for predicate in &relation_query.predicates {
        let mut best: Option<(&dyn AccessMethod, Cost)> = None;
        for method in store.access_methods().methods() {
            if !method.supports(&relation_query.relation, predicate) {
                continue;
            }
            let Some(cost) = method.cost(&relation_query.relation, predicate) else {
                continue;
            };
            if best
                .map(|(_, best_cost)| cost.rank_key() < best_cost.rank_key())
                .unwrap_or(true)
            {
                best = Some((method, cost));
            }
        }
        let Some((method, cost)) = best else {
            trace.full_relation_scans += 1;
            bitmaps.push(row_ids_to_bitmap(
                relation.rows().map(|row| &row.id),
                &row_ord,
            ));
            continue;
        };
        let stream = method.scan(&relation_query.relation, predicate)?;
        trace.access_paths.push(AccessPathTrace {
            relation: relation_query.relation.clone(),
            alias: relation_query.alias.clone(),
            predicate: predicate.kind_name().to_string(),
            method: method.name().to_string(),
            est_rows: cost.est_rows,
            est_work: cost.est_work,
            returned_rows: stream.rows.len(),
            visited_rows: stream.visited,
        });
        bitmaps.push(row_ids_to_bitmap(stream.rows.iter(), &row_ord));
    }

    let mut iter = bitmaps.into_iter();
    let mut bitmap = iter.next().unwrap_or_default();
    for next in iter {
        bitmap &= next;
        trace.bitmap_intersections += 1;
        trace.used_roaring_bitmaps = true;
    }
    Ok(bitmap
        .iter()
        .filter_map(|ord| ord_row.get(&ord).cloned())
        .collect())
}

fn row_ids_to_bitmap<'a>(
    rows: impl Iterator<Item = &'a RowId>,
    row_ord: &BTreeMap<RowId, u32>,
) -> RoaringBitmap {
    let mut bitmap = RoaringBitmap::new();
    for row in rows {
        if let Some(ord) = row_ord.get(row) {
            bitmap.insert(*ord);
        }
    }
    bitmap
}

fn execute_joins(
    per_alias: &BTreeMap<String, Vec<RelationalRow>>,
    joins: &[JoinPredicate],
    trace: &mut PlanTrace,
) -> GraphStoreResult<Vec<BTreeMap<String, RelationalRow>>> {
    if joins.is_empty() {
        let Some((alias, rows)) = per_alias.iter().next() else {
            return Ok(Vec::new());
        };
        return Ok(rows
            .iter()
            .cloned()
            .map(|row| BTreeMap::from([(alias.clone(), row)]))
            .collect());
    }
    trace.join_algorithm = Some("hash_join".to_string());
    let mut result = Vec::new();
    let first_join = &joins[0];
    let left_rows = per_alias
        .get(&first_join.left_alias)
        .ok_or_else(|| GraphStoreError::new("unknown_join_alias", first_join.left_alias.clone()))?;
    let right_rows = per_alias.get(&first_join.right_alias).ok_or_else(|| {
        GraphStoreError::new("unknown_join_alias", first_join.right_alias.clone())
    })?;
    let mut right_index: HashMap<String, Vec<RelationalRow>> = HashMap::new();
    for row in right_rows {
        if let Some(key) = join_key(row, &first_join.right_column) {
            right_index.entry(key).or_default().push(row.clone());
        }
    }
    for left in left_rows {
        let Some(key) = join_key(left, &first_join.left_column) else {
            continue;
        };
        let Some(matches) = right_index.get(&key) else {
            continue;
        };
        for right in matches {
            let mut rowset = BTreeMap::new();
            rowset.insert(first_join.left_alias.clone(), left.clone());
            rowset.insert(first_join.right_alias.clone(), right.clone());
            result.push(rowset);
        }
    }
    for join in joins.iter().skip(1) {
        let right_rows = per_alias
            .get(&join.right_alias)
            .ok_or_else(|| GraphStoreError::new("unknown_join_alias", join.right_alias.clone()))?;
        let mut right_index: HashMap<String, Vec<RelationalRow>> = HashMap::new();
        for row in right_rows {
            if let Some(key) = join_key(row, &join.right_column) {
                right_index.entry(key).or_default().push(row.clone());
            }
        }
        let mut next_result = Vec::new();
        for rowset in result {
            let Some(left) = rowset.get(&join.left_alias) else {
                continue;
            };
            let Some(key) = join_key(left, &join.left_column) else {
                continue;
            };
            let Some(matches) = right_index.get(&key) else {
                continue;
            };
            for right in matches {
                let mut joined = rowset.clone();
                joined.insert(join.right_alias.clone(), right.clone());
                next_result.push(joined);
            }
        }
        result = next_result;
    }
    Ok(result)
}

fn project_rowset(
    rowset: &BTreeMap<String, RelationalRow>,
    projection: &[Projection],
) -> BTreeMap<String, ScalarValue> {
    if projection.is_empty() {
        return rowset
            .iter()
            .flat_map(|(alias, row)| {
                let mut fields = Vec::new();
                fields.push((format!("{alias}.id"), ScalarValue::String(row.id.clone())));
                fields.extend(
                    row.values
                        .iter()
                        .map(|(column, value)| (format!("{alias}.{column}"), value.clone())),
                );
                fields
            })
            .collect();
    }
    projection
        .iter()
        .filter_map(|field| {
            let row = rowset.get(&field.alias)?;
            if field.column == "id" {
                return Some((
                    format!("{}.{}", field.alias, field.column),
                    ScalarValue::String(row.id.clone()),
                ));
            }
            row.values
                .get(&field.column)
                .cloned()
                .map(|value| (format!("{}.{}", field.alias, field.column), value))
        })
        .collect()
}

fn join_key(row: &RelationalRow, column: &str) -> Option<String> {
    if column == "id" {
        return Some(row.id.clone());
    }
    row.values.get(column).map(scalar_key)
}

fn scalar_key(value: &ScalarValue) -> String {
    match value {
        ScalarValue::String(value) => value.clone(),
        ScalarValue::I64(value) => value.to_string(),
        ScalarValue::F64(value) => value.to_string(),
        ScalarValue::Bool(value) => value.to_string(),
    }
}

fn row_matches(row: &RelationalRow, predicate: &Predicate) -> bool {
    match predicate {
        Predicate::Equals { column, value } => row.values.get(column) == Some(value),
        Predicate::Range { column, lo, hi } => row
            .values
            .get(column)
            .and_then(ScalarValue::as_f64)
            .map(|score| lo.contains_numeric(score, true) && hi.contains_numeric(score, false))
            .unwrap_or(false),
        Predicate::Prefix { column, prefix } => row
            .values
            .get(column)
            .and_then(ScalarValue::as_str)
            .map(|value| value.starts_with(prefix))
            .unwrap_or(false),
        Predicate::TimeRange {
            column,
            lo_ms,
            hi_ms,
        } => row
            .values
            .get(column)
            .and_then(ScalarValue::as_i64)
            .map(|value| value >= *lo_ms && value <= *hi_ms)
            .unwrap_or(false),
        Predicate::Knn { .. }
        | Predicate::GeoWithin { .. }
        | Predicate::TextMatch { .. }
        | Predicate::Expand { .. } => true,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphqlSelection {
    pub relation: RelationId,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub fields: Vec<ColumnId>,
    #[serde(default)]
    pub joins: Vec<GraphqlJoinSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphqlJoinSelection {
    pub relation: RelationId,
    #[serde(default)]
    pub alias: Option<String>,
    pub left_column: ColumnId,
    pub right_column: ColumnId,
    #[serde(default)]
    pub fields: Vec<ColumnId>,
}

pub fn compile_graphql_selection(selection: GraphqlSelection) -> QueryIr {
    let root_alias = selection
        .alias
        .clone()
        .unwrap_or_else(|| selection.relation.clone());
    let mut relations = vec![QueryRelation {
        alias: root_alias.clone(),
        relation: selection.relation.clone(),
        predicates: Vec::new(),
    }];
    let mut joins = Vec::new();
    let mut projection = selection
        .fields
        .into_iter()
        .map(|column| Projection {
            alias: root_alias.clone(),
            column,
        })
        .collect::<Vec<_>>();
    let mut seen_aliases = BTreeSet::from([root_alias.clone()]);
    for nested in selection.joins {
        let alias = nested.alias.unwrap_or_else(|| nested.relation.clone());
        if seen_aliases.insert(alias.clone()) {
            relations.push(QueryRelation {
                alias: alias.clone(),
                relation: nested.relation,
                predicates: Vec::new(),
            });
        }
        joins.push(JoinPredicate {
            left_alias: root_alias.clone(),
            left_column: nested.left_column,
            right_alias: alias.clone(),
            right_column: nested.right_column,
        });
        projection.extend(nested.fields.into_iter().map(|column| Projection {
            alias: alias.clone(),
            column,
        }));
    }
    QueryIr {
        relations,
        joins,
        projection,
        limit: selection.limit,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::access_method::{ScalarBound, ScalarValue};
    use crate::relational::{RelationalRow, RelationalStore};

    fn row(relation: &str, id: &str, values: &[(&str, ScalarValue)]) -> RelationalRow {
        RelationalRow::new(
            relation,
            id,
            values
                .iter()
                .map(|(key, value)| ((*key).to_string(), value.clone()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn scalar_and_time_predicates_use_access_methods_and_roaring_intersection() {
        let mut store = RelationalStore::new();
        store
            .upsert_row(row(
                "memory",
                "m1",
                &[
                    ("kind", ScalarValue::String("episode".to_string())),
                    ("t_ms", ScalarValue::I64(5)),
                ],
            ))
            .unwrap();
        store
            .upsert_row(row(
                "memory",
                "m2",
                &[
                    ("kind", ScalarValue::String("episode".to_string())),
                    ("t_ms", ScalarValue::I64(15)),
                ],
            ))
            .unwrap();
        store
            .upsert_row(row(
                "memory",
                "m3",
                &[
                    ("kind", ScalarValue::String("note".to_string())),
                    ("t_ms", ScalarValue::I64(15)),
                ],
            ))
            .unwrap();

        let result = execute_query(
            &store,
            QueryIr {
                relations: vec![QueryRelation {
                    alias: "m".to_string(),
                    relation: "memory".to_string(),
                    predicates: vec![
                        Predicate::Equals {
                            column: "kind".to_string(),
                            value: ScalarValue::String("episode".to_string()),
                        },
                        Predicate::TimeRange {
                            column: "t_ms".to_string(),
                            lo_ms: 10,
                            hi_ms: 20,
                        },
                    ],
                }],
                projection: vec![Projection {
                    alias: "m".to_string(),
                    column: "id".to_string(),
                }],
                ..QueryIr::default()
            },
        )
        .unwrap();

        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            result.rows[0].get("m.id"),
            Some(&ScalarValue::String("m2".to_string()))
        );
        assert_eq!(result.trace.full_relation_scans, 0);
        assert!(result.trace.used_roaring_bitmaps);
        assert_eq!(result.trace.bitmap_intersections, 1);
        assert!(result
            .trace
            .access_paths
            .iter()
            .any(|path| path.method == "ordered" && path.predicate == "equals"));
        assert!(result
            .trace
            .access_paths
            .iter()
            .any(|path| path.method == "time_series" && path.predicate == "time_range"));
    }

    #[test]
    fn planner_hash_join_returns_many_to_many_content_epistemic_association() {
        let mut store = RelationalStore::new();
        store
            .upsert_row(row(
                "content",
                "c1",
                &[
                    ("content_key", ScalarValue::String("doc:1".to_string())),
                    ("title", ScalarValue::String("Root".to_string())),
                ],
            ))
            .unwrap();
        store
            .upsert_row(row(
                "epistemic",
                "e1",
                &[
                    ("content_key", ScalarValue::String("doc:1".to_string())),
                    ("claim", ScalarValue::String("supports".to_string())),
                ],
            ))
            .unwrap();
        store
            .upsert_row(row(
                "epistemic",
                "e2",
                &[
                    ("content_key", ScalarValue::String("doc:1".to_string())),
                    ("claim", ScalarValue::String("undercuts".to_string())),
                ],
            ))
            .unwrap();

        let result = execute_query(
            &store,
            QueryIr {
                relations: vec![
                    QueryRelation {
                        alias: "c".to_string(),
                        relation: "content".to_string(),
                        predicates: Vec::new(),
                    },
                    QueryRelation {
                        alias: "e".to_string(),
                        relation: "epistemic".to_string(),
                        predicates: Vec::new(),
                    },
                ],
                joins: vec![JoinPredicate {
                    left_alias: "c".to_string(),
                    left_column: "content_key".to_string(),
                    right_alias: "e".to_string(),
                    right_column: "content_key".to_string(),
                }],
                projection: vec![
                    Projection {
                        alias: "c".to_string(),
                        column: "title".to_string(),
                    },
                    Projection {
                        alias: "e".to_string(),
                        column: "claim".to_string(),
                    },
                ],
                ..QueryIr::default()
            },
        )
        .unwrap();

        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.trace.join_algorithm.as_deref(), Some("hash_join"));
        let claims = result
            .rows
            .iter()
            .filter_map(|row| row.get("e.claim"))
            .collect::<Vec<_>>();
        assert!(claims.contains(&&ScalarValue::String("supports".to_string())));
        assert!(claims.contains(&&ScalarValue::String("undercuts".to_string())));
    }

    #[test]
    fn graphql_selection_compiles_to_single_planner_pass_join() {
        let mut store = RelationalStore::new();
        store
            .upsert_row(row(
                "content",
                "c1",
                &[("key", ScalarValue::String("k1".to_string()))],
            ))
            .unwrap();
        store
            .upsert_row(row(
                "epistemic",
                "e1",
                &[
                    ("content_key", ScalarValue::String("k1".to_string())),
                    ("claim", ScalarValue::String("grounded".to_string())),
                ],
            ))
            .unwrap();

        let query = compile_graphql_selection(GraphqlSelection {
            relation: "content".to_string(),
            alias: Some("content".to_string()),
            fields: vec!["key".to_string()],
            joins: vec![GraphqlJoinSelection {
                relation: "epistemic".to_string(),
                alias: Some("epistemic".to_string()),
                left_column: "key".to_string(),
                right_column: "content_key".to_string(),
                fields: vec!["claim".to_string()],
            }],
            limit: None,
        });
        let result = execute_query(&store, query).unwrap();

        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            result.rows[0].get("epistemic.claim"),
            Some(&ScalarValue::String("grounded".to_string()))
        );
        assert_eq!(result.trace.join_algorithm.as_deref(), Some("hash_join"));
    }

    #[test]
    fn range_predicate_filters_exclusive_bounds_after_index_scan() {
        let mut store = RelationalStore::new();
        for (id, score) in [("a", 1), ("b", 2), ("c", 3)] {
            store
                .upsert_row(row("scores", id, &[("score", ScalarValue::I64(score))]))
                .unwrap();
        }
        let result = execute_query(
            &store,
            QueryIr {
                relations: vec![QueryRelation {
                    alias: "s".to_string(),
                    relation: "scores".to_string(),
                    predicates: vec![Predicate::Range {
                        column: "score".to_string(),
                        lo: ScalarBound::Excluded(ScalarValue::I64(1)),
                        hi: ScalarBound::Included(ScalarValue::I64(3)),
                    }],
                }],
                projection: vec![Projection {
                    alias: "s".to_string(),
                    column: "id".to_string(),
                }],
                ..QueryIr::default()
            },
        )
        .unwrap();
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.trace.access_paths[0].method, "ordered");
    }
}
