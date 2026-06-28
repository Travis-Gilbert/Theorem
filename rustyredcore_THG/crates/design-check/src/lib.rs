//! # design-check
//!
//! Static design-engineering checker and skill-pack payload for Theorem harness
//! design systems. The crate owns the runnable static axes from
//! `docs/plans/skill-encoder/design-engineering-corpus-spec.md`: CSS lowering,
//! token lowering, WCAG contrast math, grid/type/motion checks, token linting,
//! component fixture artifact hashes, and honest pending declarations for the
//! render-backed `axe_render` and `apg_behavioral` axes.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

pub mod scout;

pub use scout::{
    apca_contrast_lc, color_fact_from_hex, delta_e2000, design_audit, design_audit_from_json,
    design_drift, design_drift_from_json, design_fact_set_from_dembrandt_json,
    design_fact_set_from_json, design_html_report, design_scout_parity_receipt, design_tokens_dtcg,
    design_tokens_tailwind, facts_hash, AccessibilityFact, BorderFact, BreakpointFact, ColorFact,
    ColorSpaceFact, ComponentFact, ContrastPairFact, CoverageScore, DesignAuditFinding,
    DesignAuditReport, DesignAuditScores, DesignDriftCategory, DesignDriftChange,
    DesignDriftReport, DesignDriftSummary, DesignFactSet, DriftConfig, RadiusFact, RgbFact,
    ShadowFact, SpacingFact, TypographyFact, DEMBRANDT_SYNTHETIC_FIXTURE,
    DESIGN_SCOUT_REFERENCE_COMMIT, DESIGN_SCOUT_REFERENCE_REPO,
};

pub const PACK_ID: &str = "skill-pack:design-engineering-general-v0.1";
pub const PACK_NAME: &str = "design-engineering";
pub const SOURCE_REF: &str = "source:design-engineering-external-corpus-v0.1";
pub const SOURCE_CLASS: &str = "code_corpus_v1";
pub const MARKETPLACE_PATH: &str = "theorems-harness/skills/design-engineering/";

