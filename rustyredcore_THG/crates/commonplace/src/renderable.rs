//! Renderable object contract for CommonPlace surfaces.
//!
//! Every surface receives the same `RenderableObject` projection and chooses a
//! renderer by `object_type_slug`. Actions mutate graph objects, not surface
//! private state, so the Index, board, project lens, and compose lens share one
//! substrate.

use rustyred_thg_core::{GraphStore, GraphStoreError, GraphStoreResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::blob::BlobStore;
use crate::item::{Item, ItemBody, ItemKind};
use crate::store::Commonplace;

pub const OBJECT_TYPE_SLUGS: &[&str] = &[
    "source",
    "claim",
    "quote",
    "person",
    "note",
    "task",
    "issue",
    "cycle",
    "code-symbol",
    "draft",
    "artifact",
    "file",
    "link",
    "image",
    "doc",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectTypeIdentity {
    pub slug: String,
    pub label: String,
    pub default_renderer: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderableObject {
    pub id: String,
    pub object_type_slug: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub body_preview: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub collections: Vec<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizeActionVerb {
    File,
    Delegate,
    Draft,
    Develop,
}

impl OrganizeActionVerb {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Delegate => "delegate",
            Self::Draft => "draft",
            Self::Develop => "develop",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrganizeAction {
    pub verb: OrganizeActionVerb,
    pub object_id: String,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizeActionReceipt {
    pub verb: OrganizeActionVerb,
    pub object_id: String,
    #[serde(default)]
    pub created_object_id: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
    pub graph_transform: String,
}

pub fn get_object_type_identity(slug: &str) -> ObjectTypeIdentity {
    let clean = normalize_slug(slug);
    let label = match clean.as_str() {
        "code-symbol" => "Code symbol".to_string(),
        "doc" => "Document".to_string(),
        "file" => "File".to_string(),
        "image" => "Image".to_string(),
        "link" => "Link".to_string(),
        "note" => "Note".to_string(),
        "task" => "Task".to_string(),
        "source" => "Source".to_string(),
        "claim" => "Claim".to_string(),
        "quote" => "Quote".to_string(),
        "person" => "Person".to_string(),
        "issue" => "Issue".to_string(),
        "cycle" => "Cycle".to_string(),
        "draft" => "Draft".to_string(),
        "artifact" => "Artifact".to_string(),
        _ => title_from_slug(&clean),
    };
    ObjectTypeIdentity {
        slug: clean.clone(),
        label,
        default_renderer: format!("{clean}-renderer"),
    }
}

pub fn renderable_from_item(item: &Item) -> RenderableObject {
    let mut metadata = item.extra.clone();
    metadata.insert("kind".to_string(), json!(item.kind.as_str()));
    metadata.insert("residency".to_string(), json!(item.residency.as_str()));
    if let Some(status) = &item.status {
        metadata.insert("status".to_string(), json!(status));
    }
    if let Some(priority) = &item.priority {
        metadata.insert("priority".to_string(), json!(priority));
    }
    if let Some(due_at_ms) = item.due_at_ms {
        metadata.insert("due_at_ms".to_string(), json!(due_at_ms));
    }

    RenderableObject {
        id: item.id.clone(),
        object_type_slug: item_object_type_slug(item),
        title: item.title.clone(),
        summary: item.classification.clone().unwrap_or_default(),
        body_preview: body_preview(&item.body),
        source: item.source.clone(),
        tags: item.tags.clone(),
        collections: item.collections.clone(),
        created_at_ms: item.created_at_ms,
        updated_at_ms: item.updated_at_ms,
        metadata,
    }
}

pub fn renderables_from_items(items: &[Item]) -> Vec<RenderableObject> {
    items.iter().map(renderable_from_item).collect()
}

pub fn item_object_type_slug(item: &Item) -> String {
    normalize_slug(item.kind.as_str())
}

impl<S, B> Commonplace<S, B>
where
    S: GraphStore,
    B: BlobStore,
{
    pub fn get_renderable(&self, item_id: &str) -> GraphStoreResult<Option<RenderableObject>> {
        Ok(self
            .get_item(item_id)?
            .map(|item| renderable_from_item(&item)))
    }

    pub fn all_renderables(&self) -> GraphStoreResult<Vec<RenderableObject>> {
        Ok(renderables_from_items(&self.all_items()?))
    }

    pub fn apply_organize_action(
        &mut self,
        action: OrganizeAction,
    ) -> GraphStoreResult<OrganizeActionReceipt> {
        let object_id = action.object_id.trim();
        if object_id.is_empty() {
            return Err(input_error("organize action requires object_id"));
        }
        match action.verb {
            OrganizeActionVerb::File => {
                let target_id = required_target(&action, "file")?;
                self.add_to_collection(object_id, &target_id)?;
                Ok(OrganizeActionReceipt {
                    verb: action.verb,
                    object_id: object_id.to_string(),
                    created_object_id: None,
                    target_id: Some(target_id),
                    graph_transform: "IN_COLLECTION".to_string(),
                })
            }
            OrganizeActionVerb::Delegate => {
                let target_id = required_target(&action, "delegate")?;
                self.link_worked_by(object_id, &target_id)?;
                Ok(OrganizeActionReceipt {
                    verb: action.verb,
                    object_id: object_id.to_string(),
                    created_object_id: None,
                    target_id: Some(target_id),
                    graph_transform: "WORKED_BY".to_string(),
                })
            }
            OrganizeActionVerb::Draft => {
                let source = self.get_item(object_id)?.ok_or_else(|| {
                    input_error(format!("object not found for draft: {object_id}"))
                })?;
                let title = action_title(&action, "Draft", &source.title);
                let draft = self.put_item(
                    Item::new(ItemKind::Other("draft".to_string()), title)
                        .with_text(body_preview(&source.body)),
                )?;
                self.link_about(&draft.id, object_id)?;
                Ok(OrganizeActionReceipt {
                    verb: action.verb,
                    object_id: object_id.to_string(),
                    created_object_id: Some(draft.id),
                    target_id: None,
                    graph_transform: "Item(draft)+ABOUT".to_string(),
                })
            }
            OrganizeActionVerb::Develop => {
                let source = self.get_item(object_id)?.ok_or_else(|| {
                    input_error(format!("object not found for develop: {object_id}"))
                })?;
                let title = action_title(&action, "Develop", &source.title);
                let task = self.put_item(Item::task(title, body_preview(&source.body)))?;
                self.link_about(&task.id, object_id)?;
                Ok(OrganizeActionReceipt {
                    verb: action.verb,
                    object_id: object_id.to_string(),
                    created_object_id: Some(task.id),
                    target_id: None,
                    graph_transform: "Item(task)+ABOUT".to_string(),
                })
            }
        }
    }
}

fn body_preview(body: &ItemBody) -> String {
    match body {
        ItemBody::Empty => String::new(),
        ItemBody::Inline { text } => text.chars().take(240).collect(),
        ItemBody::Blob {
            content_hash,
            byte_len,
            mime,
        } => format!(
            "{} blob {} ({} bytes)",
            mime.as_deref().unwrap_or("content-addressed"),
            content_hash,
            byte_len
        ),
    }
}

fn action_title(action: &OrganizeAction, prefix: &str, source_title: &str) -> String {
    action
        .metadata
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{prefix}: {source_title}"))
}

fn required_target(action: &OrganizeAction, verb: &str) -> GraphStoreResult<String> {
    action
        .target_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| input_error(format!("{verb} action requires target_id")))
}

fn normalize_slug(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn title_from_slug(slug: &str) -> String {
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn input_error(message: impl Into<String>) -> GraphStoreError {
    GraphStoreError::new("commonplace_organize_action_invalid", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CollectionKind, InMemoryBlobStore};
    use rustyred_thg_core::InMemoryGraphStore;

    fn fresh() -> Commonplace<InMemoryGraphStore, InMemoryBlobStore> {
        Commonplace::new(InMemoryGraphStore::new(), InMemoryBlobStore::new())
    }

    #[test]
    fn projects_any_item_kind_to_renderable_slug() {
        let item = Item::new(ItemKind::Other("claim".to_string()), "A sourced claim")
            .with_text("The claim body")
            .with_tags(["evidence"]);

        let renderable = renderable_from_item(&item);

        assert_eq!(renderable.object_type_slug, "claim");
        assert_eq!(get_object_type_identity("claim").label, "Claim");
        assert_eq!(
            get_object_type_identity("new-surface-object").label,
            "New Surface Object"
        );
        assert_eq!(renderable.body_preview, "The claim body");
        assert_eq!(renderable.tags, vec!["evidence".to_string()]);
    }

    #[test]
    fn file_action_is_collection_graph_transform() {
        let mut cp = fresh();
        let collection = cp
            .create_collection("Evidence", CollectionKind::Manual)
            .unwrap();
        let item = cp.put_item(Item::note("Source note", "body")).unwrap();

        let receipt = cp
            .apply_organize_action(OrganizeAction {
                verb: OrganizeActionVerb::File,
                object_id: item.id.clone(),
                target_id: Some(collection.id.clone()),
                actor_id: "codex".to_string(),
                metadata: Map::new(),
            })
            .unwrap();

        assert_eq!(receipt.graph_transform, "IN_COLLECTION");
        assert_eq!(cp.collection_items(&collection.id).unwrap()[0].id, item.id);
    }

    #[test]
    fn draft_and_develop_actions_create_linked_objects() {
        let mut cp = fresh();
        let source = cp
            .put_item(Item::note("Observation", "needs work"))
            .unwrap();

        let draft = cp
            .apply_organize_action(OrganizeAction {
                verb: OrganizeActionVerb::Draft,
                object_id: source.id.clone(),
                target_id: None,
                actor_id: "codex".to_string(),
                metadata: Map::new(),
            })
            .unwrap();
        let draft_id = draft.created_object_id.unwrap();
        assert_eq!(
            cp.get_item(&draft_id).unwrap().unwrap().kind.as_str(),
            "draft"
        );
        assert_eq!(cp.task_about(&draft_id).unwrap(), vec![source.id.clone()]);

        let task = cp
            .apply_organize_action(OrganizeAction {
                verb: OrganizeActionVerb::Develop,
                object_id: source.id.clone(),
                target_id: None,
                actor_id: "codex".to_string(),
                metadata: Map::new(),
            })
            .unwrap();
        let task_id = task.created_object_id.unwrap();
        assert_eq!(cp.get_item(&task_id).unwrap().unwrap().kind, ItemKind::Task);
        assert_eq!(cp.task_about(&task_id).unwrap(), vec![source.id]);
    }
}
