use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub const DATALOG_RULE_IDS: [&str; 14] = [
    "unsupported_claim",
    "dependent_claim",
    "source_reused_support",
    "likely_duplicate_entity",
    "evidence_path_too_long",
    "claim_has_no_independent_support",
    "object_in_unresolved_tension_neighborhood",
    "code_symbol_touched_by_failing_postmortem_pattern",
    "context_atom_tainted_by_generated_artifact",
    "private_source_reaches_export_candidate",
    "demolition_window",
    "conflict_set",
    "vacancy_duration",
    "ownership_chain",
];

pub fn canonical_json(value: &Value) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| format!("could not serialize JSON: {err}"))
}

pub fn stable_hash_value(value: &Value) -> Result<String, String> {
    Ok(sha256_hex(&canonical_json(value)?))
}

pub fn stable_hash_json(payload_json: &str) -> Result<String, String> {
    let payload = parse_json(payload_json)?;
    stable_hash_value(&payload)
}

pub fn derive_datalog_receipt_from_json(payload_json: &str) -> Result<Value, String> {
    let payload = parse_json(payload_json)?;
    derive_datalog_receipt(&payload)
}

pub fn derive_datalog_receipt(payload: &Value) -> Result<Value, String> {
    let (raw_facts, requested_rule_ids) = datalog_payload_parts(payload)?;
    let facts = normalize_facts(raw_facts)?;
    let index = facts_by_relation(&facts);
    let selected_rule_ids = selected_rule_ids(requested_rule_ids);
    let mut warnings: Vec<String> = Vec::new();
    let mut derived: Vec<Value> = Vec::new();

    for rule_id in &selected_rule_ids {
        match rule_id.as_str() {
            "unsupported_claim" => derived.extend(unsupported_claim(&index)?),
            "dependent_claim" => derived.extend(dependent_claim(&index)?),
            "source_reused_support" => derived.extend(source_reused_support(&index)?),
            "likely_duplicate_entity" => derived.extend(likely_duplicate_entity(&index)?),
            "evidence_path_too_long" => derived.extend(evidence_path_too_long(&index)?),
            "claim_has_no_independent_support" => {
                derived.extend(claim_has_no_independent_support(&index)?)
            }
            "object_in_unresolved_tension_neighborhood" => {
                derived.extend(object_in_unresolved_tension_neighborhood(&index)?)
            }
            "code_symbol_touched_by_failing_postmortem_pattern" => {
                derived.extend(code_symbol_touched_by_failing_postmortem_pattern(&index)?)
            }
            "context_atom_tainted_by_generated_artifact" => {
                derived.extend(context_atom_tainted_by_generated_artifact(&index)?)
            }
            "private_source_reaches_export_candidate" => {
                derived.extend(private_source_reaches_export_candidate(&index)?)
            }
            "demolition_window" => derived.extend(demolition_window(&index)?),
            "conflict_set" => derived.extend(conflict_set(&index)?),
            "vacancy_duration" => derived.extend(vacancy_duration(&index, 5)?),
            "ownership_chain" => derived.extend(ownership_chain(&index)?),
            unknown => warnings.push(format!("Unknown Datalog rule skipped: {unknown}")),
        }
    }

    let mut deduped: BTreeMap<String, Value> = BTreeMap::new();
    for fact in derived {
        let fact_id = object_field(&fact, "fact_id").to_string();
        deduped.insert(fact_id, fact);
    }
    let mut derived_facts: Vec<Value> = deduped.into_values().collect();
    derived_facts.sort_by(|left, right| {
        (
            object_field(left, "rule_id"),
            object_field(left, "subject_id"),
            object_field(left, "fact_id"),
        )
            .cmp(&(
                object_field(right, "rule_id"),
                object_field(right, "subject_id"),
                object_field(right, "fact_id"),
            ))
    });

    Ok(json!({
        "engine": "python-reference-datalog",
        "fact_pack_hash": fact_pack_hash_for_facts(&facts)?,
        "rule_ids": selected_rule_ids,
        "derived_count": derived_facts.len(),
        "derived_facts": derived_facts,
        "warnings": warnings,
        "writeback_policy": "read-only",
    }))
}

pub fn probabilistic_source_reliability_from_json(payload_json: &str) -> Result<Value, String> {
    let payload = parse_json(payload_json)?;
    probabilistic_source_reliability(&payload)
}

pub fn probabilistic_source_reliability(payload: &Value) -> Result<Value, String> {
    let source_id = arg_string(payload, "source_id", "source");
    let prior_alpha = numeric_arg(payload, "prior_alpha", 1.0);
    let prior_beta = numeric_arg(payload, "prior_beta", 1.0);
    let corroborated = integer_arg(payload, "corroborated", 0).max(0) as f64;
    let contradicted = integer_arg(payload, "contradicted", 0).max(0) as f64;
    let alpha = prior_alpha + corroborated;
    let beta = prior_beta + contradicted;
    let total = alpha + beta;
    let mean = if total != 0.0 { alpha / total } else { 0.5 };
    let variance = if total > 0.0 {
        (alpha * beta) / ((total * total) * (total + 1.0))
    } else {
        0.0
    };
    posterior_receipt(
        "source-reliability",
        &source_id,
        json!({"alpha": prior_alpha, "beta": prior_beta}),
        json!({"corroborated": integer_arg(payload, "corroborated", 0), "contradicted": integer_arg(payload, "contradicted", 0)}),
        json!({"alpha": alpha, "beta": beta, "mean": mean, "variance": variance}),
        json!({"source_id": source_id, "distribution": "beta"}),
    )
}

pub fn probabilistic_expected_value_from_json(payload_json: &str) -> Result<Value, String> {
    let payload = parse_json(payload_json)?;
    probabilistic_expected_value(&payload)
}

