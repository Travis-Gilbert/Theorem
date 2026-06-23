use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::OnceLock;

pub const PACK_ID: &str = "skill-pack:writing-engineering-prose-v0.1";
pub const TOKENIZER_NAME: &str = "cl100k_base_estimate";

pub const CORE_DIRECTIVE: &str = include_str!("rules/directive.txt");
pub const REGISTER_TABLE: &str = include_str!("rules/registers.txt");

const CLUTTER: &str = include_str!("rules/clutter.tsv");
const REDUNDANT_PAIRS: &str = include_str!("rules/redundant-pairs.tsv");
const LATINATE_SWAPS: &str = include_str!("rules/latinate-swaps.tsv");
const ADVERB_WHITELIST: &str = include_str!("rules/adverb-whitelist.txt");
const HEDGES: &str = include_str!("rules/hedges.txt");
const WIRE_ABBREV: &str = include_str!("rules/wire-abbrev.tsv");

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Register {
    Plain,
    Spare,
    Wire,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Fidelity {
    pub preserved: bool,
    pub missing: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClutterHit {
    pub phrase: String,
    pub span: Span,
    pub suggestion: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StyleReceipt {
    pub register: Register,
    pub tokens: u32,
    pub baseline_tokens: Option<u32>,
    pub reduction: Option<f32>,
    pub fidelity: Fidelity,
    pub passive_rate: f32,
    pub adverb_rate: f32,
    pub clutter_hits: Vec<ClutterHit>,
    pub nominalization_rate: f32,
    pub sentence_mean: f32,
    pub sentence_stdev: f32,
    pub flesch_kincaid: f32,
    pub em_dash_count: u32,
    pub clarity_breaks: Vec<Span>,
    pub code_spans: Vec<Span>,
    pub pack_hash: String,
}

pub fn check(text: &str, register: Register, source_identifiers: &[String]) -> StyleReceipt {
    check_inner(text, None, register, source_identifiers)
}

pub fn check_with_baseline(
    text: &str,
    baseline: &str,
    register: Register,
    source_identifiers: &[String],
) -> StyleReceipt {
    check_inner(text, Some(baseline), register, source_identifiers)
}

pub fn pack_hash() -> String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let mut hasher = Sha256::new();
        for (name, body) in [
            ("directive", CORE_DIRECTIVE),
            ("registers", REGISTER_TABLE),
            ("clutter.tsv", CLUTTER),
            ("redundant-pairs.tsv", REDUNDANT_PAIRS),
            ("latinate-swaps.tsv", LATINATE_SWAPS),
            ("adverb-whitelist.txt", ADVERB_WHITELIST),
            ("hedges.txt", HEDGES),
            ("wire-abbrev.tsv", WIRE_ABBREV),
        ] {
            hasher.update(name.as_bytes());
            hasher.update([0]);
            hasher.update(body.as_bytes());
            hasher.update([0xff]);
        }
        format!("sha256:{:x}", hasher.finalize())
    })
    .clone()
}

pub fn writing_engineering_pack_payload(parent_hash: Option<&str>) -> Value {
    let mut metadata = json!({
        "status": "shadow",
        "promotion_state": "shadow",
        "pack_content_hash": pack_hash(),
        "source_content_hash": source_hash(),
        "tokenizer": TOKENIZER_NAME,
        "artifacts": {
            "rules": rule_artifacts()
        },
        "fitness": {
            "style_receipt_count": 0,
            "fidelity_failures": 0,
            "hard_axis_failures": 0,
            "promotion_gate": "benchmark_pending"
        }
    });
    if let Some(parent_hash) = parent_hash.filter(|value| !value.trim().is_empty()) {
        metadata["parent_pack_content_hash"] = Value::String(parent_hash.to_string());
    }
    json!({
        "id": PACK_ID,
        "name": "writing-engineering",
        "kind": "skill_pack",
        "title": "Writing Engineering",
        "description": "Encoded prose pack for plain, spare, and wire-register synthesis receipts.",
        "directive": CORE_DIRECTIVE,
        "registers": {
            "plain": {
                "audience": "user-facing reports and chat",
                "clutter_hits": 0,
                "passive_rate_max": 0.10,
                "adverb_rate_max": 1.5,
                "sentence_mean_min": 12.0,
                "sentence_mean_max": 18.0,
                "sentence_stdev_min": 4.0
            },
            "spare": {
                "audience": "briefs and postmortems",
                "clutter_hits": 0,
                "passive_rate_max": 0.05,
                "adverb_rate_max": 0.8,
                "sentence_mean_min": 7.0,
                "sentence_mean_max": 12.0,
                "sentence_stdev_min": 3.0
            },
            "wire": {
                "audience": "agent-to-agent packets, intents, and coordination",
                "clutter_hits": 0,
                "passive_rate_max": 0.05,
                "sentence_mean_min": 5.0,
                "sentence_mean_max": 9.0,
                "article_drop": "allowed only when unambiguous"
            }
        },
        "capabilities": [
            "style_receipt",
            "synthesis_boundary_check",
            "report_boundary_check",
            "coordination_wire_register",
            "fidelity_gate",
            "fitness_fold"
        ],
        "validators": [
            {"id": "prose-check-api", "kind": "required_field", "field": "directive"},
            {"id": "register-table", "kind": "required_field", "field": "registers"},
            {"id": "lexicon-artifacts", "kind": "artifact_hash_present"}
        ],
        "metadata": metadata
    })
}

