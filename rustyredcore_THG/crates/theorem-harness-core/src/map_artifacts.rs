use crate::types::now_string;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub type MapEntry = Map<String, Value>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapArtifactState {
    pub map_id: String,
    pub map_kind: String,
    pub scope_kind: String,
    pub scope_ref: String,
    #[serde(default)]
    pub workstream_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub descriptor: Map<String, Value>,
    #[serde(default)]
    pub entries: Vec<MapEntry>,
    #[serde(default)]
    pub source_delta_ids: Vec<String>,
    #[serde(default)]
    pub pending_delta_count: i64,
    #[serde(default)]
    pub applied_delta_count: i64,
    #[serde(default)]
    pub state_hash: String,
    #[serde(default)]
    pub token_estimate: i64,
    #[serde(default)]
    pub markdown_body: String,
    #[serde(default)]
    pub json_export: Map<String, Value>,
    #[serde(default = "now_string")]
    pub created_at: String,
    #[serde(default = "now_string")]
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapDeltaState {
    pub delta_id: String,
    pub map_kind: String,
    pub scope_kind: String,
    pub scope_ref: String,
    #[serde(default)]
    pub workstream_id: String,
    #[serde(default)]
    pub target_map_id: String,
    #[serde(default = "proposed_status")]
    pub status: String,
    #[serde(default = "upsert_action")]
    pub action: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub entry_id: String,
    #[serde(default)]
    pub entry: Map<String, Value>,
    #[serde(default)]
    pub proposed_by: String,
    #[serde(default)]
    pub source_run_id: String,
    #[serde(default)]
    pub source_event_ids: Vec<String>,
    #[serde(default = "now_string")]
    pub created_at: String,
    #[serde(default)]
    pub applied_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct MapArtifactCompileInput {
    pub map_kind: String,
    pub scope_kind: String,
    pub scope_ref: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub domain: Map<String, Value>,
    #[serde(default)]
    pub selected_tools: Vec<Map<String, Value>>,
    #[serde(default)]
    pub validators: Vec<String>,
    #[serde(default)]
    pub recall_policy: Map<String, Value>,
    #[serde(default)]
    pub memory_recall_preview: Map<String, Value>,
    #[serde(default)]
    pub risk_mode: String,
    #[serde(default)]
    pub applied_deltas: Vec<MapDeltaState>,
    #[serde(default)]
    pub pending_delta_count: i64,
    #[serde(default)]
    pub current: Option<MapArtifactState>,
    #[serde(default)]
    pub workstream_id: String,
    /// KG-projected baseline entries (module/entry_point/dependency/key_symbol)
    /// injected alongside the task-context entries for a CodebaseMap. Empty for
    /// every existing task-context caller, so those callers keep the same entry
    /// baseline; grouped CodebaseMap markdown is the intentional render change.
    /// The code-graph projection (uniform-seed PageRank over the code KG) is the
    /// only producer.
    #[serde(default)]
    pub precomputed_entries: Vec<MapEntry>,
}

pub fn stable_map_id(map_kind: &str, scope_kind: &str, scope_ref: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{map_kind}\0{scope_kind}\0{scope_ref}").as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("map:{}", &digest[..32])
}

pub fn scope_for_map_kind(
    map_kind: &str,
    repo: Option<&str>,
    domain: Option<&Map<String, Value>>,
) -> (String, String) {
    let normalized_repo = repo.unwrap_or("").trim();
    let normalized_domain = domain
        .and_then(|value| value.get("domain"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if map_kind == "DomainMap" && !normalized_domain.is_empty() {
        return ("domain".to_string(), normalized_domain.to_string());
    }
    if !normalized_repo.is_empty() {
        return ("repo".to_string(), normalized_repo.to_string());
    }
    if !normalized_domain.is_empty() {
        return ("domain".to_string(), normalized_domain.to_string());
    }
    ("global".to_string(), "default".to_string())
}

pub fn compile_map_artifact(input: MapArtifactCompileInput) -> MapArtifactState {
    let now = now_string();
    let map_kind = non_empty(&input.map_kind, "CodebaseMap");
    let scope_kind = non_empty(&input.scope_kind, "repo");
    let scope_ref = input.scope_ref.clone();
    let baseline = baseline_entries(&input, &map_kind);
    let (resolved_entries, applied_ids) = apply_deltas(baseline, &input.applied_deltas);
    let title = format!("{map_kind} for {scope_ref}");
    let summary = format!(
        "{map_kind} exposes {} compact orientation entries for {scope_ref}.",
        resolved_entries.len()
    );
    let map_id = input
        .current
        .as_ref()
        .map(|current| current.map_id.clone())
        .unwrap_or_else(|| stable_map_id(&map_kind, &scope_kind, &scope_ref));
    let descriptor = map_descriptor(
        &map_id,
        resolved_entries.len(),
        &scope_kind,
        &scope_ref,
        input.pending_delta_count,
        applied_ids.len() as i64,
    );
    let markdown_body = if map_kind == "CodebaseMap" {
        render_codebase_markdown(&title, &summary, &resolved_entries)
    } else {
        render_markdown(&map_kind, &title, &summary, &resolved_entries)
    };
    let mut json_export = Map::new();
    json_export.insert("map_id".to_string(), json!(map_id));
    json_export.insert("map_kind".to_string(), json!(map_kind));
    json_export.insert("scope_kind".to_string(), json!(scope_kind));
    json_export.insert("scope_ref".to_string(), json!(scope_ref));
    json_export.insert("title".to_string(), json!(title));
    json_export.insert("summary".to_string(), json!(summary));
    json_export.insert("descriptor".to_string(), Value::Object(descriptor.clone()));
    json_export.insert("entries".to_string(), json!(resolved_entries));
    json_export.insert("source_delta_ids".to_string(), json!(applied_ids));
    json_export.insert(
        "pending_delta_count".to_string(),
        json!(input.pending_delta_count),
    );
    json_export.insert(
        "applied_delta_count".to_string(),
        json!(applied_ids.len() as i64),
    );
    let state_hash = stable_map_hash(&Value::Object(json_export.clone()));
    json_export.insert("state_hash".to_string(), json!(state_hash));
    let workstream_id = non_empty(
        &input.workstream_id,
        input
            .current
            .as_ref()
            .map(|current| current.workstream_id.as_str())
            .unwrap_or(""),
    );
    MapArtifactState {
        map_id,
        map_kind,
        scope_kind,
        scope_ref,
        workstream_id,
        title,
        summary,
        descriptor,
        entries: resolved_entries,
        source_delta_ids: applied_ids.clone(),
        pending_delta_count: input.pending_delta_count,
        applied_delta_count: applied_ids.len() as i64,
        state_hash,
        token_estimate: approx_tokens(&markdown_body),
        markdown_body,
        json_export,
        created_at: input
            .current
            .as_ref()
            .map(|current| current.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
    }
}

pub fn describe_map_artifact(artifact: &MapArtifactState) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("map_id".to_string(), json!(artifact.map_id));
    out.insert("map_kind".to_string(), json!(artifact.map_kind));
    out.insert("scope_kind".to_string(), json!(artifact.scope_kind));
    out.insert("scope_ref".to_string(), json!(artifact.scope_ref));
    out.insert("title".to_string(), json!(artifact.title));
    out.insert("summary".to_string(), json!(artifact.summary));
    out.insert("entry_count".to_string(), json!(artifact.entries.len()));
    out.insert(
        "hydration_handle".to_string(),
        json!(artifact
            .descriptor
            .get("hydration_handle")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("map_artifact:{}", artifact.map_id))),
    );
    out.insert("state_hash".to_string(), json!(artifact.state_hash));
    out.insert(
        "pending_delta_count".to_string(),
        json!(artifact.pending_delta_count),
    );
    out.insert(
        "applied_delta_count".to_string(),
        json!(artifact.applied_delta_count),
    );
    out.insert(
        "export_formats".to_string(),
        artifact
            .descriptor
            .get("export_formats")
            .cloned()
            .unwrap_or_else(|| json!([])),
    );
    out
}

fn baseline_entries(input: &MapArtifactCompileInput, map_kind: &str) -> Vec<MapEntry> {
    match map_kind {
        "ToolMap" => tool_entries(&input.selected_tools),
        "CodebaseMap" | "SpecMap" => {
            // Task-context orientation first, then any KG-projected entries. With
            // no projection (the default) this keeps the prior entry baseline.
            let mut entries = codebase_entries(input);
            entries.extend(input.precomputed_entries.iter().cloned());
            entries
        }
        "RuleMap" => rule_entries(input),
        "UserMemoryMap" => user_memory_entries(&input.memory_recall_preview),
        "PostmortemMap" => postmortem_entries(&input.memory_recall_preview),
        "DomainMap" => domain_entries(&input.domain, &input.validators),
        _ => generic_entries(input, map_kind),
    }
}

fn tool_entries(selected_tools: &[Map<String, Value>]) -> Vec<MapEntry> {
    selected_tools
        .iter()
        .filter_map(|tool| {
            let tool_id = string_field(tool, "tool_id", "")
                .or_else(|| string_field(tool, "id", ""))
                .unwrap_or_default();
            if tool_id.trim().is_empty() {
                return None;
            }
            let mut metadata = Map::new();
            metadata.insert("cost".to_string(), json!(string_or(tool, "cost", "low")));
            metadata.insert(
                "permissions".to_string(),
                array_or_empty(tool, "permissions"),
            );
            metadata.insert("inputs".to_string(), array_or_empty(tool, "inputs"));
            metadata.insert("outputs".to_string(), array_or_empty(tool, "outputs"));
            Some(normalize_entry(
                &tool_id,
                "tool",
                &string_or(tool, "name", &tool_id),
                &string_or(tool, "reason", "Recommended for this task."),
                metadata,
            ))
        })
        .collect()
}

fn codebase_entries(input: &MapArtifactCompileInput) -> Vec<MapEntry> {
    let mut entries = Vec::new();
    if !input.repo.trim().is_empty() {
        entries.push(normalize_entry(
            "repo-boundary",
            "repo",
            "Repo boundary",
            &format!("Primary repository scope for this task is {}.", input.repo),
            Map::from_iter([("repo".to_string(), json!(input.repo))]),
        ));
    }
    if !input.target.trim().is_empty() {
        entries.push(normalize_entry(
            "task-target",
            "target",
            "Task target",
            &input.target,
            Map::from_iter([("target".to_string(), json!(input.target))]),
        ));
    }
    for (index, text) in preview_strings(&input.memory_recall_preview, "read_first")
        .into_iter()
        .enumerate()
    {
        entries.push(normalize_entry(
            &format!("read-first:{}", index + 1),
            "read_first",
            &format!("Read first {}", index + 1),
            &text,
            Map::new(),
        ));
    }
    if !input.validators.is_empty() {
        entries.push(normalize_entry(
            "focused-validators",
            "validators",
            "Focused validators",
            "Validation should stay narrow and scoped to this work.",
            Map::from_iter([("validators".to_string(), json!(input.validators))]),
        ));
    }
    entries
}

fn rule_entries(input: &MapArtifactCompileInput) -> Vec<MapEntry> {
    let mut entries = Vec::new();
    if !input.risk_mode.trim().is_empty() {
        entries.push(normalize_entry(
            "risk-mode",
            "risk_mode",
            "Risk mode",
            &input.risk_mode,
            Map::new(),
        ));
    }
    if !input.recall_policy.is_empty() {
        entries.push(normalize_entry(
            "recall-policy",
            "policy",
            "Recall policy",
            "Memory recall policy for this scope.",
            input.recall_policy.clone(),
        ));
    }
    for (index, text) in preview_strings(&input.memory_recall_preview, "do_not")
        .into_iter()
        .enumerate()
    {
        entries.push(normalize_entry(
            &format!("do-not:{}", index + 1),
            "do_not",
            &format!("Do not {}", index + 1),
            &text,
            Map::new(),
        ));
    }
    for (index, text) in preview_strings(&input.memory_recall_preview, "active_policy")
        .into_iter()
        .enumerate()
    {
        entries.push(normalize_entry(
            &format!("active-policy:{}", index + 1),
            "active_policy",
            &format!("Active policy {}", index + 1),
            &text,
            Map::new(),
        ));
    }
    if !input.validators.is_empty() {
        entries.push(normalize_entry(
            "validator-policy",
            "validators",
            "Validator policy",
            "These validators are expected before writeback or completion.",
            Map::from_iter([("validators".to_string(), json!(input.validators))]),
        ));
    }
    entries
}

fn user_memory_entries(preview: &Map<String, Value>) -> Vec<MapEntry> {
    let mut entries = Vec::new();
    for (index, text) in preview_strings(preview, "next_actions")
        .into_iter()
        .enumerate()
    {
        entries.push(normalize_entry(
            &format!("next-action:{}", index + 1),
            "next_action",
            &format!("Next action {}", index + 1),
            &text,
            Map::new(),
        ));
    }
    for (index, text) in preview_strings(preview, "proposed_policy")
        .into_iter()
        .enumerate()
    {
        entries.push(normalize_entry(
            &format!("proposed-policy:{}", index + 1),
            "proposed_policy",
            &format!("Proposed policy {}", index + 1),
            &text,
            Map::new(),
        ));
    }
    for (index, text) in preview_strings(preview, "recalled_evidence")
        .into_iter()
        .enumerate()
    {
        entries.push(normalize_entry(
            &format!("recalled-evidence:{}", index + 1),
            "evidence",
            &format!("Recalled evidence {}", index + 1),
            &text,
            Map::new(),
        ));
    }
    entries
}

fn postmortem_entries(preview: &Map<String, Value>) -> Vec<MapEntry> {
    let mut entries = Vec::new();
    for (index, text) in preview_strings(preview, "risks").into_iter().enumerate() {
        entries.push(normalize_entry(
            &format!("risk:{}", index + 1),
            "risk",
            &format!("Risk {}", index + 1),
            &text,
            Map::new(),
        ));
    }
    for (index, text) in preview_strings(preview, "do_not").into_iter().enumerate() {
        entries.push(normalize_entry(
            &format!("avoid-repeat:{}", index + 1),
            "avoid_repeat",
            &format!("Avoid repeat {}", index + 1),
            &text,
            Map::new(),
        ));
    }
    entries
}

fn domain_entries(domain: &Map<String, Value>, validators: &[String]) -> Vec<MapEntry> {
    if domain.is_empty() {
        return Vec::new();
    }
    let mut metadata = Map::new();
    metadata.insert(
        "domain_pack_id".to_string(),
        json!(string_or(domain, "domain_pack_id", "")),
    );
    metadata.insert(
        "domain_version".to_string(),
        json!(string_or(domain, "domain_version", "")),
    );
    metadata.insert(
        "profile_id".to_string(),
        json!(string_or(domain, "profile_id", "")),
    );
    metadata.insert(
        "memory_banks".to_string(),
        array_or_empty(domain, "memory_banks"),
    );
    metadata.insert("validators".to_string(), json!(validators));
    vec![normalize_entry(
        "domain-pack",
        "domain",
        &string_or(domain, "domain", "Domain"),
        "Primary domain pack selected for this task.",
        metadata,
    )]
}

fn generic_entries(input: &MapArtifactCompileInput, map_kind: &str) -> Vec<MapEntry> {
    let mut metadata = Map::new();
    metadata.insert("repo".to_string(), json!(input.repo));
    metadata.insert(
        "domain".to_string(),
        json!(string_or(&input.domain, "domain", "")),
    );
    metadata.insert(
        "selected_banks".to_string(),
        array_or_empty(&input.memory_recall_preview, "selected_banks"),
    );
    vec![normalize_entry(
        &entry_id("summary", map_kind),
        "summary",
        map_kind,
        &non_empty(&input.task, &format!("Orientation surface for {map_kind}.")),
        metadata,
    )]
}

fn apply_deltas(entries: Vec<MapEntry>, deltas: &[MapDeltaState]) -> (Vec<MapEntry>, Vec<String>) {
    let mut by_id = entries
        .into_iter()
        .map(|entry| {
            let id = string_or(
                &entry,
                "entry_id",
                &entry_id("entry", &string_or(&entry, "title", "item")),
            );
            (id, entry)
        })
        .collect::<BTreeMap<_, _>>();
    let mut applied_ids = Vec::new();
    for delta in deltas.iter().filter(|delta| delta.status == "applied") {
        applied_ids.push(delta.delta_id.clone());
        let entry_id = non_empty(
            &delta.entry_id,
            &string_or(
                &delta.entry,
                "entry_id",
                &entry_id("delta", &non_empty(&delta.summary, &delta.map_kind)),
            ),
        );
        if delta.action == "remove" {
            by_id.remove(&entry_id);
            continue;
        }
        let mut next_entry = by_id.get(&entry_id).cloned().unwrap_or_default();
        let normalized = normalize_entry(
            &entry_id,
            &string_or(&delta.entry, "kind", "delta"),
            &string_or(&delta.entry, "title", &non_empty(&delta.summary, &entry_id)),
            &string_or(
                &delta.entry,
                "summary",
                &non_empty(
                    &non_empty(&delta.summary, &delta.rationale),
                    "Delta-applied map entry.",
                ),
            ),
            object_or_empty(&delta.entry, "metadata"),
        );
        for (key, value) in normalized {
            next_entry.insert(key, value);
        }
        let mut metadata = object_or_empty(&next_entry, "metadata");
        metadata.insert("delta_id".to_string(), json!(delta.delta_id));
        metadata.insert("delta_action".to_string(), json!(delta.action));
        next_entry.insert("metadata".to_string(), Value::Object(metadata));
        by_id.insert(entry_id, next_entry);
    }
    (by_id.into_values().collect(), applied_ids)
}

fn normalize_entry(
    entry_id: &str,
    kind: &str,
    title: &str,
    summary: &str,
    metadata: Map<String, Value>,
) -> MapEntry {
    Map::from_iter([
        ("entry_id".to_string(), json!(entry_id)),
        ("kind".to_string(), json!(non_empty(kind, "note"))),
        ("title".to_string(), json!(title.trim())),
        ("summary".to_string(), json!(truncate(summary, 180))),
        ("metadata".to_string(), Value::Object(metadata)),
    ])
}

fn map_descriptor(
    map_id: &str,
    entry_count: usize,
    scope_kind: &str,
    scope_ref: &str,
    pending_delta_count: i64,
    applied_delta_count: i64,
) -> Map<String, Value> {
    Map::from_iter([
        (
            "hydration_handle".to_string(),
            json!(format!("map_artifact:{map_id}")),
        ),
        ("entry_count".to_string(), json!(entry_count)),
        ("scope_kind".to_string(), json!(scope_kind)),
        ("scope_ref".to_string(), json!(scope_ref)),
        (
            "pending_delta_count".to_string(),
            json!(pending_delta_count),
        ),
        (
            "applied_delta_count".to_string(),
            json!(applied_delta_count),
        ),
        ("export_formats".to_string(), json!(["json", "markdown"])),
    ])
}

fn render_markdown(map_kind: &str, title: &str, summary: &str, entries: &[MapEntry]) -> String {
    let mut lines = vec![
        format!("# {title}"),
        String::new(),
        non_empty(summary.trim(), &format!("{map_kind} artifact.")),
        String::new(),
    ];
    for entry in entries {
        lines.push(format!(
            "- {}: {}",
            string_or(entry, "title", ""),
            string_or(entry, "summary", "")
        ));
    }
    lines.join("\n").trim().to_string()
}

/// Render a CodebaseMap as sectioned markdown: entries are grouped by `kind`
/// (KG-projected structure first, task-context orientation after) so `codebase.md`
/// reads as labelled sections instead of one flat list. A kind with no preset
/// label renders under a title-cased version of its own name, so a future
/// projection kind needs no change here.
fn render_codebase_markdown(title: &str, summary: &str, entries: &[MapEntry]) -> String {
    let mut lines = vec![
        format!("# {title}"),
        String::new(),
        non_empty(summary.trim(), "CodebaseMap artifact."),
    ];
    let mut groups: BTreeMap<String, Vec<&MapEntry>> = BTreeMap::new();
    let mut first_seen: Vec<String> = Vec::new();
    for entry in entries {
        let kind = string_or(entry, "kind", "note");
        if !groups.contains_key(&kind) {
            first_seen.push(kind.clone());
        }
        groups.entry(kind).or_default().push(entry);
    }
    // Preferred sections first (when present), then any remaining kinds in the
    // order they first appeared.
    let mut ordered: Vec<String> = CODEBASE_SECTION_ORDER
        .iter()
        .map(|kind| kind.to_string())
        .filter(|kind| groups.contains_key(kind))
        .collect();
    for kind in &first_seen {
        if !ordered.contains(kind) {
            ordered.push(kind.clone());
        }
    }
    for kind in ordered {
        let Some(group) = groups.get(&kind) else {
            continue;
        };
        lines.push(String::new());
        lines.push(format!("## {}", codebase_section_label(&kind)));
        for entry in group {
            lines.push(format!(
                "- {}: {}",
                string_or(entry, "title", ""),
                string_or(entry, "summary", "")
            ));
        }
    }
    lines.join("\n").trim().to_string()
}

/// Section order for grouped CodebaseMap rendering: KG-projected structure first,
/// then task-context orientation.
const CODEBASE_SECTION_ORDER: [&str; 8] = [
    "module",
    "entry_point",
    "dependency",
    "key_symbol",
    "repo",
    "target",
    "read_first",
    "validators",
];

fn codebase_section_label(kind: &str) -> String {
    match kind {
        "module" => "Modules".to_string(),
        "entry_point" => "Entry points".to_string(),
        "dependency" => "Dependencies".to_string(),
        "key_symbol" => "Key symbols".to_string(),
        "repo" => "Repo boundary".to_string(),
        "target" => "Task target".to_string(),
        "read_first" => "Read first".to_string(),
        "validators" => "Validators".to_string(),
        other => {
            let spaced = other.replace('_', " ");
            let mut chars = spaced.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Other".to_string(),
            }
        }
    }
}

fn stable_map_hash(value: &Value) -> String {
    let encoded = canonical_json_python_style(value);
    let mut hasher = Sha256::new();
    hasher.update(encoded.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn canonical_json_python_style(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => {
            serde_json::to_string(value).expect("string serialization should be infallible")
        }
        Value::Array(items) => items
            .iter()
            .map(canonical_json_python_style)
            .collect::<Vec<_>>()
            .join(", ")
            .pipe(|inner| format!("[{inner}]")),
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            keys.into_iter()
                .map(|key| {
                    format!(
                        "{}: {}",
                        canonical_json_python_style(&Value::String(key.clone())),
                        canonical_json_python_style(map.get(key).unwrap_or(&Value::Null))
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
                .pipe(|inner| format!("{{{inner}}}"))
        }
    }
}

fn approx_tokens(text: &str) -> i64 {
    if text.is_empty() {
        0
    } else {
        ((text.len() as i64 + 3) / 4).max(1)
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= max_chars {
        compact
    } else {
        format!("{}...", compact[..max_chars.saturating_sub(1)].trim_end())
    }
}

fn entry_id(prefix: &str, value: &str) -> String {
    let normalized = value
        .trim()
        .to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | ':'))
        .collect::<String>();
    let normalized = non_empty(&normalized, "item");
    format!("{prefix}:{}", &normalized[..normalized.len().min(64)])
}

fn preview_strings(data: &Map<String, Value>, key: &str) -> Vec<String> {
    data.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn string_field(data: &Map<String, Value>, key: &str, fallback: &str) -> Option<String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| Some(fallback.to_string()).filter(|value| !value.is_empty()))
}

fn string_or(data: &Map<String, Value>, key: &str, fallback: &str) -> String {
    string_field(data, key, fallback).unwrap_or_default()
}

fn object_or_empty(data: &Map<String, Value>, key: &str) -> Map<String, Value> {
    data.get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn array_or_empty(data: &Map<String, Value>, key: &str) -> Value {
    data.get(key)
        .and_then(Value::as_array)
        .cloned()
        .map(Value::Array)
        .unwrap_or_else(|| json!([]))
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn proposed_status() -> String {
    "proposed".to_string()
}

fn upsert_action() -> String {
    "upsert".to_string()
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}