pub fn probabilistic_expected_value(payload: &Value) -> Result<Value, String> {
    let current_uncertainty = numeric_arg(payload, "current_uncertainty", 0.0);
    let expected_uncertainty_after = numeric_arg(payload, "expected_uncertainty_after", 0.0);
    let decision_value = numeric_arg(payload, "decision_value", 1.0);
    let validator_cost = numeric_arg(payload, "validator_cost", 0.0);
    let uncertainty_reduction = (current_uncertainty - expected_uncertainty_after).max(0.0);
    let expected_value = (uncertainty_reduction * decision_value) - validator_cost;
    posterior_receipt(
        "expected-value-of-information",
        "",
        json!({"current_uncertainty": current_uncertainty}),
        json!({
            "expected_uncertainty_after": expected_uncertainty_after,
            "validator_cost": validator_cost,
        }),
        json!({
            "expected_value": expected_value,
            "uncertainty_reduction": uncertainty_reduction,
        }),
        json!({"decision_value": decision_value}),
    )
}

pub fn evolution_archive_from_json(payload_json: &str) -> Result<Value, String> {
    let payload = parse_json(payload_json)?;
    evolution_archive(payload)
}

pub fn evolution_archive(payload: Value) -> Result<Value, String> {
    let candidates = evolution_candidates_from_payload(&payload)?;
    let elites_per_niche = integer_arg(&payload, "elites_per_niche", 2);
    let mut niches: BTreeMap<String, Vec<EvolutionCandidateRecord>> = BTreeMap::new();
    for candidate in candidates {
        niches
            .entry(candidate.niche.clone())
            .or_default()
            .push(candidate);
    }

    let mut elites_by_niche: Map<String, Value> = Map::new();
    let mut selected: Vec<EvolutionCandidateRecord> = Vec::new();
    let mut candidate_count = 0usize;
    for (niche, mut niche_candidates) in niches {
        candidate_count += niche_candidates.len();
        niche_candidates.sort_by(compare_evolution_elite_rank);
        let take_count = python_slice_stop_len(niche_candidates.len(), elites_per_niche);
        let ranked = niche_candidates
            .into_iter()
            .take(take_count)
            .collect::<Vec<_>>();
        selected.extend(ranked.clone());
        elites_by_niche.insert(
            niche,
            Value::Array(
                ranked
                    .iter()
                    .map(EvolutionCandidateRecord::to_value)
                    .collect(),
            ),
        );
    }

    let archive_hash = evolution_archive_hash(&selected)?;
    let rejected_count = candidate_count.saturating_sub(selected.len());
    Ok(json!({
        "engine": "quality-diversity-python-fallback",
        "archive_hash": archive_hash,
        "elites_by_niche": Value::Object(elites_by_niche),
        "rejected_count": rejected_count,
        "writeback_policy": "read-only",
    }))
}

fn posterior_receipt(
    kind: &str,
    source_id: &str,
    prior: Value,
    observations: Value,
    posterior: Value,
    metadata: Value,
) -> Result<Value, String> {
    let model_id = if kind == "source-reliability" {
        format!("source-reliability:{source_id}")
    } else {
        kind.to_string()
    };
    let engine = "beta-binomial-python-fallback";
    let hash_payload = json!({
        "engine": engine,
        "model_id": model_id,
        "prior": prior,
        "observations": observations,
        "posterior": posterior,
        "metadata": metadata,
    });
    let receipt_hash = stable_hash_value(&hash_payload)?;
    Ok(json!({
        "engine": engine,
        "model_id": hash_payload["model_id"].clone(),
        "prior": hash_payload["prior"].clone(),
        "observations": hash_payload["observations"].clone(),
        "posterior": hash_payload["posterior"].clone(),
        "metadata": hash_payload["metadata"].clone(),
        "receipt_hash": receipt_hash,
        "writeback_policy": "read-only",
    }))
}

fn unsupported_claim(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let (support_evidence, dependencies) = claim_support_indexes(index);
    let mut out = Vec::new();
    for claim in relation(index, "claim") {
        let status = attr_string(claim, "status").to_lowercase();
        if matches!(status.as_str(), "archived" | "refuted" | "superseded") {
            continue;
        }
        let claim_id = object_field(claim, "entity_id");
        if !support_evidence.contains_key(claim_id) && !dependencies.contains_key(claim_id) {
            out.push(derived_fact(
                "unsupported_claim",
                "unsupported_claim",
                claim_id,
                "This claim has no supporting EvidenceLink or ClaimDependency in the current fact pack.",
                vec![object_field(claim, "fact_id").to_string()],
                json!({"status": attr_value_or(claim, "status", json!(""))}),
                1.0,
                "read-only",
            )?);
        }
    }
    Ok(out)
}

fn dependent_claim(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for dep in relation(index, "claim_dependency") {
        let claim_id = attr_string(dep, "claim_id");
        if claim_id.is_empty() {
            continue;
        }
        out.push(derived_fact(
            "dependent_claim",
            "dependent_claim",
            &claim_id,
            "This claim depends on another graph object for its justification.",
            vec![object_field(dep, "fact_id").to_string()],
            json!({
                "depends_on_object_id": attr_value_or(dep, "depends_on_object_id", json!("")),
                "justification_type": attr_value_or(dep, "justification_type", json!("")),
                "strength": attr_value_or(dep, "strength", json!(0.0)),
            }),
            1.0,
            "read-only",
        )?);
    }
    Ok(out)
}

