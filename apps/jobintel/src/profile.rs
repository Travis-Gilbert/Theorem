//! Profile loading. A profile file (profiles/<id>.toml) carries the free-text
//! self-description plus the fixed proof points injected into drafted packs.
//! `ResolvedProfile` is the in-memory form used by rank + draft: text, derived
//! skills, and the embedding.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::embed::Embedder;
use crate::error::{JobIntelError, Result};
use crate::graph::skills_of;
use crate::model::profile_id;

#[derive(Debug, Clone, Deserialize)]
pub struct ProofPoints {
    pub repo: String,
    pub metal_to_model: String,
    pub benchmarks: String,
}

impl Default for ProofPoints {
    fn default() -> Self {
        Self {
            repo: "https://github.com/Travis-Gilbert/Theseus".into(),
            metal_to_model:
                "Rust-native substrate: one in-process graph engine from metal to model.".into(),
            benchmarks: "Durable GraphStore with HNSW vector search and personalized PageRank."
                .into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProfileFile {
    id: String,
    text: String,
    #[serde(default)]
    proof_points: Option<ProofPoints>,
}

/// A profile resolved into the fields rank + draft consume.
#[derive(Debug, Clone)]
pub struct ResolvedProfile {
    /// Graph node id, e.g. "profile:travis".
    pub id: String,
    /// Bare handle, e.g. "travis".
    pub handle: String,
    pub text: String,
    pub skills: Vec<String>,
    pub embedding: Vec<f32>,
    pub proof: ProofPoints,
}

/// Load and resolve a profile from `profiles/<handle>.toml` (or an explicit
/// path). Derives skills via `skills_of` and embeds the profile text.
pub fn load_profile(handle: &str, embedder: &dyn Embedder) -> Result<ResolvedProfile> {
    let path = resolve_path(handle);
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        JobIntelError::Config(format!("could not read profile '{}': {e}", path.display()))
    })?;
    let file: ProfileFile = toml::from_str(&raw).map_err(|e| {
        JobIntelError::Config(format!("profile '{}' parse error: {e}", path.display()))
    })?;

    let text = file.text.trim().to_string();
    let skills = skills_of(&text);
    let embedding = embedder.embed(&text)?;

    Ok(ResolvedProfile {
        id: profile_id(&file.id),
        handle: file.id,
        text,
        skills,
        embedding,
        proof: file.proof_points.unwrap_or_default(),
    })
}

/// If `handle` is a path to a .toml file, use it directly; otherwise look under
/// `profiles/<handle>.toml`.
fn resolve_path(handle: &str) -> PathBuf {
    let direct = if handle.ends_with(".toml") {
        PathBuf::from(handle)
    } else {
        PathBuf::from("profiles").join(format!("{handle}.toml"))
    };

    if direct.is_absolute() || direct.exists() {
        direct
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(direct)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_relative_profile_path_falls_back_to_crate_dir() {
        let path = resolve_path("__missing__/profile.toml");

        assert_eq!(
            path,
            Path::new(env!("CARGO_MANIFEST_DIR")).join("__missing__/profile.toml")
        );
    }
}