fn check_inner(
    text: &str,
    baseline: Option<&str>,
    register: Register,
    source_identifiers: &[String],
) -> StyleReceipt {
    let code_spans = code_spans(text);
    let clarity_breaks = clarity_breaks(text, &code_spans);
    let excluded = merge_spans(code_spans.iter().chain(clarity_breaks.iter()).cloned());
    let score_text = text_outside_spans(text, &excluded);
    let sentences = split_sentences(&score_text);
    let words = word_tokens(&score_text);
    let word_count = words.len().max(1) as f32;
    let passive_sentences = sentences
        .iter()
        .filter(|sentence| sentence_has_passive(sentence))
        .count();
    let adverbs = words.iter().filter(|word| is_scored_adverb(word)).count() as f32;
    let nominalizations = words.iter().filter(|word| is_nominalization(word)).count() as f32;
    let sentence_lengths = sentences
        .iter()
        .map(|sentence| word_tokens(sentence).len() as f32)
        .filter(|count| *count > 0.0)
        .collect::<Vec<_>>();
    let sentence_mean = mean(&sentence_lengths);
    let sentence_stdev = stdev(&sentence_lengths, sentence_mean);
    let tokens = estimate_tokens(text);
    let baseline_tokens = baseline.map(estimate_tokens);
    let reduction = baseline_tokens.and_then(|baseline_tokens| {
        (baseline_tokens > 0).then(|| 1.0 - (tokens as f32 / baseline_tokens as f32))
    });

    StyleReceipt {
        register,
        tokens,
        baseline_tokens,
        reduction,
        fidelity: fidelity(text, source_identifiers),
        passive_rate: ratio(passive_sentences as f32, sentences.len().max(1) as f32),
        adverb_rate: ratio(adverbs * 100.0, word_count),
        clutter_hits: clutter_hits(text, &excluded),
        nominalization_rate: ratio(nominalizations * 100.0, word_count),
        sentence_mean,
        sentence_stdev,
        flesch_kincaid: flesch_kincaid(&sentences, &words),
        em_dash_count: em_dash_count(&score_text),
        clarity_breaks,
        code_spans,
        pack_hash: pack_hash(),
    }
}