fn source_reused_support(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let mut by_source: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for dep in relation(index, "claim_dependency") {
        let source_id = attr_string(dep, "depends_on_object_id");
        if !source_id.is_empty() {
            by_source
                .entry(format!("object:{source_id}"))
                .or_default()
                .push(dep.clone());
        }
    }
    for link in relation(index, "evidence_link") {
        let artifact_id = attr_string(link, "artifact_id");
        if !artifact_id.is_empty() {
            by_source
                .entry(format!("artifact:{artifact_id}"))
                .or_default()
                .push(link.clone());
        }
    }

    let mut out = Vec::new();
    for (source_id, facts) in by_source {
        let claim_ids: BTreeSet<String> = facts
            .iter()
            .map(|fact| attr_string(fact, "claim_id"))
            .filter(|claim_id| !claim_id.is_empty())
            .collect();
        if claim_ids.len() < 2 {
            continue;
        }
        out.push(derived_fact(
            "source_reused_support",
            "source_reused_support",
            &source_id,
            "The same source is reused as support for multiple claims.",
            facts
                .iter()
                .map(|fact| object_field(fact, "fact_id").to_string())
                .collect(),
            json!({"claim_ids": claim_ids.into_iter().collect::<Vec<_>>()}),
            0.8,
            "read-only",
        )?);
    }
    Ok(out)
}

fn likely_duplicate_entity(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let mut by_title: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for object in relation(index, "object") {
        let title = normalize_title(&attr_string(object, "title"));
        if !title.is_empty() {
            by_title.entry(title).or_default().push(object.clone());
        }
    }

    let mut out = Vec::new();
    for (title, objects) in by_title {
        if objects.len() < 2 {
            continue;
        }
        out.push(derived_fact(
            "likely_duplicate_entity",
            "likely_duplicate_entity",
            object_field(&objects[0], "entity_id"),
            "Multiple graph objects share the same normalized title.",
            objects
                .iter()
                .map(|object| object_field(object, "fact_id").to_string())
                .collect(),
            json!({
                "normalized_title": title,
                "duplicate_object_ids": objects[1..]
                    .iter()
                    .map(|object| object_field(object, "entity_id").to_string())
                    .collect::<Vec<_>>(),
            }),
            0.7,
            "proposal-only",
        )?);
    }
    Ok(out)
}

fn evidence_path_too_long(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let max_length = 3_i64;
    for path in relation(index, "evidence_path") {
        let path_length = integer_attr(path, "path_length", 0);
        if path_length <= max_length {
            continue;
        }
        out.push(derived_fact(
            "evidence_path_too_long",
            "evidence_path_too_long",
            object_field(path, "entity_id"),
            "The evidence path exceeds the configured symbolic derivation depth.",
            vec![object_field(path, "fact_id").to_string()],
            json!({"path_length": path_length, "max_length": max_length}),
            0.9,
            "read-only",
        )?);
    }
    Ok(out)
}

fn claim_has_no_independent_support(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let (support_evidence, dependencies) = claim_support_indexes(index);
    let mut out = Vec::new();
    for claim in relation(index, "claim") {
        let claim_id = object_field(claim, "entity_id");
        let mut source_refs: BTreeSet<String> = BTreeSet::new();
        for dep in dependencies.get(claim_id).into_iter().flatten() {
            let source_id = attr_string(dep, "depends_on_object_id");
            if !source_id.is_empty() {
                source_refs.insert(format!("object:{source_id}"));
            }
        }
        for link in support_evidence.get(claim_id).into_iter().flatten() {
            let artifact_id = attr_string(link, "artifact_id");
            if !artifact_id.is_empty() {
                source_refs.insert(format!("artifact:{artifact_id}"));
            }
        }
        if source_refs.len() >= 2 {
            continue;
        }
        let mut dependency_ids = vec![object_field(claim, "fact_id").to_string()];
        dependency_ids.extend(
            dependencies
                .get(claim_id)
                .into_iter()
                .flatten()
                .map(|fact| object_field(fact, "fact_id").to_string()),
        );
        dependency_ids.extend(
            support_evidence
                .get(claim_id)
                .into_iter()
                .flatten()
                .map(|fact| object_field(fact, "fact_id").to_string()),
        );
        let support_sources: Vec<String> = source_refs.into_iter().collect();
        out.push(derived_fact(
            "claim_has_no_independent_support",
            "claim_has_no_independent_support",
            claim_id,
            "This claim does not have two independent support sources in the current fact pack.",
            dependency_ids,
            json!({
                "support_source_count": support_sources.len(),
                "support_sources": support_sources,
            }),
            0.85,
            "read-only",
        )?);
    }
    Ok(out)
}

fn object_in_unresolved_tension_neighborhood(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for edge in relation(index, "edge") {
        let edge_type = attr_string(edge, "edge_type").to_lowercase();
        let status = attr_string(edge, "acceptance_status").to_lowercase();
        if edge_type != "contradicts" && status != "contested" {
            continue;
        }
        for object_id in [
            attr_string(edge, "from_object_id"),
            attr_string(edge, "to_object_id"),
        ] {
            if object_id.is_empty() {
                continue;
            }
            out.push(derived_fact(
                "object_in_unresolved_tension_neighborhood",
                "object_in_unresolved_tension_neighborhood",
                &object_id,
                "This object is adjacent to a contradicting or contested edge.",
                vec![object_field(edge, "fact_id").to_string()],
                json!({
                    "edge_id": object_field(edge, "entity_id"),
                    "edge_type": edge_type,
                    "acceptance_status": status,
                }),
                0.8,
                "read-only",
            )?);
        }
    }
    Ok(out)
}

fn code_symbol_touched_by_failing_postmortem_pattern(
    index: &RelationIndex,
) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for atom in relation(index, "context_atom") {
        let metadata = attr_object(atom, "metadata");
        if attr_string(atom, "kind") != "code_symbol" {
            continue;
        }
        if ![
            "failing_postmortem_pattern",
            "postmortem_failure",
            "failed_tests",
        ]
        .iter()
        .any(|key| metadata.get(*key).is_some_and(truthy))
        {
            continue;
        }
        out.push(derived_fact(
            "code_symbol_touched_by_failing_postmortem_pattern",
            "code_symbol_touched_by_failing_postmortem_pattern",
            object_field(atom, "entity_id"),
            "This code symbol is linked to a failing postmortem or failed-test pattern.",
            vec![object_field(atom, "fact_id").to_string()],
            json!({"title": attr_value_or(atom, "title", json!(""))}),
            0.85,
            "read-only",
        )?);
    }
    Ok(out)
}

