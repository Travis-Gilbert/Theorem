//! Organize: the daily-driver triage surface over the consumer store.
//!
//! Reuses the engine's classification signal (cosine of an item's stored
//! embedding to each collection's label embedding) to partition items that
//! arrived in a timeframe into two piles: what the engine filed confidently
//! ("organized today") and what still needs a human ("needs you"). The
//! classification is engine-sourced via [`IngestPipeline::classify_item`], so
//! this surface ranks and presents; it never re-derives the embedding model.
//!
//! Scope notes (surfaced):
//! - "now" is wall-clock by default (`SystemTime::now()`), overridable on the
//!   config so tests are deterministic.
//! - `arrived_at` / `filed_at` are rendered to ISO-8601 UTC from the item's
//!   millisecond timestamps with an in-crate proleptic-Gregorian helper, so no
//!   date dependency is added (the crate has neither `chrono` nor `time`).
//! - "time sensitive" leans on the kind heuristic (email -> reply expected); a
//!   richer urgency model is a named follow-up.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use commonplace::{
    BlobStore, Classification, ClassificationRank, Commonplace, EmbeddingGraphStore,
    IngestPipeline, Item, ItemBody, ItemKind,
};
use rustyred_thg_core::GraphStoreResult;

const DAY_MS: i64 = 86_400_000;
const WEEK_MS: i64 = 7 * DAY_MS;
const MONTH_MS: i64 = 30 * DAY_MS;

/// The window an organize call considers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Timeframe {
    #[default]
    Day,
    Week,
    Month,
}

impl Timeframe {
    fn window_ms(self) -> i64 {
        match self {
            Timeframe::Day => DAY_MS,
            Timeframe::Week => WEEK_MS,
            Timeframe::Month => MONTH_MS,
        }
    }

    /// The token surfaced in `daily_progress.timeframe`.
    pub fn as_str(self) -> &'static str {
        match self {
            Timeframe::Day => "day",
            Timeframe::Week => "week",
            Timeframe::Month => "month",
        }
    }
}

impl From<&str> for Timeframe {
    fn from(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "week" => Timeframe::Week,
            "month" => Timeframe::Month,
            _ => Timeframe::Day,
        }
    }
}

/// Knobs for an organize call.
#[derive(Clone, Copy, Debug)]
pub struct OrganizeConfig {
    /// Confidence at or above which the engine files an item without a human.
    pub needs_you_ceiling: f32,
    pub timeframe: Timeframe,
    /// Cap on the returned `needs_you` vec (the full count still feeds progress).
    pub needs_you_cap: usize,
    /// "Now" in epoch millis; overridable for deterministic tests.
    pub now_ms: i64,
}

impl Default for OrganizeConfig {
    fn default() -> Self {
        Self {
            needs_you_ceiling: 0.58,
            timeframe: Timeframe::Day,
            needs_you_cap: 24,
            now_ms: now_ms(),
        }
    }
}

/// A checkbox line parsed out of an item's body: the actionable subtasks that
/// make an item a "task". Order is preserved from the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subtask {
    pub text: String,
    pub done: bool,
}

/// One arrived item, with the engine's classification verdict attached.
#[derive(Clone, Debug)]
pub struct OrganizeItem {
    pub item: Item,
    /// Derived presentation kind: "email" | "event" | "task" | "file" | "note".
    pub kind: String,
    /// First ~200 chars of body text, trimmed.
    pub preview: String,
    pub source: String,
    pub target_collection_id: Option<String>,
    pub target_collection_label: Option<String>,
    pub confidence: f32,
    /// Next-best candidate collections (up to 3), excluding the target.
    pub alternatives: Vec<(String, String)>,
    pub time_sensitive: bool,
    /// "reply" | "open" | None.
    pub expected_action: Option<String>,
    /// Checkbox subtasks parsed from the body (empty unless the item is a task).
    pub subtasks: Vec<Subtask>,
    /// The item's tags (note cards surface these).
    pub tags: Vec<String>,
}

