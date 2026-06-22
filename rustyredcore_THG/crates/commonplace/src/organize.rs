//! The two-tier organize boundary (Layer B) plus explicit source routing.
//!
//! Tier one is the engine: one embedding and a cosine ranking over collections,
//! no model call, on every item. Tier two is the agent, and it touches only what
//! tier one declines. [`decide`] is the dial between them, applied to
//! [`Classification::confidence`](crate::ingest::Classification::confidence); the
//! bounded "needs you" set it produces ([`OrganizeDecision::NeedsYou`]) is the
//! tier-two queue. The moat lives in this boundary: tier one runs on all items
//! with no model call, and the agent budget is spent only on `NeedsYou`.
//!
//! Routing ([`RoutingRule`]) layers on top: an explicit rule hard-routes a
//! source-and-container to a collection regardless of cosine (B1). The soft
//! source prior is the separate
//! [`classify_item_with_source_prior`](crate::ingest::IngestPipeline::classify_item_with_source_prior).

use rustyred_thg_core::GraphStoreResult;

use crate::blob::BlobStore;
use crate::ingest::{classify_item_ranking, ClassificationRank, EmbeddingGraphStore};
use crate::item::Item;
use crate::store::Commonplace;

/// The trust-versus-precision dial: the two cut points plus the ambiguity margin
/// (open fork 2). `review_floor` is anchored on F2's existing 0.58 auto-file
/// gate; `auto_ceiling` is the higher bar for filing silently.
#[derive(Clone, Copy, Debug)]
pub struct OrganizePolicy {
    /// At or above this score (and unambiguous), file silently.
    pub auto_ceiling: f32,
    /// Between `review_floor` and `auto_ceiling`: filed, but shown for review.
    pub review_floor: f32,
    /// If the top two candidates are within this margin, the call is ambiguous
    /// and goes to "needs you" no matter how high the top score is.
    pub ambiguity_margin: f32,
}

impl Default for OrganizePolicy {
    fn default() -> Self {
        Self {
            auto_ceiling: 0.72,
            review_floor: 0.58,
            ambiguity_margin: 0.05,
        }
    }
}

/// The tier-one outcome for one item.
#[derive(Clone, Debug, PartialEq)]
pub enum OrganizeDecision {
    /// Tier one is confident and unambiguous. File silently.
    AutoFiled { collection_id: String, confidence: f32 },
    /// Tier one filed it but the call is close enough to show for review.
    /// Reversible, low-stakes, lands in "organized today".
    FiledForReview { collection_id: String, confidence: f32 },
    /// Tier one declined. The bounded "needs you" set: the tier-two queue,
    /// optionally carrying an agent suggestion downstream.
    NeedsYou {
        candidates: Vec<ClassificationRank>,
        reason: NeedsYouReason,
    },
}

/// Why an item landed in the "needs you" set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeedsYouReason {
    /// Top score below the file line.
    LowConfidence,
    /// Top two scores within the ambiguity margin.
    Ambiguous,
    /// No collection has a label embedding to match against yet.
    NoCandidates,
}

/// Decide which tier-one band an item falls in (B2). Pure cosine over stored
/// embeddings: NO model call, on every item. The three bands fall out of the
/// policy; `NeedsYou` carries the ranked candidates so a downstream agent has
/// the alternatives tier one could not separate.
pub fn decide<S, B>(
    commonplace: &Commonplace<S, B>,
    item: &Item,
    policy: &OrganizePolicy,
) -> GraphStoreResult<OrganizeDecision>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    let ranked = classify_item_ranking(commonplace, item)?.ranked;
    let Some(top) = ranked.first() else {
        return Ok(OrganizeDecision::NeedsYou {
            candidates: ranked,
            reason: NeedsYouReason::NoCandidates,
        });
    };
    let confidence = top.score;
    let collection_id = top.collection_id.clone();

    // Ambiguity sends an item to "needs you" no matter how high the top score is.
    let ambiguous = ranked
        .get(1)
        .map(|second| (confidence - second.score) < policy.ambiguity_margin)
        .unwrap_or(false);
    if ambiguous {
        return Ok(OrganizeDecision::NeedsYou {
            candidates: ranked,
            reason: NeedsYouReason::Ambiguous,
        });
    }

    if confidence >= policy.auto_ceiling {
        Ok(OrganizeDecision::AutoFiled {
            collection_id,
            confidence,
        })
    } else if confidence >= policy.review_floor {
        Ok(OrganizeDecision::FiledForReview {
            collection_id,
            confidence,
        })
    } else {
        Ok(OrganizeDecision::NeedsYou {
            candidates: ranked,
            reason: NeedsYouReason::LowConfidence,
        })
    }
}

/// An explicit routing rule (B1): pin a source-and-container to a collection.
/// Rules live tenant-scoped in the catalog (F3) and are read at ingest; this is
/// the in-memory shape the driver matches against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingRule {
    /// The `Item.source` this rule applies to (e.g. "gmail", "linear").
    pub source: String,
    /// If set, the rule only matches records from this container (a Gmail label,
    /// a Linear team, a Notion database). `None` matches any container.
    pub container_match: Option<String>,
    /// The collection (by name) a matching record hard-routes into.
    pub collection: String,
}

impl RoutingRule {
    pub fn new(
        source: impl Into<String>,
        container_match: Option<String>,
        collection: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            container_match,
            collection: collection.into(),
        }
    }

    fn matches(&self, source: &str, container: Option<&str>) -> bool {
        if self.source != source {
            return false;
        }
        match (&self.container_match, container) {
            (None, _) => true,
            (Some(want), Some(have)) => want == have,
            (Some(_), None) => false,
        }
    }
}

/// The first matching rule for a `(source, container)`, if any. A match
/// hard-routes regardless of cosine (B1); pass `rule.collection` to
/// [`IngestPipeline::ingest_routed`](crate::ingest::IngestPipeline::ingest_routed).
pub fn route<'a>(
    rules: &'a [RoutingRule],
    source: &str,
    container: Option<&str>,
) -> Option<&'a RoutingRule> {
    rules.iter().find(|rule| rule.matches(source, container))
}