fn context_atom_tainted_by_generated_artifact(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for atom in relation(index, "context_atom") {
        let metadata = attr_object(atom, "metadata");
        let generated = metadata.get("generated").is_some_and(truthy)
            || object_value_lower(metadata.get("source_kind")) == "generated_artifact"
            || object_value_lower(metadata.get("provenance")) == "generated";
        if !generated {
            continue;
        }
        out.push(derived_fact(
            "context_atom_tainted_by_generated_artifact",
            "context_atom_tainted_by_generated_artifact",
            object_field(atom, "entity_id"),
            "This context atom was derived from generated material and should not be treated as independent evidence.",
            vec![object_field(atom, "fact_id").to_string()],
            json!({"artifact_id": attr_value_or(atom, "artifact_id", json!(""))}),
            0.9,
            "read-only",
        )?);
    }
    Ok(out)
}

fn private_source_reaches_export_candidate(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let objects = facts_by_id(relation(index, "object"));
    let mut out = Vec::new();
    for atom in relation(index, "context_atom") {
        let metadata = attr_object(atom, "metadata");
        let object_id = attr_string_or_empty_if_falsey(atom, "object_pk");
        let properties = objects
            .get(&object_id)
            .map(|object| attr_object(object, "properties"))
            .unwrap_or_default();
        let is_private = metadata.get("private").is_some_and(truthy)
            || object_value_lower(metadata.get("source_visibility")) == "private"
            || properties.get("private").is_some_and(truthy)
            || object_value_lower(properties.get("visibility")) == "private";
        let is_export_candidate = metadata.get("export_candidate").is_some_and(truthy)
            || metadata.get("public_export").is_some_and(truthy)
            || object_value_lower(metadata.get("export_visibility")) == "public";
        if !(is_private && is_export_candidate) {
            continue;
        }
        let mut dependency_ids = vec![object_field(atom, "fact_id").to_string()];
        if let Some(object) = objects.get(&object_id) {
            dependency_ids.push(object_field(object, "fact_id").to_string());
        }
        out.push(derived_fact(
            "private_source_reaches_export_candidate",
            "private_source_reaches_export_candidate",
            object_field(atom, "entity_id"),
            "A private source is marked as a public export candidate.",
            dependency_ids,
            json!({
                "object_pk": object_id,
                "artifact_id": attr_value_or(atom, "artifact_id", json!("")),
            }),
            0.95,
            "proposal-only",
        )?);
    }
    Ok(out)
}

fn demolition_window(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let present = structure_observations_by_parcel(index, "structure_present");
    let absent = structure_observations_by_parcel(index, "structure_absent");
    let mut out = Vec::new();

    for (parcel, present_obs) in present {
        let Some(absent_obs) = absent.get(&parcel) else {
            continue;
        };
        let present_years: BTreeSet<i64> = present_obs.iter().map(|(year, _)| *year).collect();
        let absent_years: BTreeSet<i64> = absent_obs.iter().map(|(year, _)| *year).collect();
        let earliest_absent = absent_years.into_iter().find(|absent_year| {
            present_years
                .iter()
                .any(|present_year| present_year < absent_year)
        });
        let Some(earliest_absent) = earliest_absent else {
            continue;
        };
        let Some(latest_present) = present_years
            .iter()
            .filter(|present_year| **present_year < earliest_absent)
            .max()
            .copied()
        else {
            continue;
        };

        let present_facts: Vec<Value> = present_obs
            .iter()
            .filter(|(year, _)| *year == latest_present)
            .map(|(_, fact)| fact.clone())
            .collect();
        let absent_facts: Vec<Value> = absent_obs
            .iter()
            .filter(|(year, _)| *year == earliest_absent)
            .map(|(_, fact)| fact.clone())
            .collect();
        let present_sources = source_ids(&present_facts);
        let absent_sources = source_ids(&absent_facts);
        let present_label = if present_sources.is_empty() {
            "an earlier survey".to_string()
        } else {
            present_sources.join(", ")
        };
        let absent_label = if absent_sources.is_empty() {
            "a later survey".to_string()
        } else {
            absent_sources.join(", ")
        };

        let mut dependency_ids: Vec<String> = present_facts
            .iter()
            .map(|fact| object_field(fact, "fact_id").to_string())
            .collect();
        dependency_ids.extend(
            absent_facts
                .iter()
                .map(|fact| object_field(fact, "fact_id").to_string()),
        );
        out.push(derived_fact(
            "demolition_window",
            "demolition_between",
            &parcel,
            &format!(
                "A structure stood at parcel {parcel} in {latest_present} \
                 (per {present_label}) and was absent by {earliest_absent} \
                 (per {absent_label}); demolition occurred between \
                 {latest_present} and {earliest_absent}."
            ),
            dependency_ids,
            json!({
                "parcel_id": parcel,
                "earliest_year": latest_present,
                "latest_year": earliest_absent,
                "window_years": earliest_absent - latest_present,
                "present_source_ids": present_sources,
                "absent_source_ids": absent_sources,
            }),
            1.0,
            "read-only",
        )?);
    }

    Ok(out)
}