/// An item the engine filed, plus when it filed it.
#[derive(Clone, Debug)]
pub struct OrganizeFiled {
    pub item: OrganizeItem,
    /// ISO-8601 UTC of the item's `updated_at_ms`.
    pub filed_at: String,
}

/// A filed-into collection with its member count for the timeframe.
#[derive(Clone, Debug)]
pub struct OrganizeGroup {
    pub collection_id: String,
    pub label: String,
    pub count: usize,
}

/// What the engine organized in the timeframe, without a human.
#[derive(Clone, Debug, Default)]
pub struct OrganizedToday {
    pub most_recent: Option<OrganizeFiled>,
    pub groups: Vec<OrganizeGroup>,
    pub total_count: usize,
}

/// How much of the timeframe's intake is done vs. total.
#[derive(Clone, Debug)]
pub struct DailyProgress {
    /// "day" | "week" | "month".
    pub timeframe: String,
    pub done: usize,
    pub total: usize,
}

/// The full organize snapshot.
#[derive(Clone, Debug)]
pub struct OrganizeSnapshot {
    pub needs_you: Vec<OrganizeItem>,
    pub organized_today: OrganizedToday,
    pub daily_progress: DailyProgress,
    pub needs_you_ceiling: f32,
}

/// Partition the items that arrived in the timeframe into "needs you" (low
/// confidence or time-sensitive) and "organized today" (filed confidently),
/// reusing the engine's classification signal for both.
pub fn organize<S, B>(
    cp: &Commonplace<S, B>,
    pipeline: &IngestPipeline,
    config: &OrganizeConfig,
) -> GraphStoreResult<OrganizeSnapshot>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    let window_ms = config.timeframe.window_ms();
    let cutoff = config.now_ms.saturating_sub(window_ms);

    let all = cp.all_items()?;

    // Member counts over the WHOLE store: a collection is "established" once it
    // holds more than one item. Confidence is measured only against established
    // collections, so an item that merely seeded its own singleton bucket at
    // ingest (the F2 no-button path files everything) does not read as filed --
    // it surfaces in needs-you, which is the genuinely-ambiguous case.
    let mut member_counts: BTreeMap<String, usize> = BTreeMap::new();
    for item in &all {
        for collection_id in &item.collections {
            *member_counts.entry(collection_id.clone()).or_insert(0) += 1;
        }
    }

    let arrived: Vec<Item> = all
        .into_iter()
        .filter(|item| item.updated_at_ms >= cutoff)
        .collect();
    let total = arrived.len();

    // Classify every arrived item against the live collections.
    let mut prepared: Vec<OrganizeItem> = Vec::with_capacity(arrived.len());
    for item in arrived {
        let classification = pipeline.classify_item(cp, &item)?;
        prepared.push(build_item(
            item,
            &classification,
            config.needs_you_ceiling,
            &member_counts,
        ));
    }

    // Partition: needs-you is low-confidence OR time-sensitive; the rest is filed.
    let (mut needs_you, filed): (Vec<OrganizeItem>, Vec<OrganizeItem>) = prepared
        .into_iter()
        .partition(|item| item.confidence < config.needs_you_ceiling || item.time_sensitive);

    // Needs-you ordering: most-urgent first, then least confident, then recent.
    needs_you.sort_by(|a, b| {
        b.time_sensitive
            .cmp(&a.time_sensitive)
            .then(
                a.confidence
                    .partial_cmp(&b.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(b.item.updated_at_ms.cmp(&a.item.updated_at_ms))
            .then(a.item.id.cmp(&b.item.id))
    });
    needs_you.truncate(config.needs_you_cap);

    let organized_today = organized_today(filed);

    let snapshot = OrganizeSnapshot {
        needs_you,
        daily_progress: DailyProgress {
            timeframe: config.timeframe.as_str().to_string(),
            done: organized_today.total_count,
            total,
        },
        organized_today,
        needs_you_ceiling: config.needs_you_ceiling,
    };
    Ok(snapshot)
}

fn organized_today(mut filed: Vec<OrganizeItem>) -> OrganizedToday {
    let total_count = filed.len();

    // Most recent first.
    filed.sort_by(|a, b| {
        b.item
            .updated_at_ms
            .cmp(&a.item.updated_at_ms)
            .then(a.item.id.cmp(&b.item.id))
    });

    let most_recent = filed.first().map(|item| OrganizeFiled {
        filed_at: iso_from_ms(item.item.updated_at_ms),
        item: item.clone(),
    });

    // Group by target collection (skip null-target items; they still count in total).
    let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for item in &filed {
        if let (Some(id), label) = (
            item.target_collection_id.clone(),
            item.target_collection_label.clone().unwrap_or_default(),
        ) {
            let entry = counts.entry(id).or_insert((label, 0));
            entry.1 += 1;
        }
    }
    let mut groups: Vec<OrganizeGroup> = counts
        .into_iter()
        .map(|(collection_id, (label, count))| OrganizeGroup {
            collection_id,
            label,
            count,
        })
        .collect();
    groups.sort_by(|a, b| b.count.cmp(&a.count).then(a.label.cmp(&b.label)));

    OrganizedToday {
        most_recent,
        groups,
        total_count,
    }
}

fn build_item(
    item: Item,
    classification: &Classification,
    ceiling: f32,
    member_counts: &BTreeMap<String, usize>,
) -> OrganizeItem {
    // Only established collections (more than one member) count as a confident
    // home. This naturally keeps a confidently-matched item filed (its own
    // collection is established) while dropping a novel item whose only home is
    // the singleton it just seeded.
    let established: Vec<&ClassificationRank> = classification
        .ranked
        .iter()
        .filter(|rank| member_counts.get(&rank.collection_id).copied().unwrap_or(0) > 1)
        .collect();
    let confidence = established.first().map(|rank| rank.score).unwrap_or(0.0);
    let (target_collection_id, target_collection_label) = established
        .first()
        .map(|rank| {
            (
                Some(rank.collection_id.clone()),
                Some(rank.collection_name.clone()),
            )
        })
        .unwrap_or((None, None));
    let alternatives: Vec<(String, String)> = established
        .iter()
        .skip(1)
        .take(3)
        .map(|rank| (rank.collection_id.clone(), rank.collection_name.clone()))
        .collect();

    let source = item.source.clone().unwrap_or_default();
    let tags = item.tags.clone();
    let body = match &item.body {
        ItemBody::Inline { text } => text.as_str(),
        _ => "",
    };
    let subtasks = parse_subtasks(body);
    let kind = derive_kind(&item, &source, &subtasks);
    let time_sensitive = kind == "email";
    let expected_action = if time_sensitive {
        Some("reply".to_string())
    } else if confidence >= ceiling {
        Some("open".to_string())
    } else {
        None
    };
    let preview = preview_text(&item);

    OrganizeItem {
        item,
        kind,
        preview,
        source,
        target_collection_id,
        target_collection_label,
        confidence,
        alternatives,
        time_sensitive,
        expected_action,
        subtasks,
        tags,
    }
}

/// Derive the presentation kind. Precedence (first match wins):
/// 1. email: `source` mentions email/gmail/mail.
/// 2. event: a tag in {event, meeting, calendar}.
/// 3. task: the body has subtasks, OR the source is an `mcp:` task connector
///    (linear/asana/jira/todo/task), OR a tag in {task, todo}.
/// 4. file: the item kind is File or Image.
/// 5. note: everything else.
fn derive_kind(item: &Item, source: &str, subtasks: &[Subtask]) -> String {
    let source_lower = source.to_ascii_lowercase();
    if source_lower.contains("email")
        || source_lower.contains("gmail")
        || source_lower.contains("mail")
    {
        return "email".to_string();
    }

    let has_tag = |candidates: &[&str]| {
        item.tags.iter().any(|tag| {
            let tag = tag.trim().to_ascii_lowercase();
            candidates.contains(&tag.as_str())
        })
    };

    if has_tag(&["event", "meeting", "calendar"]) {
        return "event".to_string();
    }

    let mcp_task_source = source_lower.starts_with("mcp:")
        && ["linear", "asana", "jira", "todo", "task"]
            .iter()
            .any(|needle| source_lower.contains(needle));
    if !subtasks.is_empty() || mcp_task_source || has_tag(&["task", "todo"]) {
        return "task".to_string();
    }

    match item.kind {
        ItemKind::File | ItemKind::Image => "file".to_string(),
        _ => "note".to_string(),
    }
}

/// Parse markdown checkbox lines out of a body. Accepts a bullet prefix
/// (`- `, `* `, `+ `) followed by `[ ]` (open) or `[x]`/`[X]` (done); the trimmed
/// remainder is the subtask text. Lines with empty text or no checkbox are
/// skipped. Returns the subtasks in document order.
fn parse_subtasks(body: &str) -> Vec<Subtask> {
    let mut subtasks = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        let after_bullet = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "));
        let Some(after_bullet) = after_bullet else {
            continue;
        };
        let after_bullet = after_bullet.trim_start();
        let (done, rest) = if let Some(rest) = after_bullet.strip_prefix("[ ]") {
            (false, rest)
        } else if let Some(rest) = after_bullet
            .strip_prefix("[x]")
            .or_else(|| after_bullet.strip_prefix("[X]"))
        {
            (true, rest)
        } else {
            continue;
        };
        let text = rest.trim();
        if text.is_empty() {
            continue;
        }
        subtasks.push(Subtask {
            text: text.to_string(),
            done,
        });
    }
    subtasks
}