fn source_hash() -> String {
    let mut hasher = Sha256::new();
    hasher.update(CORE_DIRECTIVE.as_bytes());
    hasher.update(REGISTER_TABLE.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn rule_artifacts() -> Vec<Value> {
    [
        ("clutter.tsv", CLUTTER),
        ("redundant-pairs.tsv", REDUNDANT_PAIRS),
        ("latinate-swaps.tsv", LATINATE_SWAPS),
        ("adverb-whitelist.txt", ADVERB_WHITELIST),
        ("hedges.txt", HEDGES),
        ("wire-abbrev.tsv", WIRE_ABBREV),
    ]
    .into_iter()
    .map(|(name, body)| {
        json!({
            "name": name,
            "content_hash": hash_text(body),
            "line_count": body.lines().count()
        })
    })
    .collect()
}

fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn code_spans(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut search = 0;
    while let Some(start_rel) = text[search..].find("```") {
        let start = search + start_rel;
        let after_start = start + 3;
        let Some(end_rel) = text[after_start..].find("```") else {
            break;
        };
        let end = after_start + end_rel + 3;
        spans.push(span(start, end));
        search = end;
    }
    spans.extend(labeled_passthrough_spans(text, "Commit message:"));
    spans.extend(labeled_passthrough_spans(text, "PR body:"));
    merge_spans(spans)
}

fn labeled_passthrough_spans(text: &str, label: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut search = 0;
    while let Some(label_rel) = text[search..].find(label) {
        let label_start = search + label_rel;
        let start = label_start + label.len();
        let start = text[start..]
            .find('\n')
            .map(|newline| start + newline + 1)
            .unwrap_or(start);
        let end = text[start..]
            .find("\n\n")
            .map(|block_end| start + block_end)
            .unwrap_or(text.len());
        if start < end {
            spans.push(span(start, end));
        }
        search = end;
    }
    spans
}

fn clarity_breaks(text: &str, code_spans: &[Span]) -> Vec<Span> {
    let lower = text.to_ascii_lowercase();
    let triggers = [
        "drop table",
        "irreversible",
        "destructive operation",
        "security warning",
        "ordered sequence",
    ];
    let mut spans = Vec::new();
    for trigger in triggers {
        for (idx, _) in lower.match_indices(trigger) {
            if overlaps_any(idx, idx + trigger.len(), code_spans) {
                continue;
            }
            let start = text[..idx].rfind("\n\n").map(|pos| pos + 2).unwrap_or(0);
            let end = text[idx..]
                .find("\n\n")
                .map(|pos| idx + pos)
                .unwrap_or(text.len());
            spans.push(span(start, end));
        }
    }
    merge_spans(spans)
}

fn text_outside_spans(text: &str, excluded: &[Span]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for span in excluded {
        let start = span.start as usize;
        let end = span.end as usize;
        if start > cursor {
            out.push_str(&text[cursor..start]);
        }
        out.push(' ');
        cursor = end.max(cursor);
    }
    if cursor < text.len() {
        out.push_str(&text[cursor..]);
    }
    out
}

fn clutter_hits(text: &str, excluded: &[Span]) -> Vec<ClutterHit> {
    let mut hits = Vec::new();
    for (phrase, replacement) in parse_tsv(CLUTTER)
        .into_iter()
        .chain(parse_tsv(REDUNDANT_PAIRS))
        .chain(parse_tsv(LATINATE_SWAPS))
    {
        for (start, end) in phrase_matches(text, phrase) {
            if overlaps_any(start, end, excluded) {
                continue;
            }
            hits.push(ClutterHit {
                phrase: phrase.to_string(),
                span: span(start, end),
                suggestion: replacement.to_string(),
            });
        }
    }
    hits.sort_by_key(|hit| (hit.span.start, hit.span.end));
    hits
}

fn phrase_matches(text: &str, phrase: &str) -> Vec<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let phrase = phrase.to_ascii_lowercase();
    let mut matches = Vec::new();
    let mut search = 0;
    while let Some(rel) = lower[search..].find(&phrase) {
        let start = search + rel;
        let end = start + phrase.len();
        if is_boundary(&lower, start, true) && is_boundary(&lower, end, false) {
            matches.push((start, end));
        }
        search = end;
    }
    matches
}

fn is_boundary(text: &str, index: usize, left: bool) -> bool {
    let ch = if left {
        text[..index].chars().next_back()
    } else {
        text[index..].chars().next()
    };
    ch.map(|ch| !ch.is_ascii_alphanumeric() && ch != '\'')
        .unwrap_or(true)
}

fn parse_tsv(body: &'static str) -> Vec<(&'static str, &'static str)> {
    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            line.split_once('\t')
        })
        .collect()
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut start = 0;
    for (idx, ch) in text.char_indices() {
        if !matches!(ch, '.' | '!' | '?') {
            continue;
        }
        let candidate = text[start..=idx].trim();
        if candidate.is_empty() || ends_with_abbreviation(candidate) {
            continue;
        }
        sentences.push(candidate.to_string());
        start = idx + ch.len_utf8();
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        sentences.push(tail.to_string());
    }
    sentences
}

fn ends_with_abbreviation(sentence: &str) -> bool {
    let last = sentence
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .trim_matches(|ch: char| !ch.is_ascii_alphabetic() && ch != '.')
        .to_ascii_lowercase();
    matches!(
        last.as_str(),
        "mr." | "mrs." | "ms." | "dr." | "prof." | "sr." | "jr." | "e.g." | "i.e." | "vs."
    )
}

