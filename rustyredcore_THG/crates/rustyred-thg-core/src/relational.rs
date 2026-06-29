use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::access_method::{
    AccessMethodRegistry, ColumnId, RankingRegistry, RelationId, RowChange, RowChangeKind, RowId,
    ScalarValue,
};
use crate::graph_store::{GraphSnapshot, GraphStoreError, GraphStoreResult};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ColumnSchema {
    pub name: ColumnId,
    pub nullable: bool,
    pub indexed: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RelationSchema {
    pub relation: RelationId,
    pub columns: Vec<ColumnSchema>,
    #[serde(default)]
    pub graph_backed: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RelationalRow {
    pub id: RowId,
    pub relation: RelationId,
    pub values: BTreeMap<ColumnId, ScalarValue>,
    #[serde(default)]
    pub properties: Value,
}

impl RelationalRow {
    pub fn new(
        relation: impl Into<String>,
        id: impl Into<String>,
        values: BTreeMap<ColumnId, ScalarValue>,
    ) -> Self {
        Self {
            id: id.into(),
            relation: relation.into(),
            values,
            properties: json!({}),
        }
    }

    pub fn value(&self, column: &str) -> Option<&ScalarValue> {
        if column == "id" {
            return None;
        }
        self.values.get(column)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Relation {
    pub schema: RelationSchema,
    rows: BTreeMap<RowId, RelationalRow>,
}

impl Relation {
    pub fn new(schema: RelationSchema) -> Self {
        Self {
            schema,
            rows: BTreeMap::new(),
        }
    }

    pub fn upsert(&mut self, row: RelationalRow) {
        self.rows.insert(row.id.clone(), row);
    }

    pub fn get(&self, id: &str) -> Option<&RelationalRow> {
        self.rows.get(id)
    }

    pub fn rows(&self) -> impl Iterator<Item = &RelationalRow> {
        self.rows.values()
    }

    pub fn row_ids(&self) -> Vec<RowId> {
        self.rows.keys().cloned().collect()
    }
}

#[derive(Default)]
pub struct RelationalStore {
    relations: BTreeMap<RelationId, Relation>,
    access_methods: AccessMethodRegistry,
    ranking_methods: RankingRegistry,
}

impl RelationalStore {
    pub fn new() -> Self {
        Self {
            relations: BTreeMap::new(),
            access_methods: AccessMethodRegistry::with_native_defaults(),
            ranking_methods: RankingRegistry::with_native_defaults(),
        }
    }

    pub fn with_access_methods(access_methods: AccessMethodRegistry) -> Self {
        Self {
            relations: BTreeMap::new(),
            access_methods,
            ranking_methods: RankingRegistry::with_native_defaults(),
        }
    }

    pub fn access_methods(&self) -> &AccessMethodRegistry {
        &self.access_methods
    }

    pub fn ranking_methods(&self) -> &RankingRegistry {
        &self.ranking_methods
    }

    pub fn create_relation(&mut self, schema: RelationSchema) {
        self.relations
            .entry(schema.relation.clone())
            .or_insert_with(|| Relation::new(schema));
    }

    pub fn upsert_row(&mut self, row: RelationalRow) -> GraphStoreResult<()> {
        if row.id.trim().is_empty() || row.relation.trim().is_empty() {
            return Err(GraphStoreError::new(
                "invalid_relational_row",
                "relation and row id must be non-empty",
            ));
        }
        let relation_id = row.relation.clone();
        if let Some(existing) = self
            .relations
            .get(&relation_id)
            .and_then(|relation| relation.get(&row.id))
            .cloned()
        {
            self.access_methods.on_write(&RowChange {
                relation: relation_id.clone(),
                row_id: existing.id,
                values: existing.values,
                properties: existing.properties,
                kind: RowChangeKind::Delete,
            })?;
        }
        self.relations
            .entry(relation_id.clone())
            .or_insert_with(|| Relation::new(inferred_schema(&relation_id, &row)));
        self.access_methods.on_write(&RowChange {
            relation: relation_id.clone(),
            row_id: row.id.clone(),
            values: row.values.clone(),
            properties: row.properties.clone(),
            kind: RowChangeKind::Upsert,
        })?;
        if let Some(relation) = self.relations.get_mut(&relation_id) {
            relation.upsert(row);
        }
        Ok(())
    }

    pub fn delete_row(&mut self, relation: &str, row_id: &str) -> GraphStoreResult<bool> {
        let Some(existing) = self
            .relations
            .get(relation)
            .and_then(|relation| relation.get(row_id))
            .cloned()
        else {
            return Ok(false);
        };
        self.access_methods.on_write(&RowChange {
            relation: relation.to_string(),
            row_id: existing.id.clone(),
            values: existing.values,
            properties: existing.properties,
            kind: RowChangeKind::Delete,
        })?;
        if let Some(relation) = self.relations.get_mut(relation) {
            relation.rows.remove(row_id);
        }
        Ok(true)
    }

    pub fn drop_relation(&mut self, relation: &str) -> GraphStoreResult<bool> {
        let Some(existing) = self.relations.remove(relation) else {
            return Ok(false);
        };
        for row in existing.rows.into_values() {
            self.access_methods.on_write(&RowChange {
                relation: relation.to_string(),
                row_id: row.id,
                values: row.values,
                properties: row.properties,
                kind: RowChangeKind::Delete,
            })?;
        }
        Ok(true)
    }

    pub fn replace_relation(
        &mut self,
        schema: RelationSchema,
        rows: Vec<RelationalRow>,
    ) -> GraphStoreResult<()> {
        self.drop_relation(&schema.relation)?;
        self.create_relation(schema);
        for row in rows {
            self.upsert_row(row)?;
        }
        Ok(())
    }

    pub fn rename_relation(&mut self, from: &str, to: &str) -> GraphStoreResult<bool> {
        let Some(existing) = self.relations.get(from).cloned() else {
            return Ok(false);
        };
        self.drop_relation(from)?;
        let mut schema = existing.schema;
        schema.relation = to.to_string();
        self.create_relation(schema);
        for mut row in existing.rows.into_values() {
            row.relation = to.to_string();
            self.upsert_row(row)?;
        }
        Ok(true)
    }

    pub fn relation(&self, relation: &str) -> Option<&Relation> {
        self.relations.get(relation)
    }

    pub fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.relations.values()
    }

    pub fn from_graph_snapshot(snapshot: &GraphSnapshot) -> GraphStoreResult<Self> {
        let mut store = Self::new();
        store.create_relation(RelationSchema {
            relation: "nodes".to_string(),
            graph_backed: true,
            columns: vec![
                column("id", true),
                column("label", true),
                column("content_hash", false),
            ],
        });
        store.create_relation(RelationSchema {
            relation: "edges".to_string(),
            graph_backed: true,
            columns: vec![
                column("id", true),
                column("from_id", true),
                column("to_id", true),
                column("type", true),
            ],
        });
        for node in &snapshot.nodes {
            if node.tombstone {
                continue;
            }
            let primary_label = node.labels.first().cloned().unwrap_or_default();
            let mut values = BTreeMap::from([
                ("id".to_string(), ScalarValue::String(node.id.clone())),
                (
                    "label".to_string(),
                    ScalarValue::String(primary_label.clone()),
                ),
            ]);
            if let Some(hash) = &node.content_hash {
                values.insert(
                    "content_hash".to_string(),
                    ScalarValue::String(hash.clone()),
                );
            }
            if let Some(map) = node.properties.as_object() {
                for (key, value) in map {
                    if let Some(value) = ScalarValue::from_json(value) {
                        values.insert(key.clone(), value);
                    }
                }
            }
            let mut row = RelationalRow::new("nodes", node.id.clone(), values.clone());
            row.properties = node.properties.clone();
            store.upsert_row(row)?;
            for label in &node.labels {
                let relation = label.to_ascii_lowercase();
                store.upsert_row(RelationalRow {
                    id: node.id.clone(),
                    relation,
                    values: values.clone(),
                    properties: node.properties.clone(),
                })?;
            }
        }
        for edge in &snapshot.edges {
            if edge.tombstone {
                continue;
            }
            let mut values = BTreeMap::from([
                ("id".to_string(), ScalarValue::String(edge.id.clone())),
                (
                    "from_id".to_string(),
                    ScalarValue::String(edge.from_id.clone()),
                ),
                ("to_id".to_string(), ScalarValue::String(edge.to_id.clone())),
                (
                    "type".to_string(),
                    ScalarValue::String(edge.edge_type.clone()),
                ),
            ]);
            if let Some(map) = edge.properties.as_object() {
                for (key, value) in map {
                    if let Some(value) = ScalarValue::from_json(value) {
                        values.insert(key.clone(), value);
                    }
                }
            }
            let mut row = RelationalRow::new("edges", edge.id.clone(), values.clone());
            row.properties = edge.properties.clone();
            store.upsert_row(row)?;
            store.upsert_row(RelationalRow {
                id: edge.id.clone(),
                relation: edge.edge_type.to_ascii_lowercase(),
                values,
                properties: edge.properties.clone(),
            })?;
        }
        Ok(store)
    }
}

fn inferred_schema(relation: &str, row: &RelationalRow) -> RelationSchema {
    RelationSchema {
        relation: relation.to_string(),
        graph_backed: false,
        columns: row
            .values
            .keys()
            .map(|name| column(name, true))
            .collect::<Vec<_>>(),
    }
}

fn column(name: &str, indexed: bool) -> ColumnSchema {
    ColumnSchema {
        name: name.to_string(),
        nullable: true,
        indexed,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeTenantRecord {
    pub tenant_id: String,
    pub slug: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeProjectRecord {
    pub tenant_id: String,
    pub project_slug: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeBillingAccountRecord {
    pub tenant_id: String,
    pub plan: String,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeAuthPrincipalRecord {
    pub principal_id: String,
    pub tenant_id: String,
    pub kind: String,
    pub token_hash: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct NativeCatalog {
    tenants: BTreeMap<String, NativeTenantRecord>,
    tenant_slugs: BTreeMap<String, String>,
    projects: BTreeMap<(String, String), NativeProjectRecord>,
    billing: BTreeMap<String, NativeBillingAccountRecord>,
    principals: BTreeMap<String, NativeAuthPrincipalRecord>,
    principal_tenants: BTreeMap<String, BTreeSet<String>>,
}

impl NativeCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_tenant(&mut self, tenant: NativeTenantRecord) -> GraphStoreResult<()> {
        require_non_empty(&tenant.tenant_id, "tenant_id")?;
        require_non_empty(&tenant.slug, "slug")?;
        if let Some(existing) = self.tenant_slugs.get(&tenant.slug) {
            if existing != &tenant.tenant_id {
                return Err(GraphStoreError::new(
                    "catalog_unique_violation",
                    format!(
                        "tenant slug {} already belongs to {}",
                        tenant.slug, existing
                    ),
                ));
            }
        }
        self.tenant_slugs
            .insert(tenant.slug.clone(), tenant.tenant_id.clone());
        self.tenants.insert(tenant.tenant_id.clone(), tenant);
        Ok(())
    }

    pub fn upsert_project(&mut self, project: NativeProjectRecord) -> GraphStoreResult<()> {
        self.require_tenant(&project.tenant_id)?;
        require_non_empty(&project.project_slug, "project_slug")?;
        self.projects.insert(
            (project.tenant_id.clone(), project.project_slug.clone()),
            project,
        );
        Ok(())
    }

    pub fn upsert_billing_account(
        &mut self,
        account: NativeBillingAccountRecord,
    ) -> GraphStoreResult<()> {
        self.require_tenant(&account.tenant_id)?;
        require_non_empty(&account.plan, "plan")?;
        require_non_empty(&account.status, "status")?;
        self.billing.insert(account.tenant_id.clone(), account);
        Ok(())
    }

    pub fn upsert_auth_principal(
        &mut self,
        principal: NativeAuthPrincipalRecord,
    ) -> GraphStoreResult<()> {
        self.require_tenant(&principal.tenant_id)?;
        require_non_empty(&principal.principal_id, "principal_id")?;
        require_non_empty(&principal.kind, "kind")?;
        let previous_tenant = self
            .principals
            .get(&principal.principal_id)
            .map(|record| record.tenant_id.clone());
        if let Some(previous_tenant) = previous_tenant {
            if let Some(ids) = self.principal_tenants.get_mut(&previous_tenant) {
                ids.remove(&principal.principal_id);
            }
        }
        self.principal_tenants
            .entry(principal.tenant_id.clone())
            .or_default()
            .insert(principal.principal_id.clone());
        self.principals
            .insert(principal.principal_id.clone(), principal);
        Ok(())
    }

    pub fn tenants(&self) -> Vec<NativeTenantRecord> {
        self.tenants.values().cloned().collect()
    }

    pub fn projects_for_tenant(&self, tenant_id: &str) -> Vec<NativeProjectRecord> {
        self.projects
            .iter()
            .filter(|((tenant, _), _)| tenant == tenant_id)
            .map(|(_, project)| project.clone())
            .collect()
    }

    pub fn auth_principals_for_tenant(&self, tenant_id: &str) -> Vec<NativeAuthPrincipalRecord> {
        self.principal_tenants
            .get(tenant_id)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| self.principals.get(id).cloned())
            .collect()
    }

    fn require_tenant(&self, tenant_id: &str) -> GraphStoreResult<()> {
        require_non_empty(tenant_id, "tenant_id")?;
        if !self.tenants.contains_key(tenant_id) {
            return Err(GraphStoreError::new(
                "catalog_foreign_key_violation",
                format!("tenant {tenant_id} does not exist"),
            ));
        }
        Ok(())
    }
}

fn require_non_empty(value: &str, field: &str) -> GraphStoreResult<()> {
    if value.trim().is_empty() {
        Err(GraphStoreError::new(
            "catalog_empty_field",
            format!("{field} must not be empty"),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_catalog_enforces_foreign_keys_and_uniqueness() {
        let mut catalog = NativeCatalog::new();
        let missing_tenant = catalog.upsert_project(NativeProjectRecord {
            tenant_id: "tenant:missing".to_string(),
            project_slug: "theorem".to_string(),
            display_name: None,
        });
        assert_eq!(
            missing_tenant.unwrap_err().code,
            "catalog_foreign_key_violation"
        );

        catalog
            .upsert_tenant(NativeTenantRecord {
                tenant_id: "tenant:a".to_string(),
                slug: "a".to_string(),
                display_name: Some("Tenant A".to_string()),
            })
            .unwrap();
        catalog
            .upsert_project(NativeProjectRecord {
                tenant_id: "tenant:a".to_string(),
                project_slug: "theorem".to_string(),
                display_name: Some("Theorem".to_string()),
            })
            .unwrap();
        catalog
            .upsert_billing_account(NativeBillingAccountRecord {
                tenant_id: "tenant:a".to_string(),
                plan: "pro".to_string(),
                status: "active".to_string(),
            })
            .unwrap();
        catalog
            .upsert_auth_principal(NativeAuthPrincipalRecord {
                principal_id: "principal:1".to_string(),
                tenant_id: "tenant:a".to_string(),
                kind: "api_key".to_string(),
                token_hash: Some("sha256:redacted".to_string()),
                scopes: vec!["memory:read".to_string()],
            })
            .unwrap();

        assert_eq!(catalog.projects_for_tenant("tenant:a").len(), 1);
        assert_eq!(catalog.auth_principals_for_tenant("tenant:a").len(), 1);

        let duplicate_slug = catalog.upsert_tenant(NativeTenantRecord {
            tenant_id: "tenant:b".to_string(),
            slug: "a".to_string(),
            display_name: None,
        });
        assert_eq!(duplicate_slug.unwrap_err().code, "catalog_unique_violation");
    }
}