fn conflict_set(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let mut grouped: BTreeMap<(String, String), Vec<Value>> = BTreeMap::new();
    for fact in relation(index, "source_assertion") {
        let parcel = attr_string_or_entity_if_falsey(fact, "parcel_id")
            .trim()
            .to_string();
        let field = attr_string(fact, "field").trim().to_string();
        if parcel.is_empty() || field.is_empty() {
            continue;
        }
        grouped
            .entry((parcel, field))
            .or_default()
            .push(fact.clone());
    }

    let mut out = Vec::new();
    for ((parcel, field), assertions) in grouped {
        let mut values_to_sources: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for fact in &assertions {
            let value = attr_string(fact, "value").trim().to_string();
            if value.is_empty() {
                continue;
            }
            let source = attr_string(fact, "source_id");
            let source = if source.is_empty() {
                object_field(fact, "entity_id").to_string()
            } else {
                source
            };
            values_to_sources.entry(value).or_default().insert(source);
        }
        if values_to_sources.len() < 2 {
            continue;
        }

        let rendered = values_to_sources
            .iter()
            .map(|(value, sources)| {
                format!(
                    "{} (per {})",
                    value,
                    sources.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let dependency_ids = sorted_fact_ids(&assertions);
        let source_ids = assertions
            .iter()
            .map(|fact| {
                let source = attr_string(fact, "source_id");
                if source.is_empty() {
                    object_field(fact, "entity_id").to_string()
                } else {
                    source
                }
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let values = values_to_sources
            .iter()
            .map(|(value, sources)| {
                (
                    value.clone(),
                    Value::Array(sources.iter().cloned().map(Value::String).collect()),
                )
            })
            .collect::<Map<_, _>>();

        out.push(derived_fact(
            "conflict_set",
            "source_conflict",
            &format!("{parcel}:{field}"),
            &format!("Sources disagree on {field} for parcel {parcel}: {rendered}."),
            dependency_ids,
            json!({
                "parcel_id": parcel,
                "field": field,
                "values": values,
                "source_ids": source_ids,
                "distinct_value_count": values_to_sources.len(),
            }),
            1.0,
            "read-only",
        )?);
    }
    Ok(out)
}

fn vacancy_duration(index: &RelationIndex, min_years: i64) -> Result<Vec<Value>, String> {
    let mut by_parcel: BTreeMap<String, Vec<(i64, String, Value)>> = BTreeMap::new();
    for fact in relation(index, "assessor_status") {
        let parcel = attr_string_or_entity_if_falsey(fact, "parcel_id")
            .trim()
            .to_string();
        let Some(year) = attr(fact, "year").and_then(coerce_year) else {
            continue;
        };
        let status = attr_string(fact, "status").trim().to_lowercase();
        if parcel.is_empty() || status.is_empty() {
            continue;
        }
        by_parcel
            .entry(parcel)
            .or_default()
            .push((year, status, fact.clone()));
    }

    let mut out = Vec::new();
    for (parcel, mut observations) in by_parcel {
        observations.sort_by(|left, right| {
            (left.0, object_field(&left.2, "fact_id"))
                .cmp(&(right.0, object_field(&right.2, "fact_id")))
        });

        let mut best_run: Vec<(i64, String, Value)> = Vec::new();
        let mut current: Vec<(i64, String, Value)> = Vec::new();
        for observation in observations {
            if observation.1 == "vacant" {
                current.push(observation);
            } else if observation.1 == "occupied" {
                if current.len() > best_run.len() {
                    best_run = current;
                }
                current = Vec::new();
            }
        }
        if current.len() > best_run.len() {
            best_run = current;
        }
        if best_run.is_empty() {
            continue;
        }

        let earliest = best_run.first().map(|item| item.0).unwrap_or_default();
        let latest = best_run.last().map(|item| item.0).unwrap_or_default();
        let span = latest - earliest;
        if span < min_years {
            continue;
        }
        let run_facts = best_run
            .iter()
            .map(|(_, _, fact)| fact.clone())
            .collect::<Vec<_>>();
        let source_ids = source_ids(&run_facts);

        out.push(derived_fact(
            "vacancy_duration",
            "prolonged_vacancy",
            &parcel,
            &format!(
                "Parcel {parcel} was assessed vacant from {earliest} to {latest} \
                 ({span} years) with no intervening occupied record."
            ),
            sorted_fact_ids(&run_facts),
            json!({
                "parcel_id": parcel,
                "earliest_year": earliest,
                "latest_year": latest,
                "vacant_years": span,
                "min_years": min_years,
                "observation_count": run_facts.len(),
                "source_ids": source_ids,
            }),
            1.0,
            "read-only",
        )?);
    }
    Ok(out)
}

fn ownership_chain(index: &RelationIndex) -> Result<Vec<Value>, String> {
    let mut by_parcel: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for fact in relation(index, "ownership") {
        let parcel = attr_string_or_entity_if_falsey(fact, "parcel_id")
            .trim()
            .to_string();
        if parcel.is_empty() {
            continue;
        }
        by_parcel.entry(parcel).or_default().push(fact.clone());
    }

    let mut out = Vec::new();
    for (parcel, records) in by_parcel {
        if records.len() < 2 {
            continue;
        }

        let mut ordered = records.clone();
        ordered.sort_by(|left, right| {
            let left_year = attr(left, "from_year").and_then(coerce_year);
            let right_year = attr(right, "from_year").and_then(coerce_year);
            (
                left_year.is_none(),
                left_year.unwrap_or_default(),
                object_field(left, "fact_id"),
            )
                .cmp(&(
                    right_year.is_none(),
                    right_year.unwrap_or_default(),
                    object_field(right, "fact_id"),
                ))
        });

        let mut chain: Vec<Value> = Vec::new();
        for record in &ordered {
            let owner_type = attr_string(record, "owner_type").trim().to_lowercase();
            chain.push(json!({
                "owner": attr_string(record, "owner"),
                "owner_type": owner_type,
                "from_year": optional_year_value(record, "from_year"),
                "to_year": optional_year_value(record, "to_year"),
                "source_id": attr_string(record, "source_id"),
            }));
        }

        let mut foreclosure_year: Option<i64> = None;
        let mut land_bank_year: Option<i64> = None;
        for link in &chain {
            let owner_type = object_field(link, "owner_type");
            if owner_type == "tax_foreclosure" && foreclosure_year.is_none() {
                foreclosure_year = link.get("from_year").and_then(coerce_year);
            } else if owner_type == "land_bank"
                && foreclosure_year.is_some()
                && land_bank_year.is_none()
            {
                land_bank_year = link.get("from_year").and_then(coerce_year);
            }
        }
        let distressed = foreclosure_year.is_some() && land_bank_year.is_some();
        let owners_label = chain
            .iter()
            .map(|link| {
                let label = [
                    object_field(link, "owner"),
                    object_field(link, "owner_type"),
                    "unknown",
                ]
                .into_iter()
                .find(|value| !value.is_empty())
                .unwrap_or("unknown");
                match link.get("from_year").and_then(coerce_year) {
                    Some(year) => format!("{label} ({year})"),
                    None => label.to_string(),
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut reason = format!(
            "Parcel {parcel} passed through {} owners: {owners_label}.",
            chain.len()
        );
        if distressed {
            reason.push_str(&format!(
                " Acquired by the land bank via tax foreclosure ({} to {}).",
                foreclosure_year.unwrap_or_default(),
                land_bank_year.unwrap_or_default()
            ));
        }

        out.push(derived_fact(
            "ownership_chain",
            "ownership_chain",
            &parcel,
            &reason,
            sorted_fact_ids(&records),
            json!({
                "parcel_id": parcel,
                "owner_count": chain.len(),
                "chain": chain,
                "tax_foreclosure_to_land_bank": distressed,
                "foreclosure_year": foreclosure_year,
                "land_bank_year": land_bank_year,
            }),
            1.0,
            "read-only",
        )?);
    }
    Ok(out)
}

#[derive(Clone)]
struct EvolutionCandidateRecord {
    candidate_id: String,
    niche: String,
    score: f64,
    novelty: f64,
    payload: Value,
}

impl EvolutionCandidateRecord {
    fn to_value(&self) -> Value {
        json!({
            "candidate_id": self.candidate_id,
            "niche": self.niche,
            "score": self.score,
            "novelty": self.novelty,
            "payload": self.payload.clone(),
        })
    }
}

fn evolution_candidates_from_payload(
    payload: &Value,
) -> Result<Vec<EvolutionCandidateRecord>, String> {
    let candidates = payload
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| "evolution archive payload expected candidates array".to_string())?;
    candidates
        .iter()
        .map(evolution_candidate_from_value)
        .collect()
}

fn evolution_candidate_from_value(value: &Value) -> Result<EvolutionCandidateRecord, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "evolution candidate expected JSON object".to_string())?;
    let candidate_id = object
        .get("candidate_id")
        .map(value_to_string)
        .unwrap_or_default();
    let niche = object
        .get("niche")
        .map(value_to_string)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string());
    let payload = object
        .get("payload")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    Ok(EvolutionCandidateRecord {
        candidate_id,
        niche,
        score: object.get("score").and_then(value_to_f64).unwrap_or(0.0),
        novelty: object.get("novelty").and_then(value_to_f64).unwrap_or(0.0),
        payload,
    })
}

fn compare_evolution_elite_rank(
    left: &EvolutionCandidateRecord,
    right: &EvolutionCandidateRecord,
) -> Ordering {
    compare_f64_desc(left.score, right.score)
        .then_with(|| compare_f64_desc(left.novelty, right.novelty))
        .then_with(|| right.candidate_id.cmp(&left.candidate_id))
}

fn compare_evolution_hash_rank(
    left: &EvolutionCandidateRecord,
    right: &EvolutionCandidateRecord,
) -> Ordering {
    left.niche
        .cmp(&right.niche)
        .then_with(|| compare_f64_desc(left.score, right.score))
        .then_with(|| left.candidate_id.cmp(&right.candidate_id))
}

fn compare_f64_desc(left: f64, right: f64) -> Ordering {
    right.partial_cmp(&left).unwrap_or(Ordering::Equal)
}

fn python_slice_stop_len(len: usize, stop: i64) -> usize {
    if stop >= 0 {
        return len.min(stop as usize);
    }
    len.saturating_sub(stop.unsigned_abs() as usize)
}

fn evolution_archive_hash(candidates: &[EvolutionCandidateRecord]) -> Result<String, String> {
    let mut sorted = candidates.to_vec();
    sorted.sort_by(compare_evolution_hash_rank);
    stable_hash_value(&Value::Array(
        sorted
            .iter()
            .map(EvolutionCandidateRecord::to_value)
            .collect(),
    ))
}

type RelationIndex = BTreeMap<String, Vec<Value>>;

fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|err| format!("expected valid JSON: {err}"))
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn datalog_payload_parts(payload: &Value) -> Result<(&Vec<Value>, Option<Vec<String>>), String> {
    if let Some(facts) = payload.as_array() {
        return Ok((facts, None));
    }
    let facts = payload
        .get("facts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "datalog payload expected a JSON array or an object with facts array".to_string()
        })?;
    let rule_ids = payload
        .get("rule_ids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
    Ok((facts, rule_ids))
}

fn selected_rule_ids(requested_rule_ids: Option<Vec<String>>) -> Vec<String> {
    match requested_rule_ids {
        Some(ids) if !ids.is_empty() => ids,
        _ => DATALOG_RULE_IDS
            .iter()
            .map(|rule_id| (*rule_id).to_string())
            .collect(),
    }
}

fn normalize_facts(raw_facts: &[Value]) -> Result<Vec<Value>, String> {
    raw_facts.iter().map(normalize_fact).collect()
}

fn normalize_fact(raw: &Value) -> Result<Value, String> {
    let relation = field_string(raw, "relation");
    let entity_id = field_string(raw, "entity_id");
    if relation.trim().is_empty() {
        return Err("DatalogFact requires relation".to_string());
    }
    if entity_id.trim().is_empty() {
        return Err("DatalogFact requires entity_id".to_string());
    }
    let attributes = raw
        .get("attributes")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let source_ref = field_string(raw, "source_ref");
    let fallback_id = stable_hash_value(&json!({
        "relation": relation,
        "entity_id": entity_id,
        "attributes": attributes,
        "source_ref": source_ref,
    }))?;
    let fact_id = field_string(raw, "fact_id");
    Ok(json!({
        "fact_id": if fact_id.is_empty() { fallback_id } else { fact_id },
        "relation": field_string(raw, "relation"),
        "entity_id": field_string(raw, "entity_id"),
        "attributes": raw
            .get("attributes")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({})),
        "source_ref": field_string(raw, "source_ref"),
    }))
}

fn fact_pack_hash_for_facts(facts: &[Value]) -> Result<String, String> {
    let mut sorted = facts.to_vec();
    sorted.sort_by(|left, right| object_field(left, "fact_id").cmp(object_field(right, "fact_id")));
    stable_hash_value(&Value::Array(sorted))
}

fn facts_by_relation(facts: &[Value]) -> RelationIndex {
    let mut index: RelationIndex = BTreeMap::new();
    for fact in facts {
        index
            .entry(object_field(fact, "relation").to_string())
            .or_default()
            .push(fact.clone());
    }
    for facts in index.values_mut() {
        facts.sort_by(|left, right| {
            object_field(left, "fact_id").cmp(object_field(right, "fact_id"))
        });
    }
    index
}

fn claim_support_indexes(
    index: &RelationIndex,
) -> (BTreeMap<String, Vec<Value>>, BTreeMap<String, Vec<Value>>) {
    let mut support_evidence: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut dependencies: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for link in relation(index, "evidence_link") {
        let relation_type = attr_string(link, "relation_type").to_lowercase();
        if matches!(
            relation_type.as_str(),
            "supports" | "derived_from" | "cites" | "references"
        ) {
            support_evidence
                .entry(attr_string(link, "claim_id"))
                .or_default()
                .push(link.clone());
        }
    }
    for dep in relation(index, "claim_dependency") {
        dependencies
            .entry(attr_string(dep, "claim_id"))
            .or_default()
            .push(dep.clone());
    }
    (support_evidence, dependencies)
}

fn facts_by_id(facts: &[Value]) -> BTreeMap<String, Value> {
    facts
        .iter()
        .map(|fact| (object_field(fact, "entity_id").to_string(), fact.clone()))
        .collect()
}

fn structure_observations_by_parcel(
    index: &RelationIndex,
    relation_name: &str,
) -> BTreeMap<String, Vec<(i64, Value)>> {
    let mut grouped: BTreeMap<String, Vec<(i64, Value)>> = BTreeMap::new();
    for fact in relation(index, relation_name) {
        let parcel = attr_string_or_entity_if_falsey(fact, "parcel_id")
            .trim()
            .to_string();
        let Some(year) = attr(fact, "year").and_then(coerce_year) else {
            continue;
        };
        if !parcel.is_empty() {
            grouped
                .entry(parcel)
                .or_default()
                .push((year, fact.clone()));
        }
    }
    grouped
}

fn source_ids(facts: &[Value]) -> Vec<String> {
    facts
        .iter()
        .filter_map(|fact| attr(fact, "source_id"))
        .filter(|value| python_truthy(value))
        .map(value_to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sorted_fact_ids(facts: &[Value]) -> Vec<String> {
    facts
        .iter()
        .map(|fact| object_field(fact, "fact_id").to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn optional_year_value(fact: &Value, key: &str) -> Value {
    attr(fact, key)
        .and_then(coerce_year)
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn relation<'a>(index: &'a RelationIndex, relation: &str) -> &'a [Value] {
    index.get(relation).map(Vec::as_slice).unwrap_or(&[])
}

fn derived_fact(
    rule_id: &str,
    relation: &str,
    subject_id: &str,
    reason: &str,
    dependency_fact_ids: Vec<String>,
    attributes: Value,
    confidence: f64,
    writeback_policy: &str,
) -> Result<Value, String> {
    let hash_payload = json!({
        "rule_id": rule_id,
        "relation": relation,
        "subject_id": subject_id,
        "attributes": attributes,
        "dependency_fact_ids": dependency_fact_ids,
    });
    let fact_id = stable_hash_value(&hash_payload)?;
    Ok(json!({
        "fact_id": fact_id,
        "rule_id": rule_id,
        "relation": relation,
        "subject_id": subject_id,
        "reason": reason,
        "dependency_fact_ids": hash_payload["dependency_fact_ids"].clone(),
        "attributes": hash_payload["attributes"].clone(),
        "confidence": confidence,
        "writeback_policy": writeback_policy,
    }))
}

fn field_string(value: &Value, key: &str) -> String {
    value.get(key).map(value_to_string).unwrap_or_default()
}

fn object_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn attr<'a>(fact: &'a Value, key: &str) -> Option<&'a Value> {
    fact.get("attributes")
        .and_then(Value::as_object)
        .and_then(|attributes| attributes.get(key))
}

fn attr_value_or(fact: &Value, key: &str, default: Value) -> Value {
    attr(fact, key).cloned().unwrap_or(default)
}

fn attr_string(fact: &Value, key: &str) -> String {
    attr(fact, key).map(value_to_string).unwrap_or_default()
}

fn attr_string_or_empty_if_falsey(fact: &Value, key: &str) -> String {
    attr(fact, key)
        .filter(|value| python_truthy(value))
        .map(value_to_string)
        .unwrap_or_default()
}

fn attr_string_or_entity_if_falsey(fact: &Value, key: &str) -> String {
    attr(fact, key)
        .filter(|value| python_truthy(value))
        .map(value_to_string)
        .unwrap_or_else(|| object_field(fact, "entity_id").to_string())
}

fn attr_object(fact: &Value, key: &str) -> Map<String, Value> {
    attr(fact, key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn integer_attr(fact: &Value, key: &str, default: i64) -> i64 {
    attr(fact, key).map(value_to_i64).unwrap_or(default)
}

fn numeric_arg(payload: &Value, key: &str, default: f64) -> f64 {
    payload.get(key).and_then(value_to_f64).unwrap_or(default)
}

fn integer_arg(payload: &Value, key: &str, default: i64) -> i64 {
    payload.get(key).map(value_to_i64).unwrap_or(default)
}

fn arg_string(payload: &Value, key: &str, default: &str) -> String {
    payload
        .get(key)
        .map(value_to_string)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

fn value_to_f64(value: &Value) -> Option<f64> {
    if let Some(value) = value.as_f64() {
        return Some(value);
    }
    value.as_str().and_then(|value| value.parse::<f64>().ok())
}

fn value_to_i64(value: &Value) -> i64 {
    if let Some(value) = value.as_i64() {
        return value;
    }
    if let Some(value) = value.as_u64() {
        return i64::try_from(value).unwrap_or(i64::MAX);
    }
    if let Some(value) = value.as_f64() {
        return value as i64;
    }
    value
        .as_str()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
}

fn coerce_year(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(_) | Value::Null => None,
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                return Some(value);
            }
            if let Some(value) = number.as_u64() {
                return i64::try_from(value).ok();
            }
            number.as_f64().map(|value| value as i64)
        }
        Value::String(value) => first_four_digit_year(value),
        other => first_four_digit_year(&value_to_string(other)),
    }
}

fn first_four_digit_year(value: &str) -> Option<i64> {
    let chars: Vec<char> = value.chars().collect();
    for window in chars.windows(4) {
        if window.iter().all(|ch| ch.is_ascii_digit()) {
            let year: String = window.iter().collect();
            return year.parse::<i64>().ok();
        }
    }
    None
}

fn object_value_lower(value: Option<&Value>) -> String {
    value
        .map(value_to_string)
        .unwrap_or_default()
        .to_lowercase()
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::String(value) => matches!(
            value.to_lowercase().as_str(),
            "1" | "true" | "yes" | "private" | "public"
        ),
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Null => false,
    }
}

fn python_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::String(value) => !value.is_empty(),
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Null => false,
    }
}

