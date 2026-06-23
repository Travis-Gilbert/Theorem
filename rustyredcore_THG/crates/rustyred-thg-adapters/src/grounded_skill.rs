//! Grounded Agent Skill output contract.
//!
//! This module is the product-facing artifact builder for the Grounded Skill
//! API. Retrieval, fractal expansion, and encoding produce the script and
//! provenance; this module packages them as an open Agent Skills folder.

use rustyred_thg_core::{ThgError, ThgResult};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const AGENT_SKILL_STANDARD: &str = "open-agent-skills";
pub const DEFAULT_GROUNDED_SKILL_EMBEDDER_MODEL: &str = "qwen3-embedding-8b";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundedSkillScriptLanguage {
    Python,
    TypeScript,
    Shell,
}

impl GroundedSkillScriptLanguage {
    pub fn default_script_path(&self) -> &'static str {
        match self {
            Self::Python => "scripts/run.py",
            Self::TypeScript => "scripts/run.ts",
            Self::Shell => "scripts/run.sh",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GroundedSkillSourceRef {
    pub node_id: String,
    pub uri: Option<String>,
    pub confidence: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GroundedSkillProvenance {
    pub tenant_id: String,
    pub corpus_id: String,
    pub source_refs: Vec<GroundedSkillSourceRef>,
    pub embedder_model: Option<String>,
    pub fractal_receipt_id: Option<String>,
    pub confidence: f32,
}

impl GroundedSkillProvenance {
    pub fn normalized(mut self) -> Self {
        self.tenant_id = self.tenant_id.trim().to_string();
        self.corpus_id = self.corpus_id.trim().to_string();
        self.embedder_model = self
            .embedder_model
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
            .or_else(|| Some(DEFAULT_GROUNDED_SKILL_EMBEDDER_MODEL.to_string()));
        self.fractal_receipt_id = self
            .fractal_receipt_id
            .map(|receipt| receipt.trim().to_string())
            .filter(|receipt| !receipt.is_empty());
        self.confidence = self.confidence.clamp(0.0, 1.0);
        self.source_refs = self
            .source_refs
            .into_iter()
            .filter_map(|mut source| {
                source.node_id = source.node_id.trim().to_string();
                source.uri = source
                    .uri
                    .map(|uri| uri.trim().to_string())
                    .filter(|uri| !uri.is_empty());
                source.confidence = source.confidence.clamp(0.0, 1.0);
                (!source.node_id.is_empty()).then_some(source)
            })
            .collect();
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GroundedSkillScript {
    pub language: GroundedSkillScriptLanguage,
    pub path: Option<String>,
    pub contents: String,
}

impl GroundedSkillScript {
    pub fn normalized(mut self) -> Self {
        self.path = self
            .path
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty())
            .or_else(|| Some(self.language.default_script_path().to_string()));
        self.contents = self.contents.trim_end().to_string();
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GroundedSkillBuildInput {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub script: GroundedSkillScript,
    pub provenance: GroundedSkillProvenance,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GroundedSkillFile {
    pub path: String,
    pub contents: String,
    pub executable: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GroundedSkillFolder {
    pub name: String,
    pub standard: String,
    pub files: Vec<GroundedSkillFile>,
    pub provenance: GroundedSkillProvenance,
}

pub fn build_grounded_skill_folder(
    input: GroundedSkillBuildInput,
) -> ThgResult<GroundedSkillFolder> {
    let name = normalize_skill_name(&input.name)?;
    let description = single_line("description", &input.description)?;
    let instructions = input.instructions.trim().to_string();
    if instructions.is_empty() {
        return Err(ThgError::new(
            "invalid_grounded_skill",
            "instructions are required",
        ));
    }

    let script = input.script.normalized();
    let script_path = script.path.clone().unwrap_or_default();
    validate_script_path(&script_path)?;
    if script.contents.trim().is_empty() {
        return Err(ThgError::new(
            "invalid_grounded_skill",
            "script contents are required",
        ));
    }

    let provenance = input.provenance.normalized();
    validate_provenance(&provenance)?;
    let skill_md = render_skill_md(
        &name,
        &description,
        &instructions,
        &script_path,
        &provenance,
    );
    let provenance_json = serde_json::to_string_pretty(&json!({
        "standard": AGENT_SKILL_STANDARD,
        "name": name,
        "provenance": provenance,
    }))
    .map_err(|error| ThgError::new("grounded_skill_provenance_json", error.to_string()))?;

    Ok(GroundedSkillFolder {
        name,
        standard: AGENT_SKILL_STANDARD.to_string(),
        files: vec![
            GroundedSkillFile {
                path: "SKILL.md".to_string(),
                contents: skill_md,
                executable: false,
            },
            GroundedSkillFile {
                path: script_path,
                contents: script.contents,
                executable: true,
            },
            GroundedSkillFile {
                path: "theorem.provenance.json".to_string(),
                contents: provenance_json,
                executable: false,
            },
        ],
        provenance,
    })
}

fn render_skill_md(
    name: &str,
    description: &str,
    instructions: &str,
    script_path: &str,
    provenance: &GroundedSkillProvenance,
) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nx-theorem-grounding:\n  standard: {AGENT_SKILL_STANDARD}\n  corpus_id: {corpus_id}\n  embedder_model: {embedder_model}\n  confidence: {confidence:.3}\n  source_count: {source_count}\n---\n\n{instructions}\n\nRun `{script_path}` when the user asks for this capability.\n",
        corpus_id = provenance.corpus_id,
        embedder_model = provenance
            .embedder_model
            .as_deref()
            .unwrap_or(DEFAULT_GROUNDED_SKILL_EMBEDDER_MODEL),
        confidence = provenance.confidence,
        source_count = provenance.source_refs.len(),
    )
}

fn normalize_skill_name(raw: &str) -> ThgResult<String> {
    let mut name = raw
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while name.contains("--") {
        name = name.replace("--", "-");
    }
    let name = name.trim_matches('-').to_string();
    if name.is_empty() {
        Err(ThgError::new("invalid_grounded_skill", "name is required"))
    } else {
        Ok(name)
    }
}

fn single_line(field: &str, value: &str) -> ThgResult<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        Err(ThgError::new(
            "invalid_grounded_skill",
            format!("{field} is required"),
        ))
    } else {
        Ok(normalized)
    }
}

fn validate_script_path(path: &str) -> ThgResult<()> {
    if !path.starts_with("scripts/") || path.contains("..") || path.ends_with('/') {
        return Err(ThgError::new(
            "invalid_grounded_skill",
            "script path must be inside scripts/",
        ));
    }
    Ok(())
}

fn validate_provenance(provenance: &GroundedSkillProvenance) -> ThgResult<()> {
    if provenance.tenant_id.is_empty() {
        return Err(ThgError::new(
            "invalid_grounded_skill",
            "provenance tenant_id is required",
        ));
    }
    if provenance.corpus_id.is_empty() {
        return Err(ThgError::new(
            "invalid_grounded_skill",
            "provenance corpus_id is required",
        ));
    }
    if provenance.source_refs.is_empty() {
        return Err(ThgError::new(
            "invalid_grounded_skill",
            "at least one provenance source is required",
        ));
    }
    Ok(())
}