fn sentence_has_passive(sentence: &str) -> bool {
    let words = word_tokens(sentence);
    let be_forms = ["am", "is", "are", "was", "were", "be", "been", "being"];
    for (index, word) in words.iter().enumerate() {
        let lower = word.to_ascii_lowercase();
        if !be_forms.contains(&lower.as_str()) {
            continue;
        }
        for next in words.iter().skip(index + 1).take(3) {
            if is_participle(next) {
                return true;
            }
        }
    }
    false
}

fn is_participle(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    lower.ends_with("ed")
        || lower.ends_with("en")
        || matches!(
            lower.as_str(),
            "built"
                | "done"
                | "made"
                | "known"
                | "seen"
                | "shown"
                | "written"
                | "run"
                | "set"
                | "sent"
                | "kept"
                | "left"
                | "found"
                | "caught"
        )
}

fn is_scored_adverb(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    lower.ends_with("ly")
        && lower.len() > 4
        && !ADVERB_WHITELIST
            .lines()
            .any(|allowed| allowed.trim() == lower)
}

fn is_nominalization(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    lower.ends_with("tion") || lower.ends_with("ment") || lower.ends_with("ance")
}

fn word_tokens(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '\'' || ch == '-' {
            current.push(ch);
        } else if !current.is_empty() {
            words.push(current.trim_matches('-').to_string());
            current.clear();
        }
    }
    if !current.is_empty() {
        words.push(current.trim_matches('-').to_string());
    }
    words.into_iter().filter(|word| !word.is_empty()).collect()
}

fn fidelity(text: &str, source_identifiers: &[String]) -> Fidelity {
    let mut missing = Vec::new();
    for identifier in source_identifiers {
        let variants = identifier_variants(identifier);
        if variants.iter().any(|variant| text.contains(variant)) {
            continue;
        }
        missing.push(identifier.trim().to_string());
    }
    Fidelity {
        preserved: missing.is_empty(),
        missing,
    }
}

fn identifier_variants(identifier: &str) -> Vec<String> {
    let trimmed = identifier.trim();
    let stripped = trimmed.trim_matches(|ch| matches!(ch, '`' | '"' | '\''));
    let mut variants = BTreeSet::new();
    if !trimmed.is_empty() {
        variants.insert(trimmed.to_string());
    }
    if !stripped.is_empty() {
        variants.insert(stripped.to_string());
    }
    variants.into_iter().collect()
}

fn estimate_tokens(text: &str) -> u32 {
    let words = word_tokens(text).len() as u32;
    let punctuation = text.chars().filter(|ch| ch.is_ascii_punctuation()).count() as u32;
    let non_ascii = text.chars().filter(|ch| !ch.is_ascii()).count() as u32;
    words + ((punctuation + 3) / 4) + non_ascii
}

fn flesch_kincaid(sentences: &[String], words: &[String]) -> f32 {
    if words.is_empty() {
        return 0.0;
    }
    let sentence_count = sentences.len().max(1) as f32;
    let word_count = words.len() as f32;
    let syllables = words.iter().map(|word| syllable_count(word)).sum::<u32>() as f32;
    round2(0.39 * (word_count / sentence_count) + 11.8 * (syllables / word_count) - 15.59)
}

fn syllable_count(word: &str) -> u32 {
    let lower = word.to_ascii_lowercase();
    let mut count = 0;
    let mut previous_vowel = false;
    for ch in lower.chars() {
        let vowel = matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
        if vowel && !previous_vowel {
            count += 1;
        }
        previous_vowel = vowel;
    }
    if lower.ends_with('e') && count > 1 {
        count -= 1;
    }
    count.max(1)
}

fn em_dash_count(text: &str) -> u32 {
    text.chars()
        .filter(|ch| matches!(*ch, '\u{2014}' | '\u{2013}'))
        .count() as u32
}

fn ratio(numerator: f32, denominator: f32) -> f32 {
    if denominator <= 0.0 {
        0.0
    } else {
        round2(numerator / denominator)
    }
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        round2(values.iter().sum::<f32>() / values.len() as f32)
    }
}

fn stdev(values: &[f32], mean: f32) -> f32 {
    if values.len() <= 1 {
        return 0.0;
    }
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f32>()
        / values.len() as f32;
    round2(variance.sqrt())
}

fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

fn span(start: usize, end: usize) -> Span {
    Span {
        start: start as u32,
        end: end as u32,
    }
}