fn normalize_title(value: &str) -> String {
    let mut cleaned = String::with_capacity(value.len());
    for ch in value.to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            cleaned.push(ch);
        } else {
            cleaned.push(' ');
        }
    }
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datalog_receipt_matches_reference_shape_for_all_rules() {
        let payload = json!([
            {"relation": "claim", "entity_id": "claim-1", "attributes": {"status": "proposed"}, "source_ref": "u", "fact_id": "f1"},
            {"relation": "object", "entity_id": "obj-1", "attributes": {"title": "Same", "properties": {"visibility": "private"}}, "source_ref": "u", "fact_id": "f2"},
            {"relation": "object", "entity_id": "obj-2", "attributes": {"title": "same"}, "source_ref": "u", "fact_id": "f3"},
            {"relation": "context_atom", "entity_id": "atom-1", "attributes": {"object_pk": "obj-1", "artifact_id": "ctx-1", "metadata": {"export_candidate": true}}, "source_ref": "u", "fact_id": "f4"}
        ]);

        let receipt = derive_datalog_receipt(&payload).unwrap();
        let relations: BTreeSet<String> = receipt["derived_facts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|fact| object_field(fact, "relation").to_string())
            .collect();

        assert!(relations.contains("unsupported_claim"));
        assert!(relations.contains("likely_duplicate_entity"));
        assert!(relations.contains("claim_has_no_independent_support"));
        assert!(relations.contains("private_source_reaches_export_candidate"));
        assert_eq!(receipt["engine"], "python-reference-datalog");
        assert_eq!(
            receipt["rule_ids"].as_array().unwrap().len(),
            DATALOG_RULE_IDS.len()
        );
    }

    #[test]
    fn probabilistic_receipt_hashes_are_content_addressed() {
        let receipt = probabilistic_source_reliability(&json!({
            "source_id": "source-a",
            "prior_alpha": 2.0,
            "prior_beta": 2.0,
            "corroborated": 6,
            "contradicted": 2,
        }))
        .unwrap();

        assert_eq!(receipt["posterior"]["alpha"], 8.0);
        assert_eq!(receipt["posterior"]["beta"], 4.0);
        assert!(receipt["receipt_hash"].as_str().unwrap().len() == 64);
    }

    #[test]
    fn evolution_archive_preserves_python_ranking_shape() {
        let receipt = evolution_archive(json!({
            "candidates": [
                {"candidate_id": "a", "niche": "n1", "score": 0.8, "novelty": 0.1, "payload": {"k": 1}},
                {"candidate_id": "b", "niche": "n1", "score": 0.8, "novelty": 0.9, "payload": {"k": 2}},
                {"candidate_id": "c", "niche": "n2", "score": 0.4, "novelty": 0.2, "payload": {}}
            ],
            "elites_per_niche": 1,
        }))
        .unwrap();

        assert_eq!(receipt["engine"], "quality-diversity-python-fallback");
        assert_eq!(receipt["elites_by_niche"]["n1"][0]["candidate_id"], "b");
        assert_eq!(receipt["elites_by_niche"]["n2"][0]["candidate_id"], "c");
        assert_eq!(receipt["rejected_count"], 1);
        assert!(receipt["archive_hash"].as_str().unwrap().len() == 64);
    }
}
