use crate::{contrast_ratio, parse_hex_color, Color, PACK_ID};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const DESIGN_SCOUT_REFERENCE_REPO: &str = "github.com/dembrandt/dembrandt";
pub const DESIGN_SCOUT_REFERENCE_COMMIT: &str = "e7a05893d5d045c01a07008cd616035ad29e7154";
pub const DEMBRANDT_SYNTHETIC_FIXTURE: &str =
    include_str!("fixtures/dembrandt-extraction-synthetic.json");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignFactSet {
    pub source_ref: String,
    #[serde(default)]
    pub colors: Vec<ColorFact>,
    #[serde(default)]
    pub typography: Vec<TypographyFact>,
    #[serde(default)]
    pub spacing: Vec<SpacingFact>,
    #[serde(default)]
    pub radii: Vec<RadiusFact>,
    #[serde(default)]
    pub borders: Vec<BorderFact>,
    #[serde(default)]
    pub shadows: Vec<ShadowFact>,
    #[serde(default)]
    pub components: Vec<ComponentFact>,
    #[serde(default)]
    pub breakpoints: Vec<BreakpointFact>,
    #[serde(default)]
    pub contrast_pairs: Vec<ContrastPairFact>,
    #[serde(default)]
    pub accessibility: Vec<AccessibilityFact>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
}