pub const SKILL_MARKDOWN: &str = include_str!("skill/SKILL.md");
pub const STATIC_FIXTURE_CSS: &str = include_str!("fixtures/static-fixture.css");
pub const STATIC_TOKENS_JSON: &str = include_str!("fixtures/tokens.json");
const APG_FIXTURES_JSON: &str = include_str!("fixtures/apg-fixtures.json");
const CORPUS_PACKETS_JSON: &str = include_str!("fixtures/corpus-packets.json");
const VALIDATION_TASKS_JSON: &str = include_str!("fixtures/validation-tasks.json");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignRule {
    pub ruleset: String,
    pub rule_id: String,
    pub severity: String,
    pub checker: String,
    pub source_fact_model: String,
    pub validator_strategy: String,
    pub status: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignAtom {
    pub atom_id: String,
    pub kind: String,
    pub dialect: String,
    pub source_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CssStaticInput {
    pub css: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_json: Option<String>,
    pub grid_base_px: f32,
    pub rem_px: f32,
}

impl Default for CssStaticInput {
    fn default() -> Self {
        Self {
            css: String::new(),
            token_json: None,
            grid_base_px: 4.0,
            rem_px: 16.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckerFinding {
    pub rule_id: String,
    pub checker: String,
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignCheckReport {
    pub checker: String,
    pub pack_id: String,
    pub pack_hash: String,
    pub findings: Vec<CheckerFinding>,
    pub passed: usize,
    pub failed: usize,
    pub pending: usize,
    pub unsupported: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Debug)]
struct Declaration {
    selector: String,
    property: String,
    value: String,
}

pub fn design_rules() -> Vec<DesignRule> {
    let mut rules = Vec::new();
    add_rule(
        &mut rules,
        "accessibility",
        "contrast_minimum_met",
        "promotable",
        "css_static",
        "4.5:1 body text, 3:1 large text and UI components.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "accessibility",
        "target_size_minimum",
        "promotable",
        "css_static",
        "24px floor, 44px preferred touch target where dimensions are declared.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "accessibility",
        "focus_visible_not_removed",
        "promotable",
        "css_static",
        "No outline removal without an equally visible replacement.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "accessibility",
        "reduced_motion_respected",
        "promotable",
        "css_static",
        "Animation or transition declarations require a prefers-reduced-motion path.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "accessibility",
        "form_controls_labeled",
        "promotable",
        "axe_render",
        "Rendered form controls have accessible labels.",
        "pending",
    );
    add_rule(
        &mut rules,
        "accessibility",
        "heading_hierarchy_no_skips",
        "promotable",
        "axe_render",
        "Rendered headings do not skip hierarchy levels.",
        "pending",
    );
    add_rule(
        &mut rules,
        "accessibility",
        "keyboard_contract_matches_apg",
        "promotable",
        "apg_behavioral",
        "Dialog, combobox, tabs, and menu keyboard contracts match APG.",
        "pending",
    );
    add_rule(
        &mut rules,
        "tokens_and_scale",
        "spacing_on_grid",
        "promotable",
        "css_static",
        "Spacing values land on the declared 4px or 8px grid.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "tokens_and_scale",
        "colors_from_token_palette",
        "promotable",
        "token_lint",
        "Raw color literals stay in token definitions, not component declarations.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "tokens_and_scale",
        "type_scale_conformance",
        "promotable",
        "css_static",
        "Font sizes come from the modular scale.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "tokens_and_scale",
        "radii_and_borders_tokenized",
        "promotable",
        "token_lint",
        "Radii and border widths are tokenized.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "typography",
        "measure_in_range",
        "promotable",
        "css_static",
        "Body text measure is between 45 and 75 characters.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "typography",
        "line_height_floor",
        "promotable",
        "css_static",
        "Body line-height is at least 1.4.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "typography",
        "minimum_body_size",
        "promotable",
        "css_static",
        "Default body text is at least 16px.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "layout_grid",
        "gutters_consistent",
        "advisory",
        "css_static",
        "Gutter values come from the spacing scale.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "layout_grid",
        "breakpoints_tokenized",
        "advisory",
        "token_lint",
        "Media query breakpoints use breakpoint tokens.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "motion",
        "duration_within_bounds",
        "advisory",
        "css_static",
        "UI transition durations stay between 100ms and 500ms unless exempted.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "motion",
        "no_unpausable_infinite_animation",
        "advisory",
        "css_static",
        "Infinite animation has a reduced-motion or pause path.",
        "runnable",
    );
    add_rule(
        &mut rules,
        "data_viz",
        "categorical_palette_colorblind_distinguishable",
        "advisory",
        "css_static",
        "Categorical palette pairs stay distinguishable under color-vision simulation.",
        "declared",
    );
    add_rule(
        &mut rules,
        "data_viz",
        "axes_and_series_labeled",
        "advisory",
        "axe_render",
        "Axes and series are labeled in rendered visualization fixtures.",
        "pending",
    );
    rules
}

pub fn lower_css(source_ref: &str, css: &str) -> Vec<DesignAtom> {
    parse_declarations(css)
        .into_iter()
        .enumerate()
        .map(|(index, declaration)| {
            let kind = if declaration.property.starts_with("--") {
                "custom_property_definition"
            } else {
                "css_declaration"
            };
            let atom_id = stable_id(&json!({
                "source_ref": source_ref,
                "index": index,
                "selector": declaration.selector,
                "property": declaration.property,
                "value": declaration.value,
            }));
            DesignAtom {
                atom_id,
                kind: kind.to_string(),
                dialect: "css_declaration_view".to_string(),
                source_ref: source_ref.to_string(),
                selector: Some(declaration.selector),
                property: Some(declaration.property),
                name: None,
                value: Some(declaration.value),
                metadata: Map::new(),
            }
        })
        .chain(media_query_atoms(source_ref, css))
        .collect()
}

pub fn lower_tokens_json(source_ref: &str, json_text: &str) -> Result<Vec<DesignAtom>, String> {
    let value: Value =
        serde_json::from_str(json_text).map_err(|error| format!("invalid token JSON: {error}"))?;
    let mut atoms = Vec::new();
    lower_token_value(source_ref, "", &value, &mut atoms);
    Ok(atoms)
}

pub fn css_static_report(input: CssStaticInput) -> DesignCheckReport {
    let declarations = parse_declarations(&input.css);
    let custom_props = custom_properties(&declarations);
    let mut findings = Vec::new();
    findings.extend(check_contrast(&declarations, &custom_props));
    findings.extend(check_spacing_on_grid(
        &declarations,
        &custom_props,
        input.grid_base_px,
        input.rem_px,
    ));
    findings.extend(check_type_scale(&declarations, &custom_props, input.rem_px));
    findings.push(check_measure(&declarations));
    findings.push(check_line_height(&declarations));
    findings.push(check_minimum_body_size(
        &declarations,
        &custom_props,
        input.rem_px,
    ));
    findings.extend(check_target_size(
        &declarations,
        &custom_props,
        input.rem_px,
    ));
    findings.push(check_focus_visible(&declarations));
    findings.push(check_duration_bounds(&declarations));
    findings.push(check_reduced_motion(&declarations, &input.css));
    findings.push(check_infinite_animation(&declarations, &input.css));
    report("css_static", findings)
}

pub fn token_lint_report(input: CssStaticInput) -> DesignCheckReport {
    let declarations = parse_declarations(&input.css);
    let token_names = input
        .token_json
        .as_deref()
        .and_then(|tokens| lower_tokens_json("inline://tokens", tokens).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|atom| atom.name)
        .collect::<BTreeSet<_>>();
    let findings = vec![
        check_colors_from_tokens(&declarations),
        check_radii_and_borders_tokenized(&declarations, &token_names),
        check_breakpoints_tokenized(&input.css),
    ];
    report("token_lint", findings)
}

pub fn fixture_reports() -> Vec<DesignCheckReport> {
    vec![
        css_static_report(CssStaticInput {
            css: STATIC_FIXTURE_CSS.to_string(),
            token_json: Some(STATIC_TOKENS_JSON.to_string()),
            ..CssStaticInput::default()
        }),
        token_lint_report(CssStaticInput {
            css: STATIC_FIXTURE_CSS.to_string(),
            token_json: Some(STATIC_TOKENS_JSON.to_string()),
            ..CssStaticInput::default()
        }),
        report(
            "axe_render",
            vec![
                pending(
                    "form_controls_labeled",
                    "axe_render substrate is declared but not wired",
                ),
                pending(
                    "heading_hierarchy_no_skips",
                    "axe_render substrate is declared but not wired",
                ),
                pending(
                    "axes_and_series_labeled",
                    "axe_render substrate is declared but not wired",
                ),
            ],
        ),
        report(
            "apg_behavioral",
            vec![pending(
                "keyboard_contract_matches_apg",
                "APG behavioral substrate is declared but not wired",
            )],
        ),
    ]
}

pub fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.trim().strip_prefix('#')?;
    match hex.len() {
        3 => {
            let mut chars = hex.chars();
            let red = repeat_hex(chars.next()?)?;
            let green = repeat_hex(chars.next()?)?;
            let blue = repeat_hex(chars.next()?)?;
            Some(Color { red, green, blue })
        }
        6 => Some(Color {
            red: u8::from_str_radix(&hex[0..2], 16).ok()?,
            green: u8::from_str_radix(&hex[2..4], 16).ok()?,
            blue: u8::from_str_radix(&hex[4..6], 16).ok()?,
        }),
        _ => None,
    }
}

pub fn relative_luminance(color: &Color) -> f32 {
    fn channel(value: u8) -> f32 {
        let value = value as f32 / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.red) + 0.7152 * channel(color.green) + 0.0722 * channel(color.blue)
}

pub fn contrast_ratio(foreground: &Color, background: &Color) -> f32 {
    let fg = relative_luminance(foreground);
    let bg = relative_luminance(background);
    let lighter = fg.max(bg);
    let darker = fg.min(bg);
    (lighter + 0.05) / (darker + 0.05)
}

pub fn pack_hash() -> String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        hash_parts(&[
            ("skill", SKILL_MARKDOWN),
            (
                "rules",
                &serde_json::to_string(&design_rules()).unwrap_or_default(),
            ),
            ("fixture_css", STATIC_FIXTURE_CSS),
            ("fixture_tokens", STATIC_TOKENS_JSON),
            ("dembrandt_synthetic_fixture", DEMBRANDT_SYNTHETIC_FIXTURE),
            ("apg_fixtures", APG_FIXTURES_JSON),
            ("corpus_packets", CORPUS_PACKETS_JSON),
            ("validation_tasks", VALIDATION_TASKS_JSON),
        ])
    })
    .clone()
}

pub fn source_hash() -> String {
    hash_parts(&[
        ("source_ref", SOURCE_REF),
        ("source_class", SOURCE_CLASS),
        ("skill", SKILL_MARKDOWN),
    ])
}

pub fn design_engineering_pack_payload(parent_hash: Option<&str>) -> Value {
    let css_atoms = lower_css(
        "local://design-check/static-fixture.css",
        STATIC_FIXTURE_CSS,
    );
    let token_atoms = lower_tokens_json("local://design-check/tokens.json", STATIC_TOKENS_JSON)
        .unwrap_or_default();
    let fixture_reports = fixture_reports();
    let mut metadata = json!({
        "status": "shadow",
        "promotion_state": "scanned",
        "pack_content_hash": pack_hash(),
        "source_content_hash": source_hash(),
        "source_ref": SOURCE_REF,
        "source_class": SOURCE_CLASS,
        "marketplace_path": MARKETPLACE_PATH,
        "artifacts": {
            "corpus_packets": corpus_packet_artifacts(),
            "rules": rule_artifacts(),
            "fixtures": fixture_artifacts(),
            "validation_tasks": validation_task_artifacts(),
            "lowered_views": {
                "css_declaration_view": css_atoms,
                "design_token_view": token_atoms
            },
            "checker_results": fixture_reports,
            "design_scout": {
                "reference_repo": DESIGN_SCOUT_REFERENCE_REPO,
                "reference_commit": DESIGN_SCOUT_REFERENCE_COMMIT,
                "browser_dependency": "none_for_next_build_cut",
                "parity_receipts": [design_scout_parity_receipt()]
            }
        },
        "provenance": {
            "confidence": "scanned",
            "pack_content_hash": pack_hash(),
            "source_content_hash": source_hash(),
            "lowered_view_count": lowered_view_count(),
            "promotion": {
                "state": "scanned",
                "canonical_ready": false,
                "benchmark_treatment_beats_baseline": false,
                "regression_signals": [],
                "task_count": validation_task_count(),
                "scored_axes": ["css_static", "token_lint"],
                "pending_axes": ["axe_render", "apg_behavioral"]
            }
        },
        "marketplace_export": marketplace_export_manifest()
    });
    if let Some(parent_hash) = parent_hash.filter(|value| !value.trim().is_empty()) {
        metadata["parent_pack_content_hash"] = Value::String(parent_hash.to_string());
    }

    json!({
        "id": PACK_ID,
        "name": PACK_NAME,
        "kind": "skill_pack",
        "title": "Design Engineering",
        "description": "Encoded design-engineering pack for token, CSS, accessibility, typography, motion, layout, APG, and visualization correctness.",
        "directive": SKILL_MARKDOWN,
        "capabilities": [
            "checker_rule",
            "context_atom_template",
            "css_declaration_context",
            "design_token_context",
            "fallback_text_context",
            "native_validator_candidate",
            "source_file_context",
            "validator_contract"
        ],
        "validators": design_rules().into_iter().map(|rule| json!({
            "id": rule.rule_id,
            "kind": "checker_rule",
            "ruleset": rule.ruleset,
            "severity": rule.severity,
            "checker": rule.checker,
            "source_fact_model": rule.source_fact_model,
            "validator_strategy": rule.validator_strategy,
            "status": rule.status
        })).collect::<Vec<_>>(),
        "metadata": metadata
    })
}

fn add_rule(
    rules: &mut Vec<DesignRule>,
    ruleset: &str,
    rule_id: &str,
    severity: &str,
    checker: &str,
    description: &str,
    status: &str,
) {
    rules.push(DesignRule {
        ruleset: ruleset.to_string(),
        rule_id: rule_id.to_string(),
        severity: severity.to_string(),
        checker: checker.to_string(),
        source_fact_model: "design_as_facts".to_string(),
        validator_strategy: "rule_as_checker_query".to_string(),
        status: status.to_string(),
        description: description.to_string(),
    });
}

fn lower_token_value(source_ref: &str, path: &str, value: &Value, atoms: &mut Vec<DesignAtom>) {
    match value {
        Value::Object(map) => {
            let token_value = map.get("$value").or_else(|| map.get("value"));
            if let Some(token_value) = token_value {
                let token_type = map
                    .get("$type")
                    .or_else(|| map.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| infer_token_type(token_value));
                let value_text = token_value_to_string(token_value);
                let mut metadata = Map::new();
                metadata.insert("type".to_string(), Value::String(token_type.to_string()));
                metadata.insert(
                    "aliases".to_string(),
                    Value::Array(
                        aliases_in_value(&value_text)
                            .into_iter()
                            .map(Value::String)
                            .collect(),
                    ),
                );
                atoms.push(DesignAtom {
                    atom_id: stable_id(&json!({
                        "source_ref": source_ref,
                        "path": path,
                        "value": value_text,
                    })),
                    kind: "design_token".to_string(),
                    dialect: "design_token_view".to_string(),
                    source_ref: source_ref.to_string(),
                    selector: None,
                    property: None,
                    name: Some(path.trim_matches('.').to_string()),
                    value: Some(value_text),
                    metadata,
                });
                return;
            }
            for (key, child) in map {
                let next_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                lower_token_value(source_ref, &next_path, child, atoms);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                lower_token_value(source_ref, &format!("{path}.{index}"), child, atoms);
            }
        }
        _ if !path.is_empty() => {
            let value_text = token_value_to_string(value);
            let mut metadata = Map::new();
            metadata.insert(
                "type".to_string(),
                Value::String(infer_token_type(value).to_string()),
            );
            atoms.push(DesignAtom {
                atom_id: stable_id(&json!({
                    "source_ref": source_ref,
                    "path": path,
                    "value": value_text,
                })),
                kind: "design_token".to_string(),
                dialect: "design_token_view".to_string(),
                source_ref: source_ref.to_string(),
                selector: None,
                property: None,
                name: Some(path.trim_matches('.').to_string()),
                value: Some(value_text),
                metadata,
            });
        }
        _ => {}
    }
}

fn media_query_atoms(source_ref: &str, css: &str) -> Vec<DesignAtom> {
    let mut atoms = Vec::new();
    let without_comments = strip_comments(css);
    let mut rest = without_comments.as_str();
    while let Some(index) = rest.find("@media") {
        rest = &rest[index..];
        let Some(open) = rest.find('{') else {
            break;
        };
        let query = rest[..open].trim().to_string();
        atoms.push(DesignAtom {
            atom_id: stable_id(&json!({
                "source_ref": source_ref,
                "query": query,
            })),
            kind: "media_query".to_string(),
            dialect: "css_declaration_view".to_string(),
            source_ref: source_ref.to_string(),
            selector: Some(query.clone()),
            property: None,
            name: None,
            value: Some(query),
            metadata: Map::new(),
        });
        rest = &rest[open + 1..];
    }
    atoms
}

fn parse_declarations(css: &str) -> Vec<Declaration> {
    let css = strip_comments(css);
    let mut declarations = Vec::new();
    let mut rest = css.as_str();
    while let Some(open) = rest.find('{') {
        let selector = rest[..open].trim();
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('}') else {
            break;
        };
        let body = &after_open[..close];
        if !selector.starts_with("@media") && !selector.is_empty() {
            for declaration in body.split(';') {
                let Some((property, value)) = declaration.split_once(':') else {
                    continue;
                };
                let property = property.trim();
                let value = value.trim();
                if property.is_empty() || value.is_empty() {
                    continue;
                }
                declarations.push(Declaration {
                    selector: selector.to_string(),
                    property: property.to_ascii_lowercase(),
                    value: value.to_string(),
                });
            }
        }
        rest = &after_open[close + 1..];
    }
    declarations
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("*/") {
            rest = &after[end + 2..];
        } else {
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

fn custom_properties(declarations: &[Declaration]) -> BTreeMap<String, String> {
    declarations
        .iter()
        .filter(|declaration| declaration.property.starts_with("--"))
        .map(|declaration| (declaration.property.clone(), declaration.value.clone()))
        .collect()
}

fn check_contrast(
    declarations: &[Declaration],
    custom_props: &BTreeMap<String, String>,
) -> Vec<CheckerFinding> {
    let mut by_selector: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
    for declaration in declarations {
        by_selector
            .entry(&declaration.selector)
            .or_default()
            .insert(&declaration.property, &declaration.value);
    }

    let mut findings = Vec::new();
    for (selector, declarations) in by_selector {
        let Some(foreground) = declarations
            .get("color")
            .and_then(|value| resolve_color(value, custom_props, 0))
        else {
            continue;
        };
        let Some(background) = declarations
            .get("background-color")
            .or_else(|| declarations.get("background"))
            .and_then(|value| resolve_color(value, custom_props, 0))
        else {
            continue;
        };
        let ratio = contrast_ratio(&foreground, &background);
        let mut metadata = Map::new();
        metadata.insert("ratio".to_string(), json!(round2(ratio)));
        findings.push(CheckerFinding {
            rule_id: "contrast_minimum_met".to_string(),
            checker: "css_static".to_string(),
            status: if ratio >= 4.5 { "passed" } else { "failed" }.to_string(),
            message: format!("contrast ratio is {:.2}:1", ratio),
            selector: Some(selector.to_string()),
            property: Some("color/background-color".to_string()),
            value: None,
            metadata,
        });
    }
    if findings.is_empty() {
        findings.push(unsupported(
            "contrast_minimum_met",
            "no resolvable foreground/background pairs",
        ));
    }
    findings
}

fn check_spacing_on_grid(
    declarations: &[Declaration],
    custom_props: &BTreeMap<String, String>,
    grid_base_px: f32,
    rem_px: f32,
) -> Vec<CheckerFinding> {
    let mut findings = Vec::new();
    for declaration in declarations {
        if !is_spacing_property(&declaration.property) {
            continue;
        }
        for length in lengths_in_value(&declaration.value, custom_props, rem_px) {
            if length.abs() < 0.01 {
                continue;
            }
            let on_grid = multiple_of(length, grid_base_px);
            findings.push(CheckerFinding {
                rule_id: "spacing_on_grid".to_string(),
                checker: "css_static".to_string(),
                status: if on_grid { "passed" } else { "failed" }.to_string(),
                message: if on_grid {
                    format!("{length:.2}px is on the {grid_base_px:.0}px grid")
                } else {
                    format!("{length:.2}px is off the {grid_base_px:.0}px grid")
                },
                selector: Some(declaration.selector.clone()),
                property: Some(declaration.property.clone()),
                value: Some(declaration.value.clone()),
                metadata: Map::new(),
            });
        }
    }
    if findings.is_empty() {
        findings.push(unsupported(
            "spacing_on_grid",
            "no static spacing declarations were found",
        ));
    }
    findings
}

fn check_type_scale(
    declarations: &[Declaration],
    custom_props: &BTreeMap<String, String>,
    rem_px: f32,
) -> Vec<CheckerFinding> {
    let mut findings = Vec::new();
    for declaration in declarations {
        if declaration.property != "font-size" {
            continue;
        }
        let Some(size) = length_to_px(&declaration.value, custom_props, rem_px, 0) else {
            continue;
        };
        let passed = on_modular_scale(size, 16.0, 1.25);
        findings.push(CheckerFinding {
            rule_id: "type_scale_conformance".to_string(),
            checker: "css_static".to_string(),
            status: if passed { "passed" } else { "failed" }.to_string(),
            message: if passed {
                format!("{size:.2}px is on the 16px/1.25 modular scale")
            } else {
                format!("{size:.2}px is not on the 16px/1.25 modular scale")
            },
            selector: Some(declaration.selector.clone()),
            property: Some(declaration.property.clone()),
            value: Some(declaration.value.clone()),
            metadata: Map::new(),
        });
    }
    if findings.is_empty() {
        findings.push(unsupported(
            "type_scale_conformance",
            "no static font-size declarations were found",
        ));
    }
    findings
}

fn check_measure(declarations: &[Declaration]) -> CheckerFinding {
    for declaration in declarations {
        if !matches!(declaration.property.as_str(), "max-width" | "width") {
            continue;
        }
        if !selector_can_have_body_measure(&declaration.selector) {
            continue;
        }
        if let Some(measure) = ch_value(&declaration.value) {
            return CheckerFinding {
                rule_id: "measure_in_range".to_string(),
                checker: "css_static".to_string(),
                status: if (45.0..=75.0).contains(&measure) {
                    "passed"
                } else {
                    "failed"
                }
                .to_string(),
                message: format!("text measure is {measure:.0}ch"),
                selector: Some(declaration.selector.clone()),
                property: Some(declaration.property.clone()),
                value: Some(declaration.value.clone()),
                metadata: Map::new(),
            };
        }
    }
    unsupported(
        "measure_in_range",
        "no static ch-based body measure declaration was found",
    )
}

fn check_line_height(declarations: &[Declaration]) -> CheckerFinding {
    for declaration in declarations {
        if declaration.property != "line-height" || !selector_targets_body(&declaration.selector) {
            continue;
        }
        let Some(line_height) = line_height_value(&declaration.value) else {
            continue;
        };
        return CheckerFinding {
            rule_id: "line_height_floor".to_string(),
            checker: "css_static".to_string(),
            status: if line_height >= 1.4 {
                "passed"
            } else {
                "failed"
            }
            .to_string(),
            message: format!("body line-height is {line_height:.2}"),
            selector: Some(declaration.selector.clone()),
            property: Some(declaration.property.clone()),
            value: Some(declaration.value.clone()),
            metadata: Map::new(),
        };
    }
    unsupported(
        "line_height_floor",
        "no static body line-height declaration was found",
    )
}

fn check_minimum_body_size(
    declarations: &[Declaration],
    custom_props: &BTreeMap<String, String>,
    rem_px: f32,
) -> CheckerFinding {
    for declaration in declarations {
        if declaration.property != "font-size" || !selector_targets_body(&declaration.selector) {
            continue;
        }
        let Some(size) = length_to_px(&declaration.value, custom_props, rem_px, 0) else {
            continue;
        };
        return CheckerFinding {
            rule_id: "minimum_body_size".to_string(),
            checker: "css_static".to_string(),
            status: if size >= 16.0 { "passed" } else { "failed" }.to_string(),
            message: format!("body font-size is {size:.2}px"),
            selector: Some(declaration.selector.clone()),
            property: Some(declaration.property.clone()),
            value: Some(declaration.value.clone()),
            metadata: Map::new(),
        };
    }
    unsupported(
        "minimum_body_size",
        "no static body font-size declaration was found",
    )
}

fn check_target_size(
    declarations: &[Declaration],
    custom_props: &BTreeMap<String, String>,
    rem_px: f32,
) -> Vec<CheckerFinding> {
    let mut by_selector: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
    for declaration in declarations {
        if matches!(
            declaration.property.as_str(),
            "width" | "height" | "min-width" | "min-height"
        ) {
            by_selector
                .entry(&declaration.selector)
                .or_default()
                .insert(&declaration.property, &declaration.value);
        }
    }
    let mut findings = Vec::new();
    for (selector, declarations) in by_selector {
        let width = declarations
            .get("min-width")
            .or_else(|| declarations.get("width"))
            .and_then(|value| length_to_px(value, custom_props, rem_px, 0));
        let height = declarations
            .get("min-height")
            .or_else(|| declarations.get("height"))
            .and_then(|value| length_to_px(value, custom_props, rem_px, 0));
        let Some(width) = width else {
            continue;
        };
        let Some(height) = height else {
            continue;
        };
        let passed = width >= 24.0 && height >= 24.0;
        findings.push(CheckerFinding {
            rule_id: "target_size_minimum".to_string(),
            checker: "css_static".to_string(),
            status: if passed { "passed" } else { "failed" }.to_string(),
            message: format!("target size is {width:.0}px by {height:.0}px"),
            selector: Some(selector.to_string()),
            property: Some("width/height".to_string()),
            value: None,
            metadata: Map::new(),
        });
    }
    if findings.is_empty() {
        findings.push(unsupported(
            "target_size_minimum",
            "no static width and height pair was found",
        ));
    }
    findings
}

fn check_focus_visible(declarations: &[Declaration]) -> CheckerFinding {
    let mut removed = Vec::new();
    let mut replacement_by_selector = BTreeSet::new();
    for declaration in declarations {
        let focus_selector = declaration.selector.contains(":focus");
        if !focus_selector {
            continue;
        }
        if declaration.property == "outline" && removes_outline(&declaration.value) {
            removed.push(declaration.selector.clone());
        }
        if matches!(
            declaration.property.as_str(),
            "box-shadow" | "border" | "border-color" | "outline-color" | "outline-offset"
        ) && !is_none_value(&declaration.value)
        {
            replacement_by_selector.insert(declaration.selector.clone());
        }
    }
    let failing = removed
        .into_iter()
        .filter(|selector| !replacement_by_selector.contains(selector))
        .collect::<Vec<_>>();
    CheckerFinding {
        rule_id: "focus_visible_not_removed".to_string(),
        checker: "css_static".to_string(),
        status: if failing.is_empty() {
            "passed"
        } else {
            "failed"
        }
        .to_string(),
        message: if failing.is_empty() {
            "focus outlines are preserved or replaced".to_string()
        } else {
            format!(
                "focus outline removed without replacement for {}",
                failing.join(", ")
            )
        },
        selector: failing.first().cloned(),
        property: Some("outline".to_string()),
        value: None,
        metadata: Map::new(),
    }
}

fn check_duration_bounds(declarations: &[Declaration]) -> CheckerFinding {
    let mut durations = Vec::new();
    for declaration in declarations {
        if declaration.property.contains("transition") || declaration.property.contains("animation")
        {
            durations.extend(durations_in_value(&declaration.value));
        }
    }
    if durations.is_empty() {
        return unsupported(
            "duration_within_bounds",
            "no transition or animation duration declaration was found",
        );
    }
    let failing = durations
        .iter()
        .copied()
        .filter(|duration| *duration > 0.0 && (*duration < 100.0 || *duration > 500.0))
        .collect::<Vec<_>>();
    CheckerFinding {
        rule_id: "duration_within_bounds".to_string(),
        checker: "css_static".to_string(),
        status: if failing.is_empty() {
            "passed"
        } else {
            "failed"
        }
        .to_string(),
        message: if failing.is_empty() {
            "transition and animation durations are within 100-500ms".to_string()
        } else {
            format!("out-of-bounds durations: {failing:?}ms")
        },
        selector: None,
        property: Some("transition/animation".to_string()),
        value: None,
        metadata: Map::new(),
    }
}

fn check_reduced_motion(declarations: &[Declaration], css: &str) -> CheckerFinding {
    let has_motion = declarations.iter().any(|declaration| {
        declaration.property.contains("transition") || declaration.property.contains("animation")
    });
    if !has_motion {
        return unsupported(
            "reduced_motion_respected",
            "no transition or animation declaration was found",
        );
    }
    let passed = css.contains("prefers-reduced-motion");
    CheckerFinding {
        rule_id: "reduced_motion_respected".to_string(),
        checker: "css_static".to_string(),
        status: if passed { "passed" } else { "failed" }.to_string(),
        message: if passed {
            "prefers-reduced-motion override is present".to_string()
        } else {
            "motion declarations exist without a prefers-reduced-motion override".to_string()
        },
        selector: None,
        property: Some("transition/animation".to_string()),
        value: None,
        metadata: Map::new(),
    }
}

fn check_infinite_animation(declarations: &[Declaration], css: &str) -> CheckerFinding {
    let has_infinite = declarations.iter().any(|declaration| {
        declaration.property.contains("animation")
            && declaration.value.to_ascii_lowercase().contains("infinite")
    });
    if !has_infinite {
        return CheckerFinding {
            rule_id: "no_unpausable_infinite_animation".to_string(),
            checker: "css_static".to_string(),
            status: "passed".to_string(),
            message: "no infinite animation declarations were found".to_string(),
            selector: None,
            property: Some("animation".to_string()),
            value: None,
            metadata: Map::new(),
        };
    }
    let passed = css.contains("prefers-reduced-motion")
        || css
            .to_ascii_lowercase()
            .contains("animation-play-state: paused");
    CheckerFinding {
        rule_id: "no_unpausable_infinite_animation".to_string(),
        checker: "css_static".to_string(),
        status: if passed { "passed" } else { "failed" }.to_string(),
        message: if passed {
            "infinite animation has a reduced-motion or pause path".to_string()
        } else {
            "infinite animation has no reduced-motion or pause path".to_string()
        },
        selector: None,
        property: Some("animation".to_string()),
        value: None,
        metadata: Map::new(),
    }
}

fn check_colors_from_tokens(declarations: &[Declaration]) -> CheckerFinding {
    let offenders = declarations
        .iter()
        .filter(|declaration| !declaration.property.starts_with("--"))
        .filter(|declaration| contains_hex_color(&declaration.value))
        .map(|declaration| {
            format!(
                "{} {}: {}",
                declaration.selector, declaration.property, declaration.value
            )
        })
        .collect::<Vec<_>>();
    CheckerFinding {
        rule_id: "colors_from_token_palette".to_string(),
        checker: "token_lint".to_string(),
        status: if offenders.is_empty() {
            "passed"
        } else {
            "failed"
        }
        .to_string(),
        message: if offenders.is_empty() {
            "raw color literals are confined to token/custom-property definitions".to_string()
        } else {
            format!(
                "raw color literals outside tokens: {}",
                offenders.join("; ")
            )
        },
        selector: None,
        property: Some("color".to_string()),
        value: None,
        metadata: Map::new(),
    }
}

fn check_radii_and_borders_tokenized(
    declarations: &[Declaration],
    token_names: &BTreeSet<String>,
) -> CheckerFinding {
    let offenders = declarations
        .iter()
        .filter(|declaration| {
            matches!(
                declaration.property.as_str(),
                "border-radius" | "border-width" | "border"
            )
        })
        .filter(|declaration| !is_border_or_radius_tokenized(declaration, token_names))
        .filter(|declaration| !is_zero_value(&declaration.value))
        .map(|declaration| {
            format!(
                "{} {}: {}",
                declaration.selector, declaration.property, declaration.value
            )
        })
        .collect::<Vec<_>>();
    CheckerFinding {
        rule_id: "radii_and_borders_tokenized".to_string(),
        checker: "token_lint".to_string(),
        status: if offenders.is_empty() {
            "passed"
        } else {
            "failed"
        }
        .to_string(),
        message: if offenders.is_empty() {
            "radii and border widths are tokenized".to_string()
        } else {
            format!("untokenized radii or borders: {}", offenders.join("; "))
        },
        selector: None,
        property: Some("border/radius".to_string()),
        value: None,
        metadata: Map::new(),
    }
}

fn check_breakpoints_tokenized(css: &str) -> CheckerFinding {
    let offenders = media_query_atoms("inline://css", css)
        .into_iter()
        .filter_map(|atom| atom.value)
        .filter(|query| query.contains("px") && !query.contains("var("))
        .collect::<Vec<_>>();
    CheckerFinding {
        rule_id: "breakpoints_tokenized".to_string(),
        checker: "token_lint".to_string(),
        status: if offenders.is_empty() {
            "passed"
        } else {
            "failed"
        }
        .to_string(),
        message: if offenders.is_empty() {
            "media query breakpoints are tokenized or absent".to_string()
        } else {
            format!("raw breakpoint queries: {}", offenders.join("; "))
        },
        selector: None,
        property: Some("@media".to_string()),
        value: None,
        metadata: Map::new(),
    }
}

fn report(checker: &str, findings: Vec<CheckerFinding>) -> DesignCheckReport {
    let passed = findings
        .iter()
        .filter(|finding| finding.status == "passed")
        .count();
    let failed = findings
        .iter()
        .filter(|finding| finding.status == "failed")
        .count();
    let pending = findings
        .iter()
        .filter(|finding| finding.status == "pending")
        .count();
    let unsupported = findings
        .iter()
        .filter(|finding| finding.status == "unsupported")
        .count();
    DesignCheckReport {
        checker: checker.to_string(),
        pack_id: PACK_ID.to_string(),
        pack_hash: pack_hash(),
        findings,
        passed,
        failed,
        pending,
        unsupported,
    }
}

fn pending(rule_id: &str, message: &str) -> CheckerFinding {
    CheckerFinding {
        rule_id: rule_id.to_string(),
        checker: "render_pending".to_string(),
        status: "pending".to_string(),
        message: message.to_string(),
        selector: None,
        property: None,
        value: None,
        metadata: Map::new(),
    }
}

fn unsupported(rule_id: &str, message: &str) -> CheckerFinding {
    CheckerFinding {
        rule_id: rule_id.to_string(),
        checker: "css_static".to_string(),
        status: "unsupported".to_string(),
        message: message.to_string(),
        selector: None,
        property: None,
        value: None,
        metadata: Map::new(),
    }
}

fn resolve_color(
    value: &str,
    custom_props: &BTreeMap<String, String>,
    depth: usize,
) -> Option<Color> {
    if depth > 4 {
        return None;
    }
    let value = value.trim();
    if let Some(color) = parse_hex_color(first_hex_token(value).unwrap_or(value)) {
        return Some(color);
    }
    let var_name = var_name(value)?;
    let resolved = custom_props.get(&var_name)?;
    resolve_color(resolved, custom_props, depth + 1)
}

fn var_name(value: &str) -> Option<String> {
    let start = value.find("var(")? + 4;
    let end = value[start..].find(')')? + start;
    Some(value[start..end].split(',').next()?.trim().to_string())
}

fn first_hex_token(value: &str) -> Option<&str> {
    value
        .split_whitespace()
        .find(|part| part.starts_with('#') && parse_hex_color(part).is_some())
}

fn repeat_hex(value: char) -> Option<u8> {
    let text = format!("{value}{value}");
    u8::from_str_radix(&text, 16).ok()
}

fn contains_hex_color(value: &str) -> bool {
    value
        .split(|character: char| character.is_whitespace() || matches!(character, ',' | ')' | '('))
        .any(|part| parse_hex_color(part).is_some())
}

fn is_spacing_property(property: &str) -> bool {
    property == "gap"
        || property == "row-gap"
        || property == "column-gap"
        || property == "inset"
        || property.starts_with("margin")
        || property.starts_with("padding")
        || matches!(property, "top" | "right" | "bottom" | "left")
}

fn lengths_in_value(value: &str, custom_props: &BTreeMap<String, String>, rem_px: f32) -> Vec<f32> {
    value
        .split(|character: char| character.is_whitespace() || matches!(character, ',' | '/'))
        .filter_map(|part| length_to_px(part.trim(), custom_props, rem_px, 0))
        .collect()
}

fn length_to_px(
    value: &str,
    custom_props: &BTreeMap<String, String>,
    rem_px: f32,
    depth: usize,
) -> Option<f32> {
    if depth > 4 {
        return None;
    }
    if let Some(var_name) = var_name(value) {
        let resolved = custom_props.get(&var_name)?;
        return length_to_px(resolved, custom_props, rem_px, depth + 1);
    }
    let value = value
        .trim()
        .trim_matches(|character| matches!(character, ',' | ')' | '('));
    if value == "0" {
        return Some(0.0);
    }
    if let Some(number) = value.strip_suffix("px") {
        return number.parse::<f32>().ok();
    }
    if let Some(number) = value.strip_suffix("rem") {
        return number.parse::<f32>().ok().map(|number| number * rem_px);
    }
    None
}

fn contains_untokenized_nonzero_length(value: &str, rem_px: f32) -> bool {
    let empty_props = BTreeMap::new();
    value
        .split(|character: char| character.is_whitespace() || matches!(character, ',' | '/'))
        .any(|part| {
            let part = part.trim();
            if part.contains("var(") || part.contains('{') {
                return false;
            }
            length_to_px(part, &empty_props, rem_px, 0)
                .map(|length| length.abs() >= 0.01)
                .unwrap_or(false)
        })
}

fn durations_in_value(value: &str) -> Vec<f32> {
    value
        .split(|character: char| character.is_whitespace() || matches!(character, ',' | ')'))
        .filter_map(|part| duration_to_ms(part.trim()))
        .collect()
}

fn duration_to_ms(value: &str) -> Option<f32> {
    if let Some(number) = value.strip_suffix("ms") {
        return number.parse::<f32>().ok();
    }
    if let Some(number) = value.strip_suffix('s') {
        return number.parse::<f32>().ok().map(|number| number * 1000.0);
    }
    None
}

fn multiple_of(value: f32, base: f32) -> bool {
    if base <= 0.0 {
        return true;
    }
    let quotient = value / base;
    (quotient - quotient.round()).abs() < 0.01
}

fn on_modular_scale(value: f32, base: f32, ratio: f32) -> bool {
    (-6..=10).any(|step| {
        let expected = if step >= 0 {
            base * ratio.powi(step)
        } else {
            base / ratio.powi(-step)
        };
        (value - expected).abs() < 0.05
    })
}

fn ch_value(value: &str) -> Option<f32> {
    value.trim().strip_suffix("ch")?.parse::<f32>().ok()
}

fn line_height_value(value: &str) -> Option<f32> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return percent.parse::<f32>().ok().map(|value| value / 100.0);
    }
    value.parse::<f32>().ok()
}

fn selector_targets_body(selector: &str) -> bool {
    selector
        .split(',')
        .any(|part| matches!(part.trim(), "body" | ":root body"))
}

fn selector_can_have_body_measure(selector: &str) -> bool {
    selector.split(',').any(|part| {
        matches!(
            part.trim(),
            "body" | "main" | "article" | "p" | ".prose" | ".content"
        )
    })
}

fn removes_outline(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value == "none" || value == "0" || value == "0 none" || value.starts_with("0 ")
}

fn is_none_value(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "none" | "0")
}

