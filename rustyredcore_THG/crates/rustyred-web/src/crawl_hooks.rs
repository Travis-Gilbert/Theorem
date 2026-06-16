//! Hook 3 from the spec: a self-organizing crawl frontier.
//!
//! These turn the periodic frontier prioritizer into an event-driven one that
//! fires *inside* the store as pages are fetched and links are discovered:
//!
//! - [`fetch_completion_hook`]: when a `url` node flips to `state=fetched`,
//!   classify the source, extract entities from its content snapshot into the
//!   graph, and recompute frontier priorities over the link graph. This is the
//!   `PprPrioritizer.recompute` from the crawl-frontier spec, now event-driven
//!   instead of periodic, so each fetched page immediately changes what the
//!   crawler does next.
//! - [`link_discovery_hook`]: when a `links_to` edge appears, set the newly
//!   discovered url node's initial PPR-seeded priority so it lands in the
//!   frontier at the right place.
//!
//! All work is synchronous and graph-native: it writes durable `priority`/
//! `source_class` node properties and entity nodes/edges. The async in-memory
//! frontier queue re-syncs from these durable priorities on its next load, so
//! the hooks never touch the tokio queue directly.

use std::collections::BTreeMap;
use std::sync::Arc;

use rustyred_thg_core::{
    EdgeRecord, HookContext, HookDispatcher, HookDispatcherConfig, HookError, HookHandler,
    HookOutcome, HookRegistration, HookStoreAccess, MutationEvent, MutationKind, MutationMatcher,
    NodeRecord, PluginCapability, PluginCapabilityKind, RedCoreGraphStore, RustyRedPlugin,
};
use serde_json::{json, Value};
use url::Url;

use crate::frontier::model::{
    EDGE_LINKS_TO, LABEL_URL, STATE_FETCHED, STATE_FRONTIER, UrlFingerprint,
};
use crate::frontier::{FrontierCtx, PprPrioritizer, Prioritizer, SharedFrontierStore};
use crate::source_class::classify_url;

/// Entity nodes/edges the fetch-completion hook materializes from page text.
pub const WEB_ENTITY_LABEL: &str = "WebEntity";
pub const EDGE_MENTIONS: &str = "mentions";

/// Bound on entities extracted per page so a content storm stays cheap.
const MAX_ENTITIES_PER_PAGE: usize = 16;
const MIN_ENTITY_LEN: usize = 3;

/// All crawl hook registrations, for `rustyred-web`'s plugin `hooks()`.
pub fn crawl_hooks() -> Vec<HookRegistration> {
    vec![fetch_completion_hook(), link_discovery_hook()]
}

/// Ships the crawl hooks as a `RustyRedPlugin`, so an embedder registers them
/// through the same `PluginRegistry::hooks()` path as every other plugin.
#[derive(Clone, Debug, Default)]
pub struct RustyWebHooksPlugin;

impl RustyRedPlugin for RustyWebHooksPlugin {
    fn name(&self) -> &'static str {
        "rustyred.web.hooks"
    }

    fn capabilities(&self) -> Vec<PluginCapability> {
        vec![PluginCapability {
            kind: PluginCapabilityKind::Hook,
            name: "web.frontier.self_organizing".to_string(),
        }]
    }

    fn hooks(&self) -> Vec<HookRegistration> {
        crawl_hooks()
    }
}

/// Bridges the (tokio-async) frontier store to the (sync, tokio-free) hook
/// dispatcher. The dispatcher worker runs on a dedicated `std::thread`, never a
/// tokio runtime worker, so `blocking_lock` here is safe and never stalls the
/// async runtime. tokio's `Mutex` is designed for exactly this mixed sync/async
/// use and cannot be poisoned, so there is no `unwrap` on a lock result.
struct FrontierHookStore(SharedFrontierStore);

impl HookStoreAccess for FrontierHookStore {
    fn with_store_mut(&self, f: &mut dyn FnMut(&mut RedCoreGraphStore)) -> bool {
        let mut guard = self.0.blocking_lock();
        f(&mut guard);
        true
    }
}

/// Start the self-organizing crawl hooks over a shared frontier store and attach
/// the emitter so crawl commits fire them event-driven. The returned dispatcher
/// must be kept alive for the crawl's lifetime (its worker stops on drop). Async
/// because attaching the emitter takes the frontier store's async lock.
pub async fn attach_crawl_hooks(
    store: SharedFrontierStore,
    tenant: impl Into<String>,
) -> HookDispatcher {
    let dispatcher = HookDispatcher::start(
        FrontierHookStore(Arc::clone(&store)),
        crawl_hooks(),
        HookDispatcherConfig::default(),
    );
    {
        let mut guard = store.lock().await;
        guard.attach_hook_emitter(dispatcher.emitter());
        guard.set_hook_tenant(tenant);
    }
    dispatcher
}

// ---- FetchCompletionHook ------------------------------------------------