fn merge_spans<I>(spans: I) -> Vec<Span>
where
    I: IntoIterator<Item = Span>,
{
    let mut spans = spans.into_iter().collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.start, span.end));
    let mut merged: Vec<Span> = Vec::new();
    for next in spans {
        if let Some(last) = merged.last_mut() {
            if next.start <= last.end {
                last.end = last.end.max(next.end);
                continue;
            }
        }
        merged.push(next);
    }
    merged
}

fn overlaps_any(start: usize, end: usize, spans: &[Span]) -> bool {
    spans
        .iter()
        .any(|span| start < span.end as usize && end > span.start as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluttered_paragraph_reports_expected_axes() {
        let receipt = check(
            "In order to demonstrate the result, the report was written very quickly due to the fact that reviewers asked.",
            Register::Plain,
            &[],
        );

        let phrases = receipt
            .clutter_hits
            .iter()
            .map(|hit| hit.phrase.as_str())
            .collect::<Vec<_>>();
        assert!(phrases.contains(&"in order to"));
        assert!(phrases.contains(&"demonstrate"));
        assert!(phrases.contains(&"very"));
        assert!(phrases.contains(&"due to the fact that"));
        assert_eq!(receipt.passive_rate, 1.0);
        assert!(receipt.adverb_rate > 0.0);
    }

    #[test]
    fn clean_spare_paragraph_stays_inside_core_targets() {
        let receipt = check(
            "Ship the patch. Keep paths exact. Tests prove the claim and receipts carry the proof.",
            Register::Spare,
            &[],
        );

        assert!(receipt.clutter_hits.is_empty());
        assert_eq!(receipt.passive_rate, 0.0);
        assert_eq!(receipt.adverb_rate, 0.0);
        assert!(receipt.sentence_mean >= 3.0);
        assert!(receipt.sentence_mean <= 9.0);
    }

    #[test]
    fn missing_source_identifier_fails_fidelity() {
        let receipt = check(
            "The runtime module changed.",
            Register::Plain,
            &["rustyred-web/src/lib.rs".to_string()],
        );

        assert!(!receipt.fidelity.preserved);
        assert_eq!(receipt.fidelity.missing, vec!["rustyred-web/src/lib.rs"]);
    }

    #[test]
    fn em_dash_is_reported() {
        let text = format!("left{}right", '\u{2014}');
        let receipt = check(&text, Register::Plain, &[]);

        assert_eq!(receipt.em_dash_count, 1);
    }

    #[test]
    fn fenced_code_span_is_skipped_byte_identically() {
        let code = "```rust\nfn main() {\n    println!(\"ok\");\n}\n```";
        let text = format!("Keep this code:\n\n{code}\n\nThe report was written.");
        let receipt = check(&text, Register::Plain, &[]);

        assert_eq!(receipt.code_spans.len(), 1);
        let span = &receipt.code_spans[0];
        assert_eq!(&text[span.start as usize..span.end as usize], code);
        assert_eq!(receipt.passive_rate, 1.0);
    }

    #[test]
    fn commit_message_span_is_skipped_byte_identically() {
        let commit = "fix(runtime): record style receipts";
        let text = format!("Commit message:\n{commit}\n\nThe report was written.");
        let receipt = check(&text, Register::Plain, &[]);

        assert_eq!(receipt.code_spans.len(), 1);
        let span = &receipt.code_spans[0];
        assert_eq!(&text[span.start as usize..span.end as usize], commit);
        assert_eq!(receipt.passive_rate, 1.0);
    }

    #[test]
    fn clarity_break_excludes_destructive_warning_from_scoring() {
        let text = "Warning: DROP TABLE users is irreversible. Confirm backup, tenant, and rollback before running.\n\nThe patch is clear.";
        let receipt = check(text, Register::Plain, &[]);

        assert!(!receipt.clarity_breaks.is_empty());
        assert_eq!(receipt.clutter_hits.len(), 0);
    }

    #[test]
    fn pack_payload_uses_checker_hash() {
        let payload = writing_engineering_pack_payload(Some("sha256:parent"));

        assert_eq!(payload["kind"], json!("skill_pack"));
        assert_eq!(payload["metadata"]["pack_content_hash"], json!(pack_hash()));
        assert_eq!(
            payload["metadata"]["parent_pack_content_hash"],
            json!("sha256:parent")
        );
    }
}
