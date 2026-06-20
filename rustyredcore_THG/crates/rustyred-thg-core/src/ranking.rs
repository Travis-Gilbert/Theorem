//! Ranking access methods for the multimodal planner rank phase.
//!
//! Each method is a thin, stateless adapter: it declares which predicate it ranks
//! (`supports`), gives the planner a selection cost, and delegates the actual scoring to the
//! live [`ModalityResolver`] supplied at query time. No modality data (vectors, term postings,
//! coordinates, edges) is held here or copied into the relational store; the resolver resolves
//! everything by node id from the live index subsystems (TurboVec, full-text, spatial, graph).
//!
//! The residual exact-check helpers (`row_text_matches`, `row_geo_within`) stay here because the
//! planner runs them over the relation's own scalar/structured columns after an over-approximate
//! filter, exactly as the spec's "residual exact check in `row_matches`" requires.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::access_method::{
    AmResult, Cost, ModalityResolver, Predicate, PredicateMode, RankOutcome, RankingAccessMethod,
    RankingRegistry, RowId, ScalarValue,
};

impl RankingRegistry {
    pub fn with_native_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(VectorRankingMethod::new());
        registry.register(TextRankingMethod::new());
        registry.register(ExpandRankingMethod::new());
        registry
    }
}

/// kNN ranker (`vector`). Delegates to the live vector index (TurboVec) by node id.
#[derive(Clone, Copy, Debug, Default)]
pub struct VectorRankingMethod;

impl VectorRankingMethod {
    pub fn new() -> Self {
        Self
    }
}

impl RankingAccessMethod for VectorRankingMethod {
    fn name(&self) -> &'static str {
        "vector"
    }

    fn supports(&self, _relation: &str, predicate: &Predicate) -> bool {
        matches!(predicate, Predicate::Knn { .. })
    }

    fn cost(
        &self,
        _relation: &str,
        predicate: &Predicate,
        candidates: Option<usize>,
    ) -> Option<Cost> {
        let Predicate::Knn { k, .. } = predicate else {
            return None;
        };
        let rows = candidates.unwrap_or((*k).max(1)).max(1) as f64;
        Some(Cost::new((*k).max(1) as f64, rows))
    }

    fn rank(
        &self,
        relation: &str,
        predicate: &Predicate,
        candidates: Option<&BTreeSet<RowId>>,
        k: usize,
        resolver: &dyn ModalityResolver,
    ) -> AmResult<RankOutcome> {
        let Predicate::Knn { column, query, .. } = predicate else {
            return Ok(RankOutcome::default());
        };
        if k == 0 || query.iter().any(|value| !value.is_finite()) {
            return Ok(RankOutcome::default());
        }
        resolver.vector_knn(relation, column, query, candidates, k)
    }
}

/// Relevance ranker (`text_rank`). Delegates BM25 to the live full-text index by node id.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextRankingMethod;

impl TextRankingMethod {
    pub fn new() -> Self {
        Self
    }
}

impl RankingAccessMethod for TextRankingMethod {
    fn name(&self) -> &'static str {
        "text_rank"
    }

    fn supports(&self, _relation: &str, predicate: &Predicate) -> bool {
        matches!(
            predicate,
            Predicate::TextMatch {
                mode: PredicateMode::Rank,
                ..
            }
        )
    }

    fn cost(
        &self,
        _relation: &str,
        predicate: &Predicate,
        candidates: Option<usize>,
    ) -> Option<Cost> {
        if !self.supports("", predicate) {
            return None;
        }
        let rows = candidates.unwrap_or(1).max(1) as f64;
        Some(Cost::new(rows, rows.log2() + 1.0))
    }

    fn rank(
        &self,
        relation: &str,
        predicate: &Predicate,
        candidates: Option<&BTreeSet<RowId>>,
        k: usize,
        resolver: &dyn ModalityResolver,
    ) -> AmResult<RankOutcome> {
        let Predicate::TextMatch { column, query, .. } = predicate else {
            return Ok(RankOutcome::default());
        };
        if k == 0 {
            return Ok(RankOutcome::default());
        }
        Ok(RankOutcome::scored(resolver.text_rank(
            relation, column, query, candidates, k,
        )?))
    }
}