pub fn fetch_completion_hook() -> HookRegistration {
    let handler: HookHandler = Arc::new(fetch_completion_handler);
    HookRegistration::new(
        "web.fetch_completion",
        MutationMatcher::any()
            .with_kinds([MutationKind::NodeUpserted])
            .with_labels([LABEL_URL])
            .with_changed_props_any(["state"]),
        // Funnel: one frontier recompute per batch, not per fetched page.
        coalesce_web_fetch,
        handler,
    )
}

fn coalesce_web_fetch(_event: &MutationEvent) -> Option<String> {
    Some("web-fetch-completion".to_string())
}

fn fetch_completion_handler(
    ctx: &mut HookContext,
    events: &[MutationEvent],
) -> Result<HookOutcome, HookError> {
    // Resolve which url nodes actually transitioned to `fetched` (the event
    // only tells us `state` changed, not its new value).
    let mut fetched: Vec<String> = Vec::new();
    for event in events {
        if event.kind != MutationKind::NodeUpserted {
            continue;
        }
        let Some(node) = ctx.store.get_node(&event.id).map_err(HookError::from)? else {
            continue;
        };
        let is_fetched = node
            .properties
            .get("state")
            .and_then(Value::as_str)
            .map(|state| state == STATE_FETCHED)
            .unwrap_or(false);
        if is_fetched {
            fetched.push(event.id.clone());
        }
    }
    if fetched.is_empty() {
        return Ok(HookOutcome::Done);
    }

    let mut writes = 0usize;
    // Per fetched page: classify its source and extract entities into the graph.
    for url_id in &fetched {
        writes += classify_and_extract(ctx.store, url_id)?;
    }

    // Recompute frontier priorities over the whole link graph and write them
    // onto the pending (frontier-state) url nodes, reordering the crawl.
    writes += recompute_frontier_priorities(ctx)?;

    Ok(HookOutcome::Wrote { mutations: writes })
}

/// Classify the fetched page's source and materialize entities from its content
/// snapshot (`content_snapshot:<content_hash>` -> `text`) into the graph as
/// `WebEntity` nodes with `MENTIONS` edges from the page.
fn classify_and_extract(
    store: &mut RedCoreGraphStore,
    url_id: &str,
) -> Result<usize, HookError> {
    let Some(mut node) = store.get_node(url_id).map_err(HookError::from)? else {
        return Ok(0);
    };
    let mut writes = 0usize;

    // 1. Source classification (URL-based, deterministic).
    if let Some(url_str) = node.properties.get("url").and_then(Value::as_str) {
        if let Ok(url) = Url::parse(url_str) {
            let class = classify_url(&url).as_str().to_string();
            let already = node
                .properties
                .get("source_class")
                .and_then(Value::as_str)
                .map(|existing| existing == class)
                .unwrap_or(false);
            if !already {
                if let Some(map) = node.properties.as_object_mut() {
                    map.insert("source_class".to_string(), json!(class));
                }
                store.upsert_node(node.clone()).map_err(HookError::from)?;
                writes += 1;
            }
        }
    }

    // 2. Entity extraction from the page's content snapshot, if present.
    let content_hash = node
        .properties
        .get("content_hash")
        .and_then(Value::as_str)
        .filter(|hash| !hash.is_empty());
    let Some(content_hash) = content_hash else {
        return Ok(writes);
    };
    let snapshot_id = format!("content_snapshot:{content_hash}");
    let Some(snapshot) = store.get_node(&snapshot_id).map_err(HookError::from)? else {
        return Ok(writes);
    };
    let text = snapshot
        .properties
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(writes);
    }

    for entity in extract_entities(text, MAX_ENTITIES_PER_PAGE) {
        let slug = entity_slug(&entity);
        let entity_id = format!("web:entity:{slug}");
        // Idempotent upsert of the entity node.
        if store.get_node(&entity_id).map_err(HookError::from)?.is_none() {
            store
                .upsert_node(NodeRecord::new(
                    &entity_id,
                    [WEB_ENTITY_LABEL],
                    json!({ "name": entity, "slug": slug }),
                ))
                .map_err(HookError::from)?;
            writes += 1;
        }
        let edge_id = format!("{EDGE_MENTIONS}:{url_id}:{slug}");
        if store.get_edge(&edge_id).map_err(HookError::from)?.is_none() {
            store
                .upsert_edge(EdgeRecord::new(
                    &edge_id,
                    url_id,
                    EDGE_MENTIONS,
                    &entity_id,
                    json!({}),
                ))
                .map_err(HookError::from)?;
            writes += 1;
        }
    }
    Ok(writes)
}