fn is_zero_value(value: &str) -> bool {
    value
        .split_whitespace()
        .all(|part| matches!(part, "0" | "0px" | "0rem"))
}

fn is_tokenized_value(value: &str, token_names: &BTreeSet<String>) -> bool {
    let value = value.trim();
    if value.contains("var(--") || value.contains('{') {
        return true;
    }
    token_names.iter().any(|name| value.contains(name))
}

fn is_border_or_radius_tokenized(
    declaration: &Declaration,
    token_names: &BTreeSet<String>,
) -> bool {
    if !matches!(
        declaration.property.as_str(),
        "border-radius" | "border-width" | "border"
    ) {
        return true;
    }
    !contains_untokenized_nonzero_length(&declaration.value, 16.0)
        && is_tokenized_value(&declaration.value, token_names)
}

fn infer_token_type(value: &Value) -> &'static str {
    let text = token_value_to_string(value);
    if parse_hex_color(&text).is_some() {
        "color"
    } else if text.ends_with("px") || text.ends_with("rem") {
        "dimension"
    } else if text.ends_with("ms") || text.ends_with('s') {
        "duration"
    } else {
        "unknown"
    }
}

fn token_value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

fn aliases_in_value(value: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            break;
        };
        let alias = after[..end].trim();
        if !alias.is_empty() {
            aliases.push(alias.to_string());
        }
        rest = &after[end + 1..];
    }
    aliases
}

fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

fn rule_artifacts() -> Vec<Value> {
    design_rules()
        .into_iter()
        .map(|rule| {
            let body = serde_json::to_value(&rule).unwrap_or_else(|_| json!({}));
            json!({
                "artifact_id": format!("design-rule:{}", rule.rule_id),
                "kind": "checker_rule",
                "ruleset": rule.ruleset,
                "rule_id": rule.rule_id,
                "checker": rule.checker,
                "status": rule.status,
                "source_fact_model": rule.source_fact_model,
                "validator_strategy": rule.validator_strategy,
                "content_hash": stable_id(&body),
                "body": body
            })
        })
        .collect()
}

fn fixture_artifacts() -> Vec<Value> {
    let fixtures: Value = serde_json::from_str(APG_FIXTURES_JSON).unwrap_or_else(|_| json!([]));
    fixtures
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|fixture| {
            let name = fixture
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("fixture");
            json!({
                "artifact_id": format!("design-component-fixture:{name}"),
                "kind": "design_component_fixture",
                "checker_engine": fixture.get("checker_engine").cloned().unwrap_or_else(|| json!("apg_behavioral")),
                "fixture_hash": stable_id(&fixture),
                "content_hash": stable_id(&fixture),
                "parent_hashes": [pack_hash()],
                "body": fixture
            })
        })
        .collect()
}