/// Proximity ranker (`expand_ppr`). Delegates hop-distance / PPR to the live graph by node id.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExpandRankingMethod;

impl ExpandRankingMethod {
    pub fn new() -> Self {
        Self
    }
}

impl RankingAccessMethod for ExpandRankingMethod {
    fn name(&self) -> &'static str {
        "expand_ppr"
    }

    fn supports(&self, _relation: &str, predicate: &Predicate) -> bool {
        matches!(
            predicate,
            Predicate::Expand {
                mode: PredicateMode::Rank,
                ..
            }
        )
    }

    fn cost(
        &self,
        _relation: &str,
        predicate: &Predicate,
        candidates: Option<usize>,
    ) -> Option<Cost> {
        if !self.supports("", predicate) {
            return None;
        }
        let rows = candidates.unwrap_or(64).max(1) as f64;
        Some(Cost::new(rows, rows * 0.5))
    }

    fn rank(
        &self,
        _relation: &str,
        predicate: &Predicate,
        candidates: Option<&BTreeSet<RowId>>,
        k: usize,
        resolver: &dyn ModalityResolver,
    ) -> AmResult<RankOutcome> {
        let Predicate::Expand {
            from,
            edge_type,
            dir,
            ..
        } = predicate
        else {
            return Ok(RankOutcome::default());
        };
        if k == 0 {
            return Ok(RankOutcome::default());
        }
        Ok(RankOutcome::scored(resolver.expand_proximity(
            from,
            edge_type,
            dir.clone(),
            candidates,
            k,
        )?))
    }
}

/// Residual term-presence check over the row's own scalar/structured text column. Run by the
/// planner after the over-approximate full-text filter narrows the candidate set.
pub(crate) fn row_text_matches(
    row: &crate::relational::RelationalRow,
    column: &str,
    query: &str,
) -> bool {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return false;
    }
    let text = row
        .values
        .get(column)
        .and_then(ScalarValue::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            row.properties
                .get(column)
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });
    let Some(text) = text else {
        return false;
    };
    let row_tokens = tokenize(&text);
    query_tokens
        .iter()
        .all(|query_token| row_tokens.contains(query_token))
}

/// Resolve a row's coordinates for the residual geo check, run by the planner after the
/// over-approximate spatial filter (so a point inside a boundary cell but outside the exact bbox is
/// excluded). Explicit `lat_property`/`lon_property` (the spatial designation) take precedence;
/// otherwise fall back to a `{column}` `{lat,lon}` object, `{column}_lat`/`{column}_lon`, or bare
/// `lat`/`lon` for the conventional `geo` column.
pub(crate) fn row_coords(
    row: &crate::relational::RelationalRow,
    column: &str,
    lat_property: Option<&str>,
    lon_property: Option<&str>,
) -> Option<(f64, f64)> {
    if let (Some(lat_key), Some(lon_key)) = (lat_property, lon_property) {
        return Some((coord_field(row, lat_key)?, coord_field(row, lon_key)?));
    }
    if let Some(obj) = row.properties.get(column).and_then(Value::as_object) {
        let lat = obj.get("lat").and_then(Value::as_f64)?;
        let lon = obj.get("lon").and_then(Value::as_f64)?;
        return Some((lat, lon));
    }
    let lat = coord_field(row, &format!("{column}_lat"))
        .or_else(|| (column == "geo").then(|| coord_field(row, "lat")).flatten())?;
    let lon = coord_field(row, &format!("{column}_lon"))
        .or_else(|| (column == "geo").then(|| coord_field(row, "lon")).flatten())?;
    Some((lat, lon))
}

fn coord_field(row: &crate::relational::RelationalRow, key: &str) -> Option<f64> {
    row.values
        .get(key)
        .and_then(ScalarValue::as_f64)
        .or_else(|| row.properties.get(key).and_then(Value::as_f64))
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_lowercase())
        .collect()
}
