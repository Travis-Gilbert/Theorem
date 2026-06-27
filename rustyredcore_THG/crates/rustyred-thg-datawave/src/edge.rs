//! Phase 6: the edge model. Declared rules turn field-facts into entity-to-entity
//! edges, gated by a conditional evaluation step and carrying a definition
//! version.
//!
//! DATAWAVE references:
//! - `mapreduce/handler/edge/ProtobufEdgeDataTypeHandler.java`, `edge/define/`,
//!   `edge/evaluation/`: edge definitions declare which field pairs become
//!   entity-edges, an evaluation layer gates edges conditionally, and an
//!   edge-key versioning cache tracks edge-definition versions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::field::NormalizedField;

fn default_edge_version() -> u32 {
    1
}

/// One edge definition: the two fields whose values become the edge endpoints,
/// the edge type, an optional conditional gate, and a definition version.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EdgeDef {
    pub edge_type: String,
    pub from_field: String,
    pub to_field: String,
    #[serde(default)]
    pub condition: EdgeCondition,
    #[serde(default = "default_edge_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl EdgeDef {
    pub fn new(
        edge_type: impl Into<String>,
        from_field: impl Into<String>,
        to_field: impl Into<String>,
    ) -> Self {
        Self {
            edge_type: edge_type.into(),
            from_field: from_field.into(),
            to_field: to_field.into(),
            condition: EdgeCondition::Always,
            version: default_edge_version(),
            group: None,
        }
    }

    pub fn when(mut self, condition: EdgeCondition) -> Self {
        self.condition = condition;
        self
    }

    pub fn with_version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }
}

/// The conditional evaluation gate. DATAWAVE uses JEXL expressions; this covers
/// the field-presence/equality leaves plus boolean composition (`&&`/`||`/`!`),
/// which together gate the bulk of real edge rules.
/// ponytail: regex preconditions (`=~`) are the one remaining JEXL piece; they
/// need a regex dependency, so they are a named upgrade rather than a leaf here.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EdgeCondition {
    #[default]
    Always,
    FieldPresent {
        field: String,
    },
    /// True when the field has a normalized value equal to `value`.
    FieldEquals {
        field: String,
        value: String,
    },
    /// True when the field has no normalized value equal to `value` (absent or
    /// all-different), the JEXL `!=` sense.
    FieldNotEquals {
        field: String,
        value: String,
    },
    /// True when every sub-condition holds (JEXL `&&`; empty is true).
    All {
        conditions: Vec<EdgeCondition>,
    },
    /// True when any sub-condition holds (JEXL `||`; empty is false).
    Any {
        conditions: Vec<EdgeCondition>,
    },
    /// Negation (JEXL `!`).
    Not {
        condition: Box<EdgeCondition>,
    },
}

impl EdgeCondition {
    fn eval(&self, index: &FieldIndex) -> bool {
        match self {
            EdgeCondition::Always => true,
            EdgeCondition::FieldPresent { field } => index.contains_key(field.as_str()),
            EdgeCondition::FieldEquals { field, value } => index
                .get(field.as_str())
                .is_some_and(|values| values.iter().any(|v| v == value)),
            EdgeCondition::FieldNotEquals { field, value } => !index
                .get(field.as_str())
                .is_some_and(|values| values.iter().any(|v| v == value)),
            EdgeCondition::All { conditions } => conditions.iter().all(|c| c.eval(index)),
            EdgeCondition::Any { conditions } => conditions.iter().any(|c| c.eval(index)),
            EdgeCondition::Not { condition } => !condition.eval(index),
        }
    }
}

/// A derived entity-edge: the two endpoints carry their field name and
/// normalized value so materialization can mint stable entity nodes for them.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DerivedEdge {
    pub edge_type: String,
    pub from_field: String,
    pub from_value: String,
    pub to_field: String,
    pub to_value: String,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

type FieldIndex<'a> = BTreeMap<&'a str, Vec<&'a str>>;

fn index_fields(fields: &[NormalizedField]) -> FieldIndex<'_> {
    let mut index: FieldIndex = BTreeMap::new();
    for field in fields {
        index
            .entry(field.field.as_str())
            .or_default()
            .push(field.normalized.as_str());
    }
    index
}