fn corpus_packet_artifacts() -> Vec<Value> {
    let packets: Value = serde_json::from_str(CORPUS_PACKETS_JSON).unwrap_or_else(|_| json!([]));
    packets
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|packet| {
            let packet_id = packet
                .get("packet_id")
                .and_then(Value::as_str)
                .unwrap_or("packet");
            json!({
                "artifact_id": format!("corpus-packet:{packet_id}"),
                "kind": "corpus_packet",
                "content_hash": stable_id(&packet),
                "body": packet
            })
        })
        .collect()
}

fn validation_task_artifacts() -> Vec<Value> {
    let tasks: Value = serde_json::from_str(VALIDATION_TASKS_JSON).unwrap_or_else(|_| json!([]));
    tasks
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|task| {
            let task_id = task
                .get("task_id")
                .and_then(Value::as_str)
                .unwrap_or("task");
            json!({
                "artifact_id": format!("validation-task:{task_id}"),
                "kind": "held_out_validation_task",
                "content_hash": stable_id(&task),
                "body": task
            })
        })
        .collect()
}

fn validation_task_count() -> usize {
    serde_json::from_str::<Value>(VALIDATION_TASKS_JSON)
        .ok()
        .and_then(|value| value.as_array().map(Vec::len))
        .unwrap_or(0)
}

