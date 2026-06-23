use crate::{
    build_grounded_skill_folder, GroundedSkillBuildInput, GroundedSkillProvenance,
    GroundedSkillScript, GroundedSkillScriptLanguage, GroundedSkillSourceRef,
    DEFAULT_GROUNDED_SKILL_EMBEDDER_MODEL,
};

#[test]
fn grounded_skill_folder_defaults_to_qwen3_embedding_8b() {
    let folder = build_grounded_skill_folder(GroundedSkillBuildInput {
        name: "RFC 3339 Parser".to_string(),
        description: "Parse and validate RFC 3339 timestamps".to_string(),
        instructions: "Use the script to validate timestamp strings and return structured errors."
            .to_string(),
        script: GroundedSkillScript {
            language: GroundedSkillScriptLanguage::Python,
            path: None,
            contents: "print('ok')\n".to_string(),
        },
        provenance: GroundedSkillProvenance {
            tenant_id: "theorem".to_string(),
            corpus_id: "code_corpus_v1".to_string(),
            source_refs: vec![GroundedSkillSourceRef {
                node_id: "code:chrono:rfc3339".to_string(),
                uri: Some("https://example.com/chrono".to_string()),
                confidence: 0.92,
            }],
            embedder_model: None,
            fractal_receipt_id: Some("fractal:receipt:1".to_string()),
            confidence: 0.91,
        },
    })
    .unwrap();

    assert_eq!(folder.name, "rfc-3339-parser");
    assert_eq!(
        folder.provenance.embedder_model.as_deref(),
        Some(DEFAULT_GROUNDED_SKILL_EMBEDDER_MODEL)
    );
    let skill_md = folder
        .files
        .iter()
        .find(|file| file.path == "SKILL.md")
        .unwrap();
    assert!(skill_md
        .contents
        .contains("embedder_model: qwen3-embedding-8b"));
    assert!(folder
        .files
        .iter()
        .any(|file| file.path == "scripts/run.py" && file.executable));
    assert!(folder
        .files
        .iter()
        .any(|file| file.path == "theorem.provenance.json"));
}