fn preview_text(item: &Item) -> String {
    let text = match &item.body {
        ItemBody::Inline { text } => text.trim(),
        _ => item.title.trim(),
    };
    text.chars().take(200).collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// Render epoch milliseconds to an ISO-8601 UTC timestamp ("YYYY-MM-DDTHH:MM:SSZ")
/// with no date dependency (proleptic Gregorian, days since the Unix epoch).
pub fn iso_from_ms(ms: i64) -> String {
    let ms = ms.max(0);
    let total_secs = ms / 1000;
    let secs_of_day = total_secs.rem_euclid(DAY_MS / 1000);
    let days = total_secs.div_euclid(DAY_MS / 1000);

    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since 1970-01-01 -> (year, month, day), proleptic Gregorian.
/// Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_renders_known_epochs() {
        assert_eq!(iso_from_ms(0), "1970-01-01T00:00:00Z");
        // 2021-01-01T00:00:00Z = 1609459200 seconds.
        assert_eq!(iso_from_ms(1_609_459_200_000), "2021-01-01T00:00:00Z");
        // 2024-02-29 (leap day) 12:34:56 UTC = 1709210096 seconds.
        assert_eq!(iso_from_ms(1_709_210_096_000), "2024-02-29T12:34:56Z");
    }

    #[test]
    fn timeframe_parses_from_str() {
        assert_eq!(Timeframe::from("week"), Timeframe::Week);
        assert_eq!(Timeframe::from("MONTH"), Timeframe::Month);
        assert_eq!(Timeframe::from("anything-else"), Timeframe::Day);
    }

    #[test]
    fn parse_subtasks_reads_checkboxes() {
        let body = "- [x] book the room\n\
                    - [ ] send the agenda\n\
                    * [ ] invite finance\n\
                    a plain line that is not a checkbox";
        let subtasks = parse_subtasks(body);
        assert_eq!(
            subtasks.len(),
            3,
            "three checkbox lines parse, the plain line is ignored"
        );
        assert_eq!(
            subtasks.iter().filter(|s| s.done).count(),
            1,
            "exactly one is done"
        );
        // Order preserved, first done.
        assert_eq!(
            subtasks[0],
            Subtask {
                text: "book the room".to_string(),
                done: true,
            }
        );
        assert_eq!(
            subtasks[1],
            Subtask {
                text: "send the agenda".to_string(),
                done: false,
            }
        );
        assert_eq!(
            subtasks[2],
            Subtask {
                text: "invite finance".to_string(),
                done: false,
            }
        );
    }
}