fn lowered_view_count() -> Value {
    json!({
        "css_declaration_view": lower_css("local://design-check/static-fixture.css", STATIC_FIXTURE_CSS).len(),
        "design_token_view": lower_tokens_json("local://design-check/tokens.json", STATIC_TOKENS_JSON)
            .map(|atoms| atoms.len())
            .unwrap_or(0)
    })
}

fn marketplace_export_manifest() -> Value {
    let provenance = json!({
        "source_ref": SOURCE_REF,
        "source_class": SOURCE_CLASS,
        "confidence": "scanned",
        "pack_content_hash": pack_hash(),
        "source_content_hash": source_hash(),
        "promotion": {
            "state": "scanned",
            "canonical_ready": false,
            "benchmark_treatment_beats_baseline": false,
            "regression_signals": [],
            "task_count": validation_task_count()
        }
    });
    let rules = serde_json::to_string_pretty(&design_rules()).unwrap_or_default();
    json!({
        "root": MARKETPLACE_PATH,
        "files": [
            {
                "path": "theorems-harness/skills/design-engineering/SKILL.md",
                "content_hash": hash_text(SKILL_MARKDOWN)
            },
            {
                "path": "theorems-harness/skills/design-engineering/provenance.json",
                "content_hash": stable_id(&provenance)
            },
            {
                "path": "theorems-harness/skills/design-engineering/references/design-rules.json",
                "content_hash": hash_text(&rules)
            },
            {
                "path": "theorems-harness/skills/design-engineering/references/apg-fixtures.json",
                "content_hash": hash_text(APG_FIXTURES_JSON)
            },
            {
                "path": "theorems-harness/skills/design-engineering/references/corpus-packets.json",
                "content_hash": hash_text(CORPUS_PACKETS_JSON)
            },
            {
                "path": "theorems-harness/skills/design-engineering/references/validation-tasks.json",
                "content_hash": hash_text(VALIDATION_TASKS_JSON)
            },
            {
                "path": "theorems-harness/skills/design-engineering/scripts/design-check-metadata.json",
                "content_hash": stable_id(&lowered_view_count())
            }
        ]
    })
}