/// Run the crawl-frontier `PprPrioritizer.recompute` over the link graph and
/// write the resulting scores onto the pending (`frontier`-state) url nodes.
fn recompute_frontier_priorities(ctx: &mut HookContext) -> Result<usize, HookError> {
    // Compute scores under an immutable borrow, collect, then write.
    let scores: Vec<(UrlFingerprint, f64)> = {
        let frontier_ctx = FrontierCtx {
            store: &*ctx.store,
            tenant: ctx.tenant,
        };
        PprPrioritizer::default()
            .recompute(&frontier_ctx)
            .map_err(HookError::from)?
    };

    let mut writes = 0usize;
    for (fp, score) in scores {
        let node_id = fp.to_hex();
        let Some(mut node) = ctx.store.get_node(&node_id).map_err(HookError::from)? else {
            continue;
        };
        // Only reorder pending work; fetched/in-flight nodes are not re-popped.
        let is_frontier = node
            .properties
            .get("state")
            .and_then(Value::as_str)
            .map(|state| state == STATE_FRONTIER)
            .unwrap_or(false);
        if !is_frontier {
            continue;
        }
        if !set_priority_if_changed(&mut node, score) {
            continue;
        }
        ctx.store.upsert_node(node).map_err(HookError::from)?;
        writes += 1;
    }
    Ok(writes)
}

// ---- LinkDiscoveryHook --------------------------------------------------

pub fn link_discovery_hook() -> HookRegistration {
    let handler: HookHandler = Arc::new(link_discovery_handler);
    HookRegistration::new(
        "web.link_discovery",
        MutationMatcher::any()
            .with_kinds([MutationKind::EdgeUpserted])
            .with_labels([EDGE_LINKS_TO]),
        coalesce_web_links,
        handler,
    )
}

fn coalesce_web_links(_event: &MutationEvent) -> Option<String> {
    Some("web-link-discovery".to_string())
}

fn link_discovery_handler(
    ctx: &mut HookContext,
    events: &[MutationEvent],
) -> Result<HookOutcome, HookError> {
    // Resolve newly-linked target url nodes.
    let mut targets: Vec<String> = Vec::new();
    for event in events {
        if event.kind != MutationKind::EdgeUpserted {
            continue;
        }
        if let Ok(Some(edge)) = ctx.store.get_edge(&event.id) {
            targets.push(edge.to_id);
        }
    }
    if targets.is_empty() {
        return Ok(HookOutcome::Done);
    }

    // PPR-seeded priorities over the link graph (one recompute for the batch).
    let scores: BTreeMap<String, f64> = {
        let frontier_ctx = FrontierCtx {
            store: &*ctx.store,
            tenant: ctx.tenant,
        };
        PprPrioritizer::default()
            .recompute(&frontier_ctx)
            .map_err(HookError::from)?
            .into_iter()
            .map(|(fp, score)| (fp.to_hex(), score))
            .collect()
    };

    let mut writes = 0usize;
    for target_id in targets {
        let Some(mut node) = ctx.store.get_node(&target_id).map_err(HookError::from)? else {
            continue;
        };
        if !node.labels.iter().any(|label| label == LABEL_URL) {
            continue;
        }
        // PPR score when available, else a small depth-discounted default so a
        // freshly-linked node still enters the frontier with a sane priority.
        let depth = node.properties.get("depth").and_then(Value::as_u64).unwrap_or(0);
        let score = scores
            .get(&target_id)
            .copied()
            .unwrap_or_else(|| 1.0 / (depth as f64 + 1.0));
        if !set_priority_if_changed(&mut node, score) {
            continue;
        }
        ctx.store.upsert_node(node).map_err(HookError::from)?;
        writes += 1;
    }
    Ok(HookOutcome::Wrote { mutations: writes })
}

// ---- helpers ------------------------------------------------------------

/// Set the `priority` property, returning false (skip the write) when unchanged
/// so a self-induced re-trigger does no work.
fn set_priority_if_changed(node: &mut NodeRecord, priority: f64) -> bool {
    let rounded = (priority * 1_000_000.0).round() / 1_000_000.0;
    let prior = node.properties.get("priority").and_then(Value::as_f64);
    if let Some(prior) = prior {
        if (prior - rounded).abs() <= 1e-9 {
            return false;
        }
    }
    match node.properties.as_object_mut() {
        Some(map) => {
            map.insert("priority".to_string(), json!(rounded));
        }
        None => {
            node.properties = json!({ "priority": rounded });
        }
    }
    true
}

/// Lightweight, deterministic entity extraction: capitalized terms ranked by
/// frequency, deduped, capped. A real NER model can swap in behind this seam.
fn extract_entities(text: &str, cap: usize) -> Vec<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        if token.len() < MIN_ENTITY_LEN {
            continue;
        }
        let mut chars = token.chars();
        let first = chars.next().unwrap();
        if !first.is_uppercase() {
            continue;
        }
        if is_stopword(token) {
            continue;
        }
        *counts.entry(token.to_string()).or_default() += 1;
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    // Highest frequency first, then lexicographic for determinism.
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.into_iter().take(cap).map(|(term, _)| term).collect()
}

fn entity_slug(entity: &str) -> String {
    entity
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn is_stopword(token: &str) -> bool {
    const STOPWORDS: [&str; 12] = [
        "The", "This", "That", "These", "Those", "And", "But", "For", "Not", "With", "From", "Page",
    ];
    STOPWORDS.contains(&token)
}