impl Default for DesignFactSet {
    fn default() -> Self {
        Self {
            source_ref: "inline://design-facts".to_string(),
            colors: Vec::new(),
            typography: Vec::new(),
            spacing: Vec::new(),
            radii: Vec::new(),
            borders: Vec::new(),
            shadows: Vec::new(),
            components: Vec::new(),
            breakpoints: Vec::new(),
            contrast_pairs: Vec::new(),
            accessibility: Vec::new(),
            metadata: Map::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorFact {
    pub hex: String,
    pub rgb: RgbFact,
    pub lch: ColorSpaceFact,
    pub oklch: ColorSpaceFact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_role: Option<String>,
    pub usage_count: u32,
    #[serde(default)]
    pub contexts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RgbFact {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorSpaceFact {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypographyFact {
    pub context: String,
    pub family: String,
    pub size_px: f64,
    pub weight: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure_ch: Option<f64>,
    pub usage_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpacingFact {
    pub value_px: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub usage_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadiusFact {
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_px: Option<f64>,
    pub confidence: String,
    pub usage_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BorderFact {
    pub width: String,
    pub style: String,
    pub color: String,
    pub confidence: String,
    pub usage_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShadowFact {
    pub shadow: String,
    pub confidence: String,
    pub usage_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentFact {
    pub component_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_px: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_px: Option<f64>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BreakpointFact {
    pub px: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContrastPairFact {
    pub foreground: String,
    pub background: String,
    pub context: String,
    #[serde(default)]
    pub non_text: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_size_px: Option<f64>,
    #[serde(default)]
    pub bold: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityFact {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_order: Option<u32>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignAuditFinding {
    pub rule_id: String,
    pub severity: String,
    pub category: String,
    pub group: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub evidence: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignAuditScores {
    pub consistency: u8,
    pub contrast: u8,
    pub accessibility: u8,
    pub readability: u8,
    pub coverage: CoverageScore,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoverageScore {
    pub present: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignAuditReport {
    pub checker: String,
    pub pack_id: String,
    pub facts_hash: String,
    pub findings: Vec<DesignAuditFinding>,
    pub scores: DesignAuditScores,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriftConfig {
    pub color_same: f64,
    pub color_shift: f64,
    pub dim_pct: f64,
    pub dim_shift_pct: f64,
    pub fail_threshold: u8,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            color_same: 2.3,
            color_shift: 15.0,
            dim_pct: 4.0,
            dim_shift_pct: 25.0,
            fail_threshold: 10,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignDriftChange {
    pub category: String,
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignDriftCategory {
    pub category: String,
    pub score: f64,
    pub changed: usize,
    pub added: usize,
    pub removed: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignDriftSummary {
    pub changed: usize,
    pub added: usize,
    pub removed: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignDriftReport {
    pub score: u8,
    pub status: String,
    pub threshold: u8,
    pub summary: DesignDriftSummary,
    pub categories: Vec<DesignDriftCategory>,
    pub changes: Vec<DesignDriftChange>,
}

pub fn design_fact_set_from_json(json_text: &str) -> Result<DesignFactSet, String> {
    let value: Value =
        serde_json::from_str(json_text).map_err(|error| format!("invalid design JSON: {error}"))?;
    if value.get("source_ref").is_some() {
        return serde_json::from_value(value)
            .map_err(|error| format!("invalid design fact set: {error}"));
    }
    design_fact_set_from_dembrandt_value("dembrandt://extraction", &value)
}

pub fn design_fact_set_from_dembrandt_json(
    source_ref: &str,
    json_text: &str,
) -> Result<DesignFactSet, String> {
    let value: Value = serde_json::from_str(json_text)
        .map_err(|error| format!("invalid dembrandt extraction JSON: {error}"))?;
    design_fact_set_from_dembrandt_value(source_ref, &value)
}

pub fn design_fact_set_from_dembrandt_value(
    source_ref: &str,
    value: &Value,
) -> Result<DesignFactSet, String> {
    let mut facts = DesignFactSet {
        source_ref: source_ref.to_string(),
        ..DesignFactSet::default()
    };
    facts.metadata.insert(
        "reference_repo".to_string(),
        Value::String(DESIGN_SCOUT_REFERENCE_REPO.to_string()),
    );
    facts.metadata.insert(
        "reference_commit".to_string(),
        Value::String(DESIGN_SCOUT_REFERENCE_COMMIT.to_string()),
    );
    if let Some(url) = value.get("url").and_then(Value::as_str) {
        facts
            .metadata
            .insert("url".to_string(), Value::String(url.to_string()));
    }

    let semantic = value.pointer("/colors/semantic").unwrap_or(&Value::Null);
    let role_by_hex = semantic_role_map(semantic);
    if let Some(palette) = value.pointer("/colors/palette").and_then(Value::as_array) {
        for item in palette {
            let raw = item
                .get("normalized")
                .or_else(|| item.get("color"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(hex) = normalize_color_hex(raw) else {
                continue;
            };
            let mut fact = color_fact_from_hex(&hex, item_count(item), Vec::new())?;
            fact.semantic_role = role_by_hex.get(&hex).cloned();
            facts.colors.push(fact);
        }
    }
    for (hex, role) in role_by_hex {
        if !facts.colors.iter().any(|color| color.hex == hex) {
            let mut fact = color_fact_from_hex(&hex, 1, Vec::new())?;
            fact.semantic_role = Some(role);
            facts.colors.push(fact);
        }
    }

    let primary = semantic_color(semantic, "primary");
    let background = semantic_color(semantic, "background").or_else(|| Some("#ffffff".to_string()));
    if let (Some(foreground), Some(background)) = (primary, background) {
        facts.contrast_pairs.push(ContrastPairFact {
            foreground,
            background,
            context: "primary_on_canvas".to_string(),
            non_text: false,
            text_size_px: Some(16.0),
            bold: false,
        });
    }

    if let Some(styles) = value
        .pointer("/typography/styles")
        .and_then(Value::as_array)
    {
        for style in styles {
            let Some(size_px) = style.get("size").and_then(value_px) else {
                continue;
            };
            facts.typography.push(TypographyFact {
                context: style
                    .get("context")
                    .and_then(Value::as_str)
                    .unwrap_or("body")
                    .to_string(),
                family: style
                    .get("family")
                    .and_then(Value::as_str)
                    .unwrap_or("system")
                    .to_string(),
                size_px,
                weight: style.get("weight").and_then(value_u16).unwrap_or(400),
                line_height: style.get("lineHeight").and_then(line_height_ratio),
                measure_ch: style.get("measureCh").and_then(value_f64),
                usage_count: item_count(style),
            });
        }
    }

    if let Some(spacing) = value
        .pointer("/spacing/commonValues")
        .and_then(Value::as_array)
    {
        for item in spacing {
            if let Some(px) = item
                .get("numericValue")
                .and_then(value_f64)
                .or_else(|| item.get("px").and_then(value_px))
            {
                facts.spacing.push(SpacingFact {
                    value_px: px,
                    context: item
                        .get("context")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    usage_count: item_count(item),
                });
            }
        }
    }

    if let Some(radii) = value
        .pointer("/borderRadius/values")
        .and_then(Value::as_array)
    {
        for item in radii {
            let raw = item
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if raw.is_empty() {
                continue;
            }
            facts.radii.push(RadiusFact {
                value_px: parse_px_prefix(&raw),
                value: raw,
                confidence: confidence(item),
                usage_count: item_count(item),
            });
        }
    }

    if let Some(borders) = value
        .pointer("/borders/combinations")
        .and_then(Value::as_array)
    {
        for item in borders {
            facts.borders.push(BorderFact {
                width: item
                    .get("width")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                style: item
                    .get("style")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                color: item
                    .get("color")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                confidence: confidence(item),
                usage_count: item_count(item),
            });
        }
    }

    if let Some(shadows) = value.get("shadows").and_then(Value::as_array) {
        for item in shadows {
            if let Some(shadow) = item.get("shadow").and_then(Value::as_str) {
                facts.shadows.push(ShadowFact {
                    shadow: shadow.to_string(),
                    confidence: confidence(item),
                    usage_count: item_count(item),
                });
            }
        }
    }

    if let Some(breakpoints) = value.get("breakpoints").and_then(Value::as_array) {
        for item in breakpoints {
            if let Some(px) = item.get("px").and_then(value_px) {
                facts.breakpoints.push(BreakpointFact { px });
            }
        }
    }

    Ok(facts)
}

pub fn color_fact_from_hex(
    hex: &str,
    usage_count: u32,
    contexts: Vec<String>,
) -> Result<ColorFact, String> {
    let color = parse_hex_color(hex).ok_or_else(|| format!("invalid hex color: {hex}"))?;
    let rgb = RgbFact {
        red: color.red,
        green: color.green,
        blue: color.blue,
    };
    let lab = rgb_to_lab(color.red, color.green, color.blue);
    let lch = lab_to_lch(lab);
    let oklch = rgb_to_oklch(color.red, color.green, color.blue);
    Ok(ColorFact {
        hex: normalize_color_hex(hex).unwrap_or_else(|| hex.to_ascii_lowercase()),
        rgb,
        lch,
        oklch,
        semantic_role: None,
        usage_count,
        contexts,
    })
}

pub fn design_audit(facts: &DesignFactSet) -> DesignAuditReport {
    let mut findings = Vec::new();
    findings.extend(audit_contrast(facts));
    findings.extend(audit_apca(facts));
    findings.extend(audit_target_size(facts));
    findings.extend(audit_color_duplication(facts));
    findings.extend(audit_type_hierarchy(facts));
    findings.extend(audit_type_scale(facts));
    findings.extend(audit_spacing_grid(facts));
    findings.extend(audit_readability(facts));

    let errors = findings
        .iter()
        .filter(|finding| finding.severity == "error")
        .count();
    let warnings = findings
        .iter()
        .filter(|finding| finding.severity == "warn")
        .count();
    DesignAuditReport {
        checker: "design_scout_audit".to_string(),
        pack_id: PACK_ID.to_string(),
        facts_hash: facts_hash(facts),
        scores: audit_scores(facts, &findings),
        findings,
        errors,
        warnings,
    }
}

pub fn design_audit_from_json(json_text: &str) -> Result<DesignAuditReport, String> {
    let facts = design_fact_set_from_json(json_text)?;
    Ok(design_audit(&facts))
}

pub fn design_drift(
    baseline: &DesignFactSet,
    candidate: &DesignFactSet,
    config: DriftConfig,
) -> DesignDriftReport {
    let color = compare_colors(baseline, candidate, &config);
    let typography = compare_typography(baseline, candidate, &config);
    let spacing = compare_dimensions(
        "spacing",
        baseline.spacing.iter().map(|item| item.value_px).collect(),
        candidate.spacing.iter().map(|item| item.value_px).collect(),
        &config,
    );
    let radius = compare_dimensions(
        "radius",
        baseline
            .radii
            .iter()
            .filter(|item| item.confidence != "low")
            .filter_map(|item| item.value_px)
            .filter(|value| (0.0..=500.0).contains(value))
            .collect(),
        candidate
            .radii
            .iter()
            .filter(|item| item.confidence != "low")
            .filter_map(|item| item.value_px)
            .filter(|value| (0.0..=500.0).contains(value))
            .collect(),
        &config,
    );
    let shadow = compare_shadows(baseline, candidate);

    let weighted_parts = [
        (&color, 1.0),
        (&typography, 1.0),
        (&spacing, 0.8),
        (&radius, 0.6),
        (&shadow, 0.6),
    ];
    let total_weight = weighted_parts
        .iter()
        .map(|(_, weight)| *weight)
        .sum::<f64>();
    let weighted = weighted_parts
        .iter()
        .map(|(part, weight)| part.0.score * weight)
        .sum::<f64>();
    let score = ((weighted / total_weight) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8;

    let categories = vec![color.0, typography.0, spacing.0, radius.0, shadow.0];
    let mut changes = Vec::new();
    changes.extend(color.1);
    changes.extend(typography.1);
    changes.extend(spacing.1);
    changes.extend(radius.1);
    changes.extend(shadow.1);
    let summary = changes.iter().fold(
        DesignDriftSummary {
            changed: 0,
            added: 0,
            removed: 0,
        },
        |mut summary, change| {
            match change.kind.as_str() {
                "changed" => summary.changed += 1,
                "added" => summary.added += 1,
                "removed" => summary.removed += 1,
                _ => {}
            }
            summary
        },
    );

    DesignDriftReport {
        score,
        status: if score > config.fail_threshold {
            "drift"
        } else {
            "stable"
        }
        .to_string(),
        threshold: config.fail_threshold,
        summary,
        categories,
        changes,
    }
}

pub fn design_drift_from_json(
    baseline_json: &str,
    candidate_json: &str,
) -> Result<DesignDriftReport, String> {
    let baseline = design_fact_set_from_json(baseline_json)?;
    let candidate = design_fact_set_from_json(candidate_json)?;
    Ok(design_drift(&baseline, &candidate, DriftConfig::default()))
}

pub fn design_tokens_dtcg(facts: &DesignFactSet) -> Value {
    let mut color_tokens = Map::new();
    for (index, color) in facts.colors.iter().enumerate() {
        let key = token_key(
            color
                .semantic_role
                .as_deref()
                .unwrap_or_else(|| color.hex.trim_start_matches('#')),
            index,
        );
        color_tokens.insert(
            key,
            json!({
                "$type": "color",
                "$value": color.hex,
                "$extensions": {
                    "theorem": {
                        "rgb": color.rgb,
                        "lch": color.lch,
                        "oklch": color.oklch,
                        "usage_count": color.usage_count,
                        "contexts": color.contexts
                    }
                }
            }),
        );
    }

    let spacing_tokens = facts
        .spacing
        .iter()
        .enumerate()
        .map(|(index, spacing)| {
            (
                format!("space-{index}"),
                json!({
                    "$type": "dimension",
                    "$value": format!("{}px", trim_float(spacing.value_px)),
                    "$extensions": { "theorem": { "usage_count": spacing.usage_count, "context": spacing.context } }
                }),
            )
        })
        .collect::<Map<_, _>>();

    let radius_tokens = facts
        .radii
        .iter()
        .enumerate()
        .map(|(index, radius)| {
            (
                format!("radius-{index}"),
                json!({
                    "$type": "dimension",
                    "$value": radius.value,
                    "$extensions": { "theorem": { "confidence": radius.confidence, "usage_count": radius.usage_count } }
                }),
            )
        })
        .collect::<Map<_, _>>();

    json!({
        "$schema": "https://tr.designtokens.org/format/",
        "color": Value::Object(color_tokens),
        "spacing": Value::Object(spacing_tokens),
        "radius": Value::Object(radius_tokens)
    })
}

pub fn design_tokens_tailwind(facts: &DesignFactSet) -> Value {
    let colors = facts
        .colors
        .iter()
        .enumerate()
        .map(|(index, color)| {
            (
                token_key(
                    color
                        .semantic_role
                        .as_deref()
                        .unwrap_or_else(|| color.hex.trim_start_matches('#')),
                    index,
                ),
                Value::String(color.hex.clone()),
            )
        })
        .collect::<Map<_, _>>();
    let spacing = facts
        .spacing
        .iter()
        .enumerate()
        .map(|(index, item)| {
            (
                index.to_string(),
                Value::String(format!("{}px", trim_float(item.value_px))),
            )
        })
        .collect::<Map<_, _>>();
    let radius = facts
        .radii
        .iter()
        .enumerate()
        .map(|(index, item)| (format!("r{index}"), Value::String(item.value.clone())))
        .collect::<Map<_, _>>();
    let font_size = facts
        .typography
        .iter()
        .map(|item| {
            (
                sanitize_key(&item.context),
                Value::String(format!("{}px", trim_float(item.size_px))),
            )
        })
        .collect::<Map<_, _>>();
    let box_shadow = facts
        .shadows
        .iter()
        .enumerate()
        .map(|(index, item)| (format!("s{index}"), Value::String(item.shadow.clone())))
        .collect::<Map<_, _>>();
    let screens = facts
        .breakpoints
        .iter()
        .enumerate()
        .map(|(index, item)| {
            (
                format!("bp{index}"),
                Value::String(format!("{}px", trim_float(item.px))),
            )
        })
        .collect::<Map<_, _>>();

    json!({
        "theme": {
            "extend": {
                "colors": colors,
                "spacing": spacing,
                "borderRadius": radius,
                "fontSize": font_size,
                "boxShadow": box_shadow,
                "screens": screens
            }
        }
    })
}

pub fn design_html_report(
    facts: &DesignFactSet,
    audit: &DesignAuditReport,
    drift: Option<&DesignDriftReport>,
) -> String {
    let drift_html = drift
        .map(|report| {
            format!(
                "<section><h2>Drift</h2><p>Status: {}. Score: {}. Threshold: {}.</p></section>",
                escape_html(&report.status),
                report.score,
                report.threshold
            )
        })
        .unwrap_or_default();
    let findings = audit
        .findings
        .iter()
        .map(|finding| {
            format!(
                "<li><strong>{}</strong> [{}:{}] {}</li>",
                escape_html(&finding.rule_id),
                escape_html(&finding.category),
                escape_html(&finding.severity),
                escape_html(&finding.message)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Design Scout Report</title><style>body{{font-family:system-ui,sans-serif;line-height:1.5;margin:2rem;max-width:72rem}}code{{background:#f4f4f4;padding:.1rem .25rem}}li{{margin:.35rem 0}}</style></head><body><h1>Design Scout Report</h1><p>Source: <code>{}</code></p><section><h2>Audit</h2><p>Errors: {}. Warnings: {}. Consistency: {}. Contrast: {}. Accessibility: {}. Readability: {}.</p><ul>{}</ul></section>{}<section><h2>Facts</h2><p>Colors: {}. Type styles: {}. Spacing values: {}. Radii: {}. Shadows: {}. Breakpoints: {}.</p></section></body></html>",
        escape_html(&facts.source_ref),
        audit.errors,
        audit.warnings,
        audit.scores.consistency,
        audit.scores.contrast,
        audit.scores.accessibility,
        audit.scores.readability,
        findings,
        drift_html,
        facts.colors.len(),
        facts.typography.len(),
        facts.spacing.len(),
        facts.radii.len(),
        facts.shadows.len(),
        facts.breakpoints.len()
    )
}

pub fn design_scout_parity_receipt() -> Value {
    let facts = design_fact_set_from_dembrandt_json(
        "fixture://dembrandt/extraction-synthetic",
        DEMBRANDT_SYNTHETIC_FIXTURE,
    )
    .unwrap_or_default();
    let audit = design_audit(&facts);
    json!({
        "artifact_id": "design-scout-parity:dembrandt-synthetic",
        "kind": "design_scout_parity_receipt",
        "reference_repo": DESIGN_SCOUT_REFERENCE_REPO,
        "reference_commit": DESIGN_SCOUT_REFERENCE_COMMIT,
        "training_stream": "learned-scorer",
        "facts_hash": facts_hash(&facts),
        "audit_summary": {
            "errors": audit.errors,
            "warnings": audit.warnings,
            "coverage": audit.scores.coverage
        }
    })
}

pub fn delta_e2000(a: &str, b: &str) -> f64 {
    if a.eq_ignore_ascii_case(b) {
        return 0.0;
    }
    let Some(lab1) = parse_color_to_lab(a) else {
        return 100.0;
    };
    let Some(lab2) = parse_color_to_lab(b) else {
        return 100.0;
    };
    ciede2000(lab1, lab2)
}

pub fn apca_contrast_lc(foreground: &Color, background: &Color) -> f64 {
    let txt_y = apca_luminance(foreground);
    let bg_y = apca_luminance(background);
    if (bg_y - txt_y).abs() < 0.0005 {
        return 0.0;
    }
    let blk_threshold: f64 = 0.022;
    let blk_clamp: f64 = 1.414;
    let txt_y = if txt_y < blk_threshold {
        txt_y + (blk_threshold - txt_y).powf(blk_clamp)
    } else {
        txt_y
    };
    let bg_y = if bg_y < blk_threshold {
        bg_y + (blk_threshold - bg_y).powf(blk_clamp)
    } else {
        bg_y
    };
    let output = if bg_y > txt_y {
        let sapc = (bg_y.powf(0.56) - txt_y.powf(0.57)) * 1.14;
        if sapc < 0.001 {
            0.0
        } else if sapc < 0.035991 {
            (sapc - sapc * 27.7847239587675 * 0.027) * 100.0
        } else {
            (sapc - 0.027) * 100.0
        }
    } else {
        let sapc = (bg_y.powf(0.65) - txt_y.powf(0.62)) * 1.14;
        if sapc > -0.001 {
            0.0
        } else if sapc > -0.035991 {
            (sapc - sapc * 27.7847239587675 * 0.027) * 100.0
        } else {
            (sapc + 0.027) * 100.0
        }
    };
    round2_f64(output)
}

pub fn facts_hash(facts: &DesignFactSet) -> String {
    hash_value(&serde_json::to_value(facts).unwrap_or_else(|_| json!(null)))
}

fn audit_contrast(facts: &DesignFactSet) -> Vec<DesignAuditFinding> {
    let mut findings = Vec::new();
    for pair in &facts.contrast_pairs {
        let Some(foreground) = parse_color(&pair.foreground) else {
            continue;
        };
        let Some(background) = parse_color(&pair.background) else {
            continue;
        };
        let ratio = contrast_ratio(&foreground, &background) as f64;
        let large_text = pair.text_size_px.unwrap_or(16.0) >= 24.0
            || (pair.bold && pair.text_size_px.unwrap_or(16.0) >= 18.66);
        let aa_threshold = if pair.non_text || large_text {
            3.0
        } else {
            4.5
        };
        let aaa_threshold = if large_text { 4.5 } else { 7.0 };
        if ratio < aa_threshold {
            findings.push(finding(
                "wcag_contrast_aa",
                "error",
                "contrast",
                "WCAG",
                format!(
                    "{} contrast is {:.2}:1, below AA {:.1}:1",
                    pair.context, ratio, aa_threshold
                ),
                Some(ratio),
                Some(aa_threshold),
                json!({
                    "foreground": pair.foreground,
                    "background": pair.background,
                    "context": pair.context,
                    "non_text": pair.non_text
                }),
            ));
        } else if ratio < aaa_threshold && !pair.non_text {
            findings.push(finding(
                "wcag_contrast_aaa",
                "warn",
                "contrast",
                "WCAG",
                format!(
                    "{} contrast is {:.2}:1, below AAA {:.1}:1",
                    pair.context, ratio, aaa_threshold
                ),
                Some(ratio),
                Some(aaa_threshold),
                json!({
                    "foreground": pair.foreground,
                    "background": pair.background,
                    "context": pair.context
                }),
            ));
        }
    }
    findings
}

fn audit_apca(facts: &DesignFactSet) -> Vec<DesignAuditFinding> {
    let mut findings = Vec::new();
    for pair in &facts.contrast_pairs {
        let Some(foreground) = parse_color(&pair.foreground) else {
            continue;
        };
        let Some(background) = parse_color(&pair.background) else {
            continue;
        };
        let lc = apca_contrast_lc(&foreground, &background).abs();
        let threshold = if pair.non_text || pair.text_size_px.unwrap_or(16.0) >= 24.0 {
            45.0
        } else {
            60.0
        };
        if lc < threshold {
            findings.push(finding(
                "apca_contrast_lc",
                "warn",
                "contrast",
                "APCA",
                format!(
                    "{} APCA Lc is {:.1}, below {:.0}",
                    pair.context, lc, threshold
                ),
                Some(lc),
                Some(threshold),
                json!({
                    "foreground": pair.foreground,
                    "background": pair.background,
                    "context": pair.context
                }),
            ));
        }
    }
    findings
}

fn audit_target_size(facts: &DesignFactSet) -> Vec<DesignAuditFinding> {
    facts
        .components
        .iter()
        .filter_map(|component| {
            let width = component.width_px?;
            let height = component.height_px?;
            let min_side = width.min(height);
            if min_side < 24.0 {
                Some(finding(
                    "target_size_minimum",
                    "error",
                    "accessibility",
                    "Target Size",
                    format!(
                        "{} target is {}px by {}px, below 24px",
                        component.component_type,
                        trim_float(width),
                        trim_float(height)
                    ),
                    Some(min_side),
                    Some(24.0),
                    json!({ "component_type": component.component_type, "context": component.context }),
                ))
            } else if min_side < 44.0 {
                Some(finding(
                    "target_size_preferred",
                    "warn",
                    "accessibility",
                    "Target Size",
                    format!(
                        "{} target is {}px by {}px, below the 44px preferred touch size",
                        component.component_type,
                        trim_float(width),
                        trim_float(height)
                    ),
                    Some(min_side),
                    Some(44.0),
                    json!({ "component_type": component.component_type, "context": component.context }),
                ))
            } else {
                None
            }
        })
        .collect()
}

fn audit_color_duplication(facts: &DesignFactSet) -> Vec<DesignAuditFinding> {
    let mut findings = Vec::new();
    let mut seen = BTreeSet::new();
    for i in 0..facts.colors.len() {
        for j in i + 1..facts.colors.len() {
            let a = &facts.colors[i].hex;
            let b = &facts.colors[j].hex;
            let delta = delta_e2000(a, b);
            if delta < 1.0 {
                let key = [a.as_str(), b.as_str()]
                    .into_iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join("|");
                if !seen.insert(key) {
                    continue;
                }
                findings.push(finding(
                    "color_delta_e_duplicate",
                    "warn",
                    "consistency",
                    "Color",
                    format!("{a} and {b} are perceptually identical (delta_e {delta:.2})"),
                    Some(delta),
                    Some(1.0),
                    json!({ "color_a": a, "color_b": b }),
                ));
            }
        }
    }
    findings
}

fn audit_type_hierarchy(facts: &DesignFactSet) -> Vec<DesignAuditFinding> {
    let mut by_key: BTreeMap<(i64, u16), Vec<&TypographyFact>> = BTreeMap::new();
    for style in &facts.typography {
        by_key
            .entry(((style.size_px * 100.0).round() as i64, style.weight))
            .or_default()
            .push(style);
    }
    let mut findings = Vec::new();
    for ((size_x100, weight), styles) in by_key {
        if styles.len() < 2 {
            continue;
        }
        let hierarchy_count = styles
            .iter()
            .filter(|style| is_hierarchy_context(&style.context))
            .count();
        let text_count = styles
            .iter()
            .filter(|style| is_text_context(&style.context))
            .count();
        if hierarchy_count >= 2 || (hierarchy_count >= 1 && text_count >= 1) {
            let roles = styles
                .iter()
                .map(|style| style.context.clone())
                .collect::<Vec<_>>();
            findings.push(finding(
                "type_hierarchy_collision",
                "warn",
                "consistency",
                "Typography",
                format!(
                    "{} share {:.2}px / {} with no visual hierarchy",
                    roles.join(", "),
                    size_x100 as f64 / 100.0,
                    weight
                ),
                Some(size_x100 as f64 / 100.0),
                None,
                json!({ "contexts": roles, "weight": weight }),
            ));
        }
    }
    findings
}

fn audit_type_scale(facts: &DesignFactSet) -> Vec<DesignAuditFinding> {
    facts
        .typography
        .iter()
        .filter(|style| !on_modular_scale(style.size_px, 16.0, 1.25))
        .map(|style| {
            finding(
                "type_scale_conformance",
                "warn",
                "consistency",
                "Typography",
                format!(
                    "{} type size {}px is not on the 16px/1.25 modular scale",
                    style.context,
                    trim_float(style.size_px)
                ),
                Some(style.size_px),
                None,
                json!({ "context": style.context, "family": style.family, "weight": style.weight }),
            )
        })
        .collect()
}

fn audit_spacing_grid(facts: &DesignFactSet) -> Vec<DesignAuditFinding> {
    facts
        .spacing
        .iter()
        .filter(|spacing| spacing.value_px.abs() >= 0.01 && !multiple_of(spacing.value_px, 8.0))
        .map(|spacing| {
            finding(
                "spacing_eight_point_grid",
                "warn",
                "consistency",
                "Spacing",
                format!(
                    "{}px is off the 8px spacing grid",
                    trim_float(spacing.value_px)
                ),
                Some(spacing.value_px),
                Some(8.0),
                json!({ "context": spacing.context, "usage_count": spacing.usage_count }),
            )
        })
        .collect()
}

fn audit_readability(facts: &DesignFactSet) -> Vec<DesignAuditFinding> {
    let mut findings = Vec::new();
    for style in facts
        .typography
        .iter()
        .filter(|style| is_text_context(&style.context))
    {
        if style.size_px < 16.0 {
            findings.push(finding(
                "readability_size_floor",
                "error",
                "readability",
                "Typography",
                format!(
                    "{} text size is {}px, below 16px",
                    style.context,
                    trim_float(style.size_px)
                ),
                Some(style.size_px),
                Some(16.0),
                json!({ "context": style.context }),
            ));
        }
        if let Some(line_height) = style.line_height {
            if line_height < 1.4 {
                findings.push(finding(
                    "readability_line_height_floor",
                    "warn",
                    "readability",
                    "Typography",
                    format!(
                        "{} line-height is {:.2}, below 1.40",
                        style.context, line_height
                    ),
                    Some(line_height),
                    Some(1.4),
                    json!({ "context": style.context }),
                ));
            }
        }
        if let Some(measure) = style.measure_ch {
            if !(45.0..=75.0).contains(&measure) {
                findings.push(finding(
                    "readability_measure_band",
                    "warn",
                    "readability",
                    "Typography",
                    format!(
                        "{} measure is {}ch, outside 45-75ch",
                        style.context,
                        trim_float(measure)
                    ),
                    Some(measure),
                    None,
                    json!({ "context": style.context }),
                ));
            }
        }
    }
    findings
}

fn audit_scores(facts: &DesignFactSet, findings: &[DesignAuditFinding]) -> DesignAuditScores {
    DesignAuditScores {
        consistency: score_for(findings, |finding| finding.category == "consistency"),
        contrast: score_for(findings, |finding| finding.category == "contrast"),
        accessibility: score_for(findings, |finding| finding.category == "accessibility"),
        readability: score_for(findings, |finding| finding.category == "readability"),
        coverage: coverage(facts),
    }
}

fn score_for(
    findings: &[DesignAuditFinding],
    predicate: impl Fn(&DesignAuditFinding) -> bool,
) -> u8 {
    let cost = findings
        .iter()
        .filter(|finding| predicate(finding))
        .map(|finding| if finding.severity == "error" { 12 } else { 6 })
        .sum::<u16>();
    100u8.saturating_sub(cost.min(u16::from(u8::MAX)) as u8)
}

fn coverage(facts: &DesignFactSet) -> CoverageScore {
    let present = [
        !facts.colors.is_empty(),
        !facts.typography.is_empty(),
        !facts.spacing.is_empty(),
        !facts.radii.is_empty(),
        !facts.shadows.is_empty(),
        !facts.breakpoints.is_empty(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    CoverageScore { present, total: 6 }
}

fn compare_colors(
    baseline: &DesignFactSet,
    candidate: &DesignFactSet,
    config: &DriftConfig,
) -> (DesignDriftCategory, Vec<DesignDriftChange>) {
    let mut changes = Vec::new();
    let mut used = BTreeSet::new();
    let mut penalty = 0.0;
    let mut total_weight = 0.0;
    let mut changed = 0;
    let mut removed = 0;

    for base in &baseline.colors {
        let weight = color_weight(base);
        total_weight += weight;
        let mut best_idx = None;
        let mut best_delta = f64::INFINITY;
        for (index, cand) in candidate.colors.iter().enumerate() {
            if used.contains(&index) {
                continue;
            }
            let delta = delta_e2000(&base.hex, &cand.hex);
            if delta < best_delta {
                best_delta = delta;
                best_idx = Some(index);
            }
        }
        match best_idx {
            Some(best_idx) if best_delta <= config.color_same => {
                used.insert(best_idx);
            }
            Some(best_idx) if best_delta <= config.color_shift => {
                used.insert(best_idx);
                changed += 1;
                penalty += (best_delta / config.color_shift).clamp(0.0, 1.0) * weight;
                changes.push(DesignDriftChange {
                    category: "color".to_string(),
                    kind: "changed".to_string(),
                    label: base.hex.clone(),
                    before: Some(base.hex.clone()),
                    after: Some(candidate.colors[best_idx].hex.clone()),
                    delta: Some(round1(best_delta)),
                });
            }
            _ => {
                removed += 1;
                penalty += weight;
                changes.push(DesignDriftChange {
                    category: "color".to_string(),
                    kind: "removed".to_string(),
                    label: base.hex.clone(),
                    before: Some(base.hex.clone()),
                    after: None,
                    delta: None,
                });
            }
        }
    }
    let mut added = 0;
    for (index, color) in candidate.colors.iter().enumerate() {
        if used.contains(&index) {
            continue;
        }
        added += 1;
        penalty += 0.5 * color_weight(color);
        changes.push(DesignDriftChange {
            category: "color".to_string(),
            kind: "added".to_string(),
            label: color.hex.clone(),
            before: None,
            after: Some(color.hex.clone()),
            delta: None,
        });
    }
    let score = if total_weight > 0.0 {
        (penalty / total_weight).clamp(0.0, 1.0)
    } else if candidate.colors.is_empty() {
        0.0
    } else {
        1.0
    };
    (
        DesignDriftCategory {
            category: "color".to_string(),
            score,
            changed,
            added,
            removed,
        },
        changes,
    )
}

fn compare_typography(
    baseline: &DesignFactSet,
    candidate: &DesignFactSet,
    config: &DriftConfig,
) -> (DesignDriftCategory, Vec<DesignDriftChange>) {
    let mut buckets: BTreeMap<String, Vec<TypographyFact>> = BTreeMap::new();
    for style in &candidate.typography {
        buckets
            .entry(style.context.to_ascii_lowercase())
            .or_default()
            .push(style.clone());
    }
    let mut changes = Vec::new();
    let mut penalty = 0.0;
    let mut peak: f64 = 0.0;
    let mut changed = 0;
    let mut removed = 0;
    for base in &baseline.typography {
        let key = base.context.to_ascii_lowercase();
        let Some(bucket) = buckets.get_mut(&key) else {
            removed += 1;
            penalty += 1.0;
            peak = peak.max(1.0);
            changes.push(DesignDriftChange {
                category: "typography".to_string(),
                kind: "removed".to_string(),
                label: base.context.clone(),
                before: Some(format_type(base)),
                after: None,
                delta: None,
            });
            continue;
        };
        let (best_index, best_diff) = bucket
            .iter()
            .enumerate()
            .map(|(index, cand)| (index, type_field_diffs(base, cand, config)))
            .min_by_key(|(_, diff)| *diff)
            .unwrap_or((0, 0));
        let cand = bucket.remove(best_index);
        if best_diff > 0 {
            let severity = type_field_penalty(base, &cand, config);
            changed += 1;
            penalty += severity;
            peak = peak.max(severity);
            changes.push(DesignDriftChange {
                category: "typography".to_string(),
                kind: "changed".to_string(),
                label: base.context.clone(),
                before: Some(format_type(base)),
                after: Some(format_type(&cand)),
                delta: None,
            });
        }
    }
    let mut added = 0;
    for styles in buckets.values() {
        for style in styles {
            added += 1;
            penalty += 0.5;
            peak = peak.max(0.5);
            changes.push(DesignDriftChange {
                category: "typography".to_string(),
                kind: "added".to_string(),
                label: style.context.clone(),
                before: None,
                after: Some(format_type(style)),
                delta: None,
            });
        }
    }
    (
        DesignDriftCategory {
            category: "typography".to_string(),
            score: category_score(
                penalty,
                baseline.typography.len(),
                candidate.typography.len(),
            )
            .max(peak),
            changed,
            added,
            removed,
        },
        changes,
    )
}

fn compare_dimensions(
    category: &str,
    baseline: Vec<f64>,
    candidate: Vec<f64>,
    config: &DriftConfig,
) -> (DesignDriftCategory, Vec<DesignDriftChange>) {
    let mut changes = Vec::new();
    let mut used = BTreeSet::new();
    let mut penalty = 0.0;
    let mut changed = 0;
    let mut removed = 0;
    for base in &baseline {
        let mut best_idx = None;
        let mut best_pct = f64::INFINITY;
        for (index, cand) in candidate.iter().enumerate() {
            if used.contains(&index) {
                continue;
            }
            let pct = pct_change(*base, *cand);
            if pct < best_pct {
                best_pct = pct;
                best_idx = Some(index);
            }
        }
        match best_idx {
            Some(best_idx) if best_pct <= config.dim_pct => {
                used.insert(best_idx);
            }
            Some(best_idx) if best_pct <= config.dim_shift_pct => {
                used.insert(best_idx);
                changed += 1;
                penalty += (best_pct / config.dim_shift_pct).clamp(0.0, 1.0);
                changes.push(DesignDriftChange {
                    category: category.to_string(),
                    kind: "changed".to_string(),
                    label: format!("{}px", trim_float(*base)),
                    before: Some(format!("{}px", trim_float(*base))),
                    after: Some(format!("{}px", trim_float(candidate[best_idx]))),
                    delta: Some(round1(best_pct)),
                });
            }
            _ => {
                removed += 1;
                penalty += 1.0;
                changes.push(DesignDriftChange {
                    category: category.to_string(),
                    kind: "removed".to_string(),
                    label: format!("{}px", trim_float(*base)),
                    before: Some(format!("{}px", trim_float(*base))),
                    after: None,
                    delta: None,
                });
            }
        }
    }
    let mut added = 0;
    for (index, cand) in candidate.iter().enumerate() {
        if used.contains(&index) {
            continue;
        }
        added += 1;
        penalty += 0.5;
        changes.push(DesignDriftChange {
            category: category.to_string(),
            kind: "added".to_string(),
            label: format!("{}px", trim_float(*cand)),
            before: None,
            after: Some(format!("{}px", trim_float(*cand))),
            delta: None,
        });
    }
    (
        DesignDriftCategory {
            category: category.to_string(),
            score: category_score(penalty, baseline.len(), candidate.len()),
            changed,
            added,
            removed,
        },
        changes,
    )
}

fn compare_shadows(
    baseline: &DesignFactSet,
    candidate: &DesignFactSet,
) -> (DesignDriftCategory, Vec<DesignDriftChange>) {
    let base = baseline
        .shadows
        .iter()
        .filter(|shadow| shadow.confidence != "low")
        .map(|shadow| normalize_shadow(&shadow.shadow))
        .filter(|shadow| supported_shadow(shadow))
        .collect::<BTreeSet<_>>();
    let cand = candidate
        .shadows
        .iter()
        .filter(|shadow| shadow.confidence != "low")
        .map(|shadow| normalize_shadow(&shadow.shadow))
        .filter(|shadow| supported_shadow(shadow))
        .collect::<BTreeSet<_>>();
    let mut penalty = 0.0;
    let mut removed = 0;
    let mut added = 0;
    let mut changes = Vec::new();
    for shadow in &base {
        if !cand.contains(shadow) {
            removed += 1;
            penalty += 1.0;
            changes.push(DesignDriftChange {
                category: "shadow".to_string(),
                kind: "removed".to_string(),
                label: shadow.clone(),
                before: Some(shadow.clone()),
                after: None,
                delta: None,
            });
        }
    }
    for shadow in &cand {
        if !base.contains(shadow) {
            added += 1;
            penalty += 0.5;
            changes.push(DesignDriftChange {
                category: "shadow".to_string(),
                kind: "added".to_string(),
                label: shadow.clone(),
                before: None,
                after: Some(shadow.clone()),
                delta: None,
            });
        }
    }
    (
        DesignDriftCategory {
            category: "shadow".to_string(),
            score: category_score(penalty, base.len(), cand.len()),
            changed: 0,
            added,
            removed,
        },
        changes,
    )
}

fn semantic_role_map(semantic: &Value) -> BTreeMap<String, String> {
    let mut roles = BTreeMap::new();
    let Some(map) = semantic.as_object() else {
        return roles;
    };
    for (role, value) in map {
        if let Some(hex) = semantic_color_value(value) {
            roles.insert(hex, role.clone());
        }
    }
    roles
}

fn semantic_color(semantic: &Value, role: &str) -> Option<String> {
    semantic
        .get(role)
        .and_then(semantic_color_value)
        .or_else(|| {
            semantic.as_object().and_then(|map| {
                map.iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(role))
                    .and_then(|(_, value)| semantic_color_value(value))
            })
        })
}

fn semantic_color_value(value: &Value) -> Option<String> {
    value.as_str().and_then(normalize_color_hex).or_else(|| {
        value
            .get("color")
            .and_then(Value::as_str)
            .and_then(normalize_color_hex)
    })
}

fn parse_color(value: &str) -> Option<Color> {
    normalize_color_hex(value)
        .as_deref()
        .and_then(parse_hex_color)
}

fn normalize_color_hex(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if let Some(color) = parse_hex_color(trimmed) {
        return Some(format!(
            "#{:02x}{:02x}{:02x}",
            color.red, color.green, color.blue
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    let inner = lower
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))
        .or_else(|| {
            lower
                .strip_prefix("rgba(")
                .and_then(|value| value.strip_suffix(')'))
        })?;
    let parts = inner
        .split(',')
        .take(3)
        .filter_map(|part| part.trim().parse::<u8>().ok())
        .collect::<Vec<_>>();
    if parts.len() == 3 {
        Some(format!("#{:02x}{:02x}{:02x}", parts[0], parts[1], parts[2]))
    } else {
        None
    }
}

fn parse_color_to_lab(value: &str) -> Option<Lab> {
    let color = parse_color(value)?;
    Some(rgb_to_lab(color.red, color.green, color.blue))
}

fn rgb_to_lab(red: u8, green: u8, blue: u8) -> Lab {
    let r = srgb_to_linear(red);
    let g = srgb_to_linear(green);
    let b = srgb_to_linear(blue);
    let x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883;
    let fx = lab_f(x);
    let fy = lab_f(y);
    let fz = lab_f(z);
    Lab {
        l: 116.0 * fy - 16.0,
        a: 500.0 * (fx - fy),
        b: 200.0 * (fy - fz),
    }
}

fn lab_to_lch(lab: Lab) -> ColorSpaceFact {
    let c = (lab.a * lab.a + lab.b * lab.b).sqrt();
    let mut h = lab.b.atan2(lab.a).to_degrees();
    if h < 0.0 {
        h += 360.0;
    }
    ColorSpaceFact {
        l: round2_f64(lab.l),
        c: round2_f64(c),
        h: round2_f64(h),
    }
}

fn rgb_to_oklch(red: u8, green: u8, blue: u8) -> ColorSpaceFact {
    let r = srgb_to_linear(red);
    let g = srgb_to_linear(green);
    let b = srgb_to_linear(blue);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let oklab_l = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let oklab_a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let oklab_b = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
    let c = (oklab_a * oklab_a + oklab_b * oklab_b).sqrt();
    let mut h = oklab_b.atan2(oklab_a).to_degrees();
    if h < 0.0 {
        h += 360.0;
    }
    ColorSpaceFact {
        l: round2_f64(oklab_l),
        c: round3(c),
        h: round2_f64(h),
    }
}

fn srgb_to_linear(value: u8) -> f64 {
    let value = value as f64 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn lab_f(value: f64) -> f64 {
    if value > 0.008856 {
        value.cbrt()
    } else {
        (903.3 * value + 16.0) / 116.0
    }
}

#[derive(Clone, Copy)]
struct Lab {
    l: f64,
    a: f64,
    b: f64,
}

fn ciede2000(lab1: Lab, lab2: Lab) -> f64 {
    let c1 = (lab1.a.powi(2) + lab1.b.powi(2)).sqrt();
    let c2 = (lab2.a.powi(2) + lab2.b.powi(2)).sqrt();
    let cab = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cab.powi(7) / (cab.powi(7) + 25.0_f64.powi(7))).sqrt());
    let a1p = lab1.a * (1.0 + g);
    let a2p = lab2.a * (1.0 + g);
    let c1p = (a1p.powi(2) + lab1.b.powi(2)).sqrt();
    let c2p = (a2p.powi(2) + lab2.b.powi(2)).sqrt();
    let h1p = hue_degrees(lab1.b, a1p);
    let h2p = hue_degrees(lab2.b, a2p);
    let dlp = lab2.l - lab1.l;
    let dcp = c2p - c1p;
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let dhp = 2.0 * (c1p * c2p).sqrt() * (dhp / 2.0).to_radians().sin();
    let lpm = (lab1.l + lab2.l) / 2.0;
    let cpm = (c1p + c2p) / 2.0;
    let hpm = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hpm - 30.0).to_radians().cos()
        + 0.24 * (2.0 * hpm).to_radians().cos()
        + 0.32 * (3.0 * hpm + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hpm - 63.0).to_radians().cos();
    let sl = 1.0 + 0.015 * (lpm - 50.0).powi(2) / (20.0 + (lpm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cpm;
    let sh = 1.0 + 0.015 * cpm * t;
    let rc = 2.0 * (cpm.powi(7) / (cpm.powi(7) + 25.0_f64.powi(7))).sqrt();
    let dtheta = 30.0 * (-(((hpm - 275.0) / 25.0).powi(2))).exp();
    let rt = -(2.0 * dtheta).to_radians().sin() * rc;
    ((dlp / sl).powi(2) + (dcp / sc).powi(2) + (dhp / sh).powi(2) + rt * (dcp / sc) * (dhp / sh))
        .sqrt()
}

fn hue_degrees(y: f64, x: f64) -> f64 {
    let mut h = y.atan2(x).to_degrees();
    if h < 0.0 {
        h += 360.0;
    }
    h
}

fn apca_luminance(color: &Color) -> f64 {
    0.2126729 * (color.red as f64 / 255.0).powf(2.4)
        + 0.7151522 * (color.green as f64 / 255.0).powf(2.4)
        + 0.0721750 * (color.blue as f64 / 255.0).powf(2.4)
}

fn value_px(value: &Value) -> Option<f64> {
    value_f64(value).or_else(|| value.as_str().and_then(parse_px_prefix))
}

fn value_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|number| number as f64))
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
}

fn value_u16(value: &Value) -> Option<u16> {
    value
        .as_u64()
        .and_then(|number| u16::try_from(number).ok())
        .or_else(|| value.as_str().and_then(|text| text.parse::<u16>().ok()))
}

fn line_height_ratio(value: &Value) -> Option<f64> {
    if let Some(number) = value_f64(value) {
        return Some(if number > 4.0 { number / 16.0 } else { number });
    }
    let text = value.as_str()?.trim();
    if let Some(px) = parse_px_prefix(text) {
        return Some(px / 16.0);
    }
    text.strip_suffix('%')
        .and_then(|number| number.parse::<f64>().ok())
        .map(|number| number / 100.0)
}

fn parse_px_prefix(value: &str) -> Option<f64> {
    let number = value
        .trim()
        .trim_start_matches("calc(")
        .split(|character: char| {
            !(character.is_ascii_digit() || character == '.' || character == '-')
        })
        .next()
        .unwrap_or_default();
    if number.is_empty() {
        None
    } else {
        number.parse::<f64>().ok()
    }
}

fn item_count(value: &Value) -> u32 {
    value
        .get("count")
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .unwrap_or(1)
}

fn confidence(value: &Value) -> String {
    value
        .get("confidence")
        .and_then(Value::as_str)
        .unwrap_or("medium")
        .to_string()
}

fn finding(
    rule_id: &str,
    severity: &str,
    category: &str,
    group: &str,
    message: String,
    measured: Option<f64>,
    threshold: Option<f64>,
    evidence: Value,
) -> DesignAuditFinding {
    DesignAuditFinding {
        rule_id: rule_id.to_string(),
        severity: severity.to_string(),
        category: category.to_string(),
        group: group.to_string(),
        message,
        measured: measured.map(round2_f64),
        threshold,
        evidence: evidence.as_object().cloned().unwrap_or_default(),
    }
}

fn is_hierarchy_context(context: &str) -> bool {
    let context = context.to_ascii_lowercase();
    context.contains("heading")
        || context.contains("display")
        || context.contains("title")
        || matches!(context.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

fn is_text_context(context: &str) -> bool {
    let context = context.to_ascii_lowercase();
    context.contains("body")
        || context.contains("text")
        || context.contains("paragraph")
        || matches!(context.as_str(), "p")
}

fn on_modular_scale(value: f64, base: f64, ratio: f64) -> bool {
    (-6..=10).any(|step| {
        let expected = if step >= 0 {
            base * ratio.powi(step)
        } else {
            base / ratio.powi(-step)
        };
        (value - expected).abs() < 0.05
    })
}

fn multiple_of(value: f64, base: f64) -> bool {
    if base <= 0.0 {
        return true;
    }
    let quotient = value / base;
    (quotient - quotient.round()).abs() < 0.01
}

fn color_weight(color: &ColorFact) -> f64 {
    let role_weight = match color
        .semantic_role
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "accent" | "primary" | "brand" => 3.0,
        "cta" => 2.0,
        "secondary" => 1.5,
        "surface" | "background" => 0.3,
        _ => 1.0,
    };
    role_weight * (color.usage_count.max(1) as f64).sqrt()
}

fn type_field_diffs(base: &TypographyFact, cand: &TypographyFact, config: &DriftConfig) -> u8 {
    let mut diffs = 0;
    if norm_family(&base.family) != norm_family(&cand.family) {
        diffs += 1;
    }
    if pct_change(base.size_px, cand.size_px) > config.dim_pct {
        diffs += 1;
    }
    if base.weight != cand.weight {
        diffs += 1;
    }
    diffs
}

fn type_field_penalty(base: &TypographyFact, cand: &TypographyFact, config: &DriftConfig) -> f64 {
    let mut penalty: f64 = 0.0;
    if norm_family(&base.family) != norm_family(&cand.family) {
        penalty += 1.0;
    }
    if base.weight != cand.weight {
        penalty += 0.5;
    }
    let size_pct = pct_change(base.size_px, cand.size_px);
    if size_pct > config.dim_pct {
        penalty += (size_pct / config.dim_shift_pct).clamp(0.0, 1.0);
    }
    penalty.clamp(0.0, 1.0)
}

fn norm_family(value: &str) -> String {
    value
        .split(',')
        .next()
        .unwrap_or(value)
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
}

fn format_type(value: &TypographyFact) -> String {
    format!(
        "{} {}px/{}",
        value.family,
        trim_float(value.size_px),
        value.weight
    )
}

fn pct_change(a: f64, b: f64) -> f64 {
    if a == 0.0 {
        if b == 0.0 {
            0.0
        } else {
            100.0
        }
    } else {
        ((a - b).abs() / a.abs()) * 100.0
    }
}

fn category_score(penalty: f64, base_count: usize, candidate_count: usize) -> f64 {
    if base_count == 0 {
        if candidate_count > 0 {
            1.0
        } else {
            0.0
        }
    } else {
        (penalty / base_count as f64).clamp(0.0, 1.0)
    }
}

fn normalize_shadow(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn supported_shadow(value: &str) -> bool {
    !value.contains("oklab(") && !value.contains("oklch(") && !value.contains("color(")
}

fn token_key(value: &str, index: usize) -> String {
    let key = sanitize_key(value);
    if key.is_empty() {
        format!("token-{index}")
    } else {
        key
    }
}

fn sanitize_key(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn trim_float(value: f64) -> String {
    let rounded = round2_f64(value);
    if (rounded - rounded.round()).abs() < 0.001 {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded:.2}")
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round2_f64(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn hash_value(value: &Value) -> String {
    let mut hasher = Sha256::new();
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    hasher.update(text.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_facts() -> DesignFactSet {
        DesignFactSet {
            source_ref: "test://clean".to_string(),
            colors: vec![
                color_fact_from_hex("#133174", 40, Vec::new()).unwrap(),
                color_fact_from_hex("#ffffff", 30, Vec::new()).unwrap(),
            ],
            typography: vec![
                TypographyFact {
                    context: "body".to_string(),
                    family: "Inter".to_string(),
                    size_px: 16.0,
                    weight: 400,
                    line_height: Some(1.5),
                    measure_ch: Some(64.0),
                    usage_count: 10,
                },
                TypographyFact {
                    context: "heading-1".to_string(),
                    family: "Inter".to_string(),
                    size_px: 31.25,
                    weight: 700,
                    line_height: Some(1.2),
                    measure_ch: None,
                    usage_count: 2,
                },
            ],
            spacing: vec![SpacingFact {
                value_px: 16.0,
                context: Some("padding".to_string()),
                usage_count: 10,
            }],
            radii: vec![RadiusFact {
                value: "8px".to_string(),
                value_px: Some(8.0),
                confidence: "high".to_string(),
                usage_count: 8,
            }],
            shadows: vec![ShadowFact {
                shadow: "0 1px 2px rgba(0,0,0,.1)".to_string(),
                confidence: "high".to_string(),
                usage_count: 4,
            }],
            breakpoints: vec![BreakpointFact { px: 768.0 }],
            contrast_pairs: vec![ContrastPairFact {
                foreground: "#133174".to_string(),
                background: "#ffffff".to_string(),
                context: "primary_on_canvas".to_string(),
                non_text: false,
                text_size_px: Some(16.0),
                bold: false,
            }],
            ..DesignFactSet::default()
        }
    }

    #[test]
    fn delta_e2000_matches_reference_bounds() {
        assert_eq!(delta_e2000("#ff0000", "#ff0000"), 0.0);
        assert!(delta_e2000("#ff0000", "#fe0000") < 1.0);
        assert!(delta_e2000("#ff0000", "#0000ff") > 40.0);
        assert_eq!(delta_e2000("garbage", "#ffffff"), 100.0);
    }

    #[test]
    fn apca_reports_high_black_white_contrast() {
        let black = parse_hex_color("#000000").unwrap();
        let white = parse_hex_color("#ffffff").unwrap();
        assert!(apca_contrast_lc(&black, &white).abs() > 100.0);
        let grey = parse_hex_color("#eeeeee").unwrap();
        assert!(apca_contrast_lc(&grey, &white).abs() < 20.0);
    }

    #[test]
    fn dembrandt_fixture_normalizes_to_design_facts() {
        let facts =
            design_fact_set_from_dembrandt_json("fixture://dembrandt", DEMBRANDT_SYNTHETIC_FIXTURE)
                .expect("fixture parses");
        assert!(facts
            .colors
            .iter()
            .any(|color| color.semantic_role.as_deref() == Some("primary")));
        assert!(facts.typography.iter().any(|style| style.context == "body"));
        assert!(facts
            .contrast_pairs
            .iter()
            .any(|pair| pair.context == "primary_on_canvas"));
        assert_eq!(coverage(&facts).present, 6);
    }

    #[test]
    fn clean_design_facts_have_no_errors() {
        let report = design_audit(&clean_facts());
        assert_eq!(report.errors, 0);
        assert!(report.scores.coverage.present >= 5);
    }

    #[test]
    fn known_bad_facts_emit_expected_findings() {
        let mut facts = clean_facts();
        facts
            .colors
            .push(color_fact_from_hex("#133074", 1, Vec::new()).unwrap());
        facts.contrast_pairs = vec![ContrastPairFact {
            foreground: "#9aa0ff".to_string(),
            background: "#ffffff".to_string(),
            context: "bad_primary".to_string(),
            non_text: false,
            text_size_px: Some(16.0),
            bold: false,
        }];
        facts.typography = vec![
            TypographyFact {
                context: "heading-2".to_string(),
                family: "Inter".to_string(),
                size_px: 40.0,
                weight: 700,
                line_height: Some(1.2),
                measure_ch: None,
                usage_count: 1,
            },
            TypographyFact {
                context: "body".to_string(),
                family: "Inter".to_string(),
                size_px: 12.0,
                weight: 700,
                line_height: Some(1.1),
                measure_ch: Some(88.0),
                usage_count: 1,
            },
        ];
        facts.spacing.push(SpacingFact {
            value_px: 13.0,
            context: Some("padding".to_string()),
            usage_count: 1,
        });
        facts.components.push(ComponentFact {
            component_type: "button".to_string(),
            context: Some("cta".to_string()),
            width_px: Some(20.0),
            height_px: Some(20.0),
            metadata: Map::new(),
        });

        let report = design_audit(&facts);
        let ids = report
            .findings
            .iter()
            .map(|finding| finding.rule_id.as_str())
            .collect::<BTreeSet<_>>();
        assert!(ids.contains("wcag_contrast_aa"));
        assert!(ids.contains("color_delta_e_duplicate"));
        assert!(ids.contains("spacing_eight_point_grid"));
        assert!(ids.contains("target_size_minimum"));
        assert!(ids.contains("readability_size_floor"));
    }

    #[test]
    fn normalized_fact_sets_can_be_colorless() {
        let facts = DesignFactSet {
            source_ref: "fixture://typography-only".to_string(),
            typography: vec![TypographyFact {
                context: "body".to_string(),
                family: "Inter".to_string(),
                size_px: 16.0,
                weight: 400,
                line_height: Some(1.5),
                measure_ch: Some(64.0),
                usage_count: 1,
            }],
            ..DesignFactSet::default()
        };
        let json = serde_json::to_string(&facts).unwrap();
        let parsed = design_fact_set_from_json(&json).expect("normalized facts parse");
        assert_eq!(parsed.source_ref, "fixture://typography-only");
        assert_eq!(parsed.colors.len(), 0);
        assert_eq!(parsed.typography.len(), 1);
    }

    #[test]
    fn audit_scores_saturate_large_finding_counts() {
        let findings = (0..300)
            .map(|index| DesignAuditFinding {
                rule_id: format!("rule-{index}"),
                severity: "error".to_string(),
                category: "contrast".to_string(),
                group: "fixture".to_string(),
                message: "fixture".to_string(),
                measured: Some(index as f64),
                threshold: Some(1.0),
                evidence: Map::new(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            score_for(&findings, |finding| finding.category == "contrast"),
            0
        );
    }

    #[test]
    fn drift_counts_empty_candidate_palette_and_dimensions_as_removed() {
        let baseline = clean_facts();
        let mut candidate = baseline.clone();
        candidate.colors.clear();
        candidate.spacing.clear();
        candidate.radii.clear();

        let report = design_drift(&baseline, &candidate, DriftConfig::default());
        let color = report
            .categories
            .iter()
            .find(|category| category.category == "color")
            .expect("color category");
        assert_eq!(color.removed, baseline.colors.len());
        assert_eq!(
            report
                .changes
                .iter()
                .filter(|change| change.category == "color" && change.kind == "removed")
                .count(),
            baseline.colors.len()
        );

        let spacing = report
            .categories
            .iter()
            .find(|category| category.category == "spacing")
            .expect("spacing category");
        assert_eq!(spacing.removed, baseline.spacing.len());

        let radius = report
            .categories
            .iter()
            .find(|category| category.category == "radius")
            .expect("radius category");
        assert_eq!(radius.removed, baseline.radii.len());
    }

    #[test]
    fn drift_ignores_low_confidence_radius_and_shadow() {
        let mut baseline = clean_facts();
        baseline.radii.push(RadiusFact {
            value: "2px".to_string(),
            value_px: Some(2.0),
            confidence: "low".to_string(),
            usage_count: 1,
        });
        baseline.shadows.push(ShadowFact {
            shadow: "rgb(128,128,128) 0px 0px 5px 0px".to_string(),
            confidence: "low".to_string(),
            usage_count: 1,
        });
        let mut candidate = baseline.clone();
        candidate.radii.retain(|radius| radius.confidence != "low");
        candidate
            .shadows
            .retain(|shadow| shadow.confidence != "low");

        let report = design_drift(&baseline, &candidate, DriftConfig::default());
        assert_eq!(report.status, "stable");
        assert!(!report
            .changes
            .iter()
            .any(|change| change.category == "radius" || change.category == "shadow"));
    }

    #[test]
    fn token_and_html_outputs_are_structured() {
        let facts = clean_facts();
        let audit = design_audit(&facts);
        let dtcg = design_tokens_dtcg(&facts);
        let tailwind = design_tokens_tailwind(&facts);
        let html = design_html_report(&facts, &audit, None);
        assert!(dtcg["color"].is_object());
        assert!(tailwind["theme"]["extend"]["colors"].is_object());
        assert!(html.contains("Design Scout Report"));
    }
}