fn stable_id(value: &Value) -> String {
    hash_text(&canonical_json(value))
}

fn hash_parts(parts: &[(&str, &str)]) -> String {
    let mut hasher = Sha256::new();
    for (name, body) in parts {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(body.as_bytes());
        hasher.update([0xff]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn canonical_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wcag_contrast_math_matches_expected_order() {
        let ink = parse_hex_color("#2a2823").unwrap();
        let field = parse_hex_color("#f6f5f2").unwrap();
        let ratio = contrast_ratio(&ink, &field);
        assert!(ratio > 11.0);
        assert!(contrast_ratio(&field, &ink) > 11.0);
    }

    #[test]
    fn lowers_css_and_tokens_into_declared_views() {
        let css_atoms = lower_css("fixture.css", STATIC_FIXTURE_CSS);
        let token_atoms = lower_tokens_json("tokens.json", STATIC_TOKENS_JSON).unwrap();
        assert!(css_atoms
            .iter()
            .any(|atom| atom.dialect == "css_declaration_view"));
        assert!(css_atoms
            .iter()
            .any(|atom| atom.kind == "custom_property_definition"));
        assert!(token_atoms
            .iter()
            .any(|atom| atom.dialect == "design_token_view"));
        assert!(token_atoms
            .iter()
            .any(|atom| atom.name.as_deref() == Some("color.field")));
    }

    #[test]
    fn static_fixture_passes_runnable_axes_and_marks_render_pending() {
        let reports = fixture_reports();
        let css_static = reports
            .iter()
            .find(|report| report.checker == "css_static")
            .unwrap();
        let token_lint = reports
            .iter()
            .find(|report| report.checker == "token_lint")
            .unwrap();
        let axe = reports
            .iter()
            .find(|report| report.checker == "axe_render")
            .unwrap();
        assert_eq!(css_static.failed, 0);
        assert_eq!(token_lint.failed, 0);
        assert!(css_static.passed > 0);
        assert!(token_lint.passed > 0);
        assert!(css_static
            .findings
            .iter()
            .any(|finding| { finding.rule_id == "spacing_on_grid" && finding.status == "passed" }));
        assert!(axe.pending > 0);
    }

    #[test]
    fn checker_fails_bad_contrast_and_missing_reduced_motion() {
        let report = css_static_report(CssStaticInput {
            css: ".bad { color: #777; background: #777; transition: opacity 900ms; }".to_string(),
            ..CssStaticInput::default()
        });
        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "contrast_minimum_met" && finding.status == "failed"
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "reduced_motion_respected" && finding.status == "failed"
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "duration_within_bounds" && finding.status == "failed"
        }));
    }

    #[test]
    fn css_static_resolves_custom_property_spacing() {
        let report = css_static_report(CssStaticInput {
            css: ":root { --space-bad: 10px; } .bad { padding: var(--space-bad); }".to_string(),
            ..CssStaticInput::default()
        });

        assert!(report
            .findings
            .iter()
            .any(|finding| { finding.rule_id == "spacing_on_grid" && finding.status == "failed" }));
        assert!(!report.findings.iter().any(|finding| {
            finding.rule_id == "spacing_on_grid" && finding.status == "unsupported"
        }));
    }

    #[test]
    fn token_lint_rejects_raw_border_width_inside_tokenized_shorthand() {
        let report = token_lint_report(CssStaticInput {
            css: ".bad { border: 1px solid var(--color-border); }".to_string(),
            token_json: Some(r##"{ "color": { "border": "#222222" } }"##.to_string()),
            ..CssStaticInput::default()
        });

        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "radii_and_borders_tokenized" && finding.status == "failed"
        }));
    }

    #[test]
    fn pack_payload_carries_rules_provenance_and_marketplace_layout() {
        let payload = design_engineering_pack_payload(Some("sha256:parent"));
        assert_eq!(payload["id"], PACK_ID);
        assert_eq!(payload["kind"], "skill_pack");
        assert_eq!(
            payload["metadata"]["parent_pack_content_hash"],
            "sha256:parent"
        );
        assert_eq!(
            payload["metadata"]["provenance"]["promotion"]["task_count"],
            20
        );
        assert!(payload["validators"].as_array().unwrap().len() >= 20);
        assert_eq!(
            payload["metadata"]["artifacts"]["validation_tasks"]
                .as_array()
                .unwrap()
                .len(),
            20
        );
        assert!(payload["metadata"]["artifacts"]["corpus_packets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|packet| packet["body"]["packet_id"] == "github://design-systems-v0.1"));
        let files = payload["metadata"]["marketplace_export"]["files"]
            .as_array()
            .unwrap();
        assert!(files
            .iter()
            .any(|file| { file["path"] == "theorems-harness/skills/design-engineering/SKILL.md" }));
    }
}