/// Apply every edge definition to one record's field-facts. For a definition
/// whose condition holds, emit one edge per (from-value, to-value) pair across
/// the two fields. A field-fact never edges to itself.
pub fn derive_edges(defs: &[EdgeDef], fields: &[NormalizedField]) -> Vec<DerivedEdge> {
    let index = index_fields(fields);
    let mut edges = Vec::new();
    for def in defs {
        if !def.condition.eval(&index) {
            continue;
        }
        let (Some(from_values), Some(to_values)) =
            (index.get(def.from_field.as_str()), index.get(def.to_field.as_str()))
        else {
            continue;
        };
        for from_value in from_values {
            for to_value in to_values {
                if def.from_field == def.to_field && from_value == to_value {
                    continue;
                }
                edges.push(DerivedEdge {
                    edge_type: def.edge_type.clone(),
                    from_field: def.from_field.clone(),
                    from_value: (*from_value).to_string(),
                    to_field: def.to_field.clone(),
                    to_value: (*to_value).to_string(),
                    version: def.version,
                    group: def.group.clone(),
                });
            }
        }
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::{FieldOrigin, FieldType, IndexPolicy};

    fn fact(field: &str, value: &str) -> NormalizedField {
        NormalizedField {
            field: field.to_string(),
            raw_value: value.to_string(),
            normalized: value.to_string(),
            group: None,
            visibility: None,
            masked: None,
            policy: IndexPolicy::INDEXED,
            field_type: FieldType::Text,
            origin: FieldOrigin::Extracted,
        }
    }

    #[test]
    fn derives_edge_between_two_fields() {
        let defs = vec![EdgeDef::new("CONNECTS", "src_ip", "dst_ip")];
        let fields = vec![fact("src_ip", "001"), fact("dst_ip", "002")];
        let edges = derive_edges(&defs, &fields);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, "CONNECTS");
        assert_eq!(edges[0].from_value, "001");
        assert_eq!(edges[0].to_value, "002");
        assert_eq!(edges[0].version, 1);
    }

    #[test]
    fn condition_gates_edge() {
        let defs = vec![EdgeDef::new("CONNECTS", "src_ip", "dst_ip")
            .when(EdgeCondition::FieldEquals { field: "proto".into(), value: "tcp".into() })];
        let without = vec![fact("src_ip", "001"), fact("dst_ip", "002")];
        assert!(derive_edges(&defs, &without).is_empty());

        let mut with = without.clone();
        with.push(fact("proto", "tcp"));
        assert_eq!(derive_edges(&defs, &with).len(), 1);
    }

    #[test]
    fn boolean_composition_gates_edges() {
        let def = |cond| vec![EdgeDef::new("E", "a", "b").when(cond)];
        let fields = vec![fact("a", "1"), fact("b", "2"), fact("proto", "tcp")];

        // proto == tcp AND NOT (proto == udp) -> fires.
        let cond = EdgeCondition::All {
            conditions: vec![
                EdgeCondition::FieldEquals { field: "proto".into(), value: "tcp".into() },
                EdgeCondition::Not {
                    condition: Box::new(EdgeCondition::FieldEquals { field: "proto".into(), value: "udp".into() }),
                },
            ],
        };
        assert_eq!(derive_edges(&def(cond), &fields).len(), 1);

        // proto == udp OR field missing -> Any with one true leaf fires.
        let any = EdgeCondition::Any {
            conditions: vec![
                EdgeCondition::FieldEquals { field: "proto".into(), value: "udp".into() },
                EdgeCondition::FieldNotEquals { field: "proto".into(), value: "udp".into() },
            ],
        };
        assert_eq!(derive_edges(&def(any), &fields).len(), 1);

        // All with a failing leaf -> gated off.
        let blocked = EdgeCondition::All {
            conditions: vec![EdgeCondition::FieldEquals { field: "proto".into(), value: "udp".into() }],
        };
        assert!(derive_edges(&def(blocked), &fields).is_empty());
    }

    #[test]
    fn no_self_edge_when_fields_share_a_value() {
        let defs = vec![EdgeDef::new("ALIAS", "name", "name")];
        let fields = vec![fact("name", "a"), fact("name", "b")];
        let edges = derive_edges(&defs, &fields);
        // a-b and b-a, but never a-a or b-b.
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|e| e.from_value != e.to_value));
    }
}
