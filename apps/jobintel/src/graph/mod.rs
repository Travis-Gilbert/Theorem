//! Module 2 - graph write. Projects `JobRecord`s into the RustyRed graph:
//! Company / Role / Skill / Source / Person nodes plus the spec's edge types
//! (and the reverse-traversal edges PPR needs). Also designates the HNSW vector
//! index on Role.embedding + Profile.embedding, and owns `skills_of` plus the
//! `role_from_node` reverse-parser used by rank/draft.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::client::{EdgeSpec, NodeSpec, RustyRedClient};
use crate::embed::Embedder;
use crate::error::Result;
use crate::model::{edge_id, edges, labels, props, skill_id, JobRecord, Role};

/// The fixed skill vocabulary (spec Module 2). Ordering is canonical so derived
/// skill lists are stable across runs.
pub const SKILL_VOCAB: [&str; 13] = [
    "rust",
    "django",
    "rag",
    "mcp",
    "graph",
    "vector",
    "llm",
    "python",
    "gnn",
    "embedding",
    "agent",
    "retrieval",
    "infrastructure",
];

/// Extract known skills from free text. Token-exact (plural-tolerant) matching
/// against `SKILL_VOCAB`, so "vectors"/"agents" match but "storage" does not
/// match "rag". Returns canonical skill names, vocab-ordered, de-duplicated.
pub fn skills_of(body: &str) -> Vec<String> {
    let tokens: std::collections::HashSet<String> = body
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| {
            let lower = t.to_lowercase();
            // singularize a trailing plural 's' ("agents" -> "agent")
            lower.strip_suffix('s').map(String::from).unwrap_or(lower)
        })
        .collect();
    SKILL_VOCAB
        .iter()
        .filter(|skill| tokens.contains(**skill))
        .map(|s| s.to_string())
        .collect()
}

#[derive(Debug, Default, Clone)]
pub struct GraphWriteStats {
    pub companies: usize,
    pub roles: usize,
    pub skills: usize,
    pub persons: usize,
    pub sources: usize,
    pub nodes_inserted: usize,
    pub nodes_failed: usize,
    pub edges_inserted: usize,
    pub edges_failed: usize,
}

/// Build + write all nodes and edges for a batch of records. Designates the
/// vector index first so Role embeddings are indexed on insert.
pub fn upsert_records(
    client: &RustyRedClient,
    embedder: &dyn Embedder,
    records: &[JobRecord],
) -> Result<GraphWriteStats> {
    designate_vectors(client, embedder.dim());

    // Dedup by id: companies/skills/sources recur across roles; last write wins.
    let mut nodes: HashMap<String, NodeSpec> = HashMap::new();
    let mut edges: HashMap<String, EdgeSpec> = HashMap::new();
    let mut stats = GraphWriteStats::default();

    for record in records {
        // --- Company ---
        let company_id = record.company_id();
        if insert_node(
            &mut nodes,
            &company_id,
            labels::COMPANY,
            company_props(record),
        ) {
            stats.companies += 1;
        }

        // --- Source ---
        let source_id = record.source.node_id();
        if insert_node(
            &mut nodes,
            &source_id,
            labels::SOURCE,
            json!({
                props::NAME: record.source.label(),
                "kind": record.source.as_str(),
            }),
        ) {
            stats.sources += 1;
        }

        // --- Role (carries the embedding) ---
        let embed_text = format!("{}\n\n{}", record.title, record.body);
        let embedding = embedder.embed(&embed_text)?;
        insert_node(
            &mut nodes,
            &record.id,
            labels::ROLE,
            role_props(record, &embedding),
        );
        stats.roles += 1;

        // Company <-> Role (posts + reverse posted_by for traversal).
        add_edge(&mut edges, &company_id, edges::POSTS, &record.id);
        add_edge(&mut edges, &record.id, edges::POSTED_BY, &company_id);
        // Role -via-> Source.
        add_edge(&mut edges, &record.id, edges::VIA, &source_id);

        // --- Skills ---
        for skill in skills_of(&record.body) {
            let sid = skill_id(&skill);
            if insert_node(
                &mut nodes,
                &sid,
                labels::SKILL,
                json!({ props::NAME: skill }),
            ) {
                stats.skills += 1;
            }
            add_edge(&mut edges, &record.id, edges::REQUIRES, &sid);
            add_edge(&mut edges, &sid, edges::REQUIRED_BY, &record.id);
        }

        // --- Person (when there's an email or a founder signal) ---
        if record.email_present() || record.founder_posted {
            let person_id = match record.emails.first() {
                Some(email) => format!("person:{email}"),
                None => format!("person:{}:poster", record.id),
            };
            if insert_node(
                &mut nodes,
                &person_id,
                labels::PERSON,
                json!({
                    "email": record.emails.first(),
                    props::FOUNDER_POSTED: record.founder_posted,
                    props::COMPANY: record.company,
                }),
            ) {
                stats.persons += 1;
            }
            add_edge(&mut edges, &person_id, edges::HIRING_FOR, &record.id);
        }
    }

    // Bulk-write nodes then edges (edges reference node ids).
    let node_specs: Vec<NodeSpec> = nodes.into_values().collect();
    let (ni, nf) = client.bulk_nodes(&node_specs)?;
    stats.nodes_inserted = ni;
    stats.nodes_failed = nf;

    let edge_specs: Vec<EdgeSpec> = edges.into_values().collect();
    let (ei, ef) = client.bulk_edges(&edge_specs)?;
    stats.edges_inserted = ei;
    stats.edges_failed = ef;

    Ok(stats)
}

/// Upsert a Profile node (with embedding + Skill edges) and return its id. The
/// Profile -requires-> Skill edges let PPR seed on profile+skills and flow
/// outward. Idempotent: re-running overwrites the same profile id.
pub fn ensure_profile(
    client: &RustyRedClient,
    embedder: &dyn Embedder,
    handle: &str,
    profile_text: &str,
) -> Result<String> {
    designate_vectors(client, embedder.dim());
    let id = crate::model::profile_id(handle);
    let embedding = embedder.embed(profile_text)?;
    let skills = skills_of(profile_text);

    let node = NodeSpec {
        id: id.clone(),
        labels: vec![labels::PROFILE.to_string()],
        properties: json!({
            props::NAME: handle,
            "text": profile_text,
            props::EMBEDDING: embedding,
            "skills": skills,
        }),
    };
    client.upsert_node(&node)?;

    for skill in &skills {
        let sid = skill_id(skill);
        client.upsert_node(&NodeSpec {
            id: sid.clone(),
            labels: vec![labels::SKILL.to_string()],
            properties: json!({ props::NAME: skill }),
        })?;
        // Profile -requires-> Skill (same type as Role -requires-> Skill).
        client.upsert_edge(&EdgeSpec {
            id: edge_id(&id, edges::REQUIRES, &sid),
            from_id: id.clone(),
            to_id: sid.clone(),
            edge_type: edges::REQUIRES.to_string(),
            properties: json!({}),
        })?;
    }
    Ok(id)
}

/// Reverse of `role_props`: reconstruct a `Role` view from a graph node Value
/// (`{id, labels, properties}`) as returned by nodes/query. Returns None for
/// non-Role or malformed nodes.
pub fn role_from_node(node: &Value) -> Option<Role> {
    let id = node.get("id").and_then(Value::as_str)?.to_string();
    let p = node.get("properties")?;
    let get_str = |k: &str| {
        p.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let get_bool = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
    let emails = p
        .get(props::EMAILS)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Some(Role {
        id,
        company: get_str(props::COMPANY),
        company_id: get_str(props::COMPANY_ID),
        title: get_str(props::TITLE),
        location: get_str(props::LOCATION),
        url: get_str(props::URL),
        body: get_str(props::BODY),
        source: get_str(props::SOURCE),
        remote: get_bool(props::REMOTE),
        contract: get_bool(props::CONTRACT),
        founder_posted: get_bool(props::FOUNDER_POSTED),
        email_present: get_bool(props::EMAIL_PRESENT),
        emails,
        comp: p.get(props::COMP).and_then(Value::as_str).map(String::from),
        company_domain: p
            .get(props::DOMAIN)
            .and_then(Value::as_str)
            .map(String::from),
    })
}

// ---- helpers ---------------------------------------------------------------

fn designate_vectors(client: &RustyRedClient, dim: usize) {
    for label in [labels::ROLE, labels::PROFILE] {
        if let Err(err) = client.designate_vector(label, props::EMBEDDING, dim) {
            // Re-designation across runs is expected; only warn.
            eprintln!("  note: designate {label}.embedding (dim {dim}): {err}");
        }
    }
}

fn company_props(record: &JobRecord) -> Value {
    json!({
        props::NAME: record.company,
        props::DOMAIN: record.company_domain,
    })
}

fn role_props(record: &JobRecord, embedding: &[f32]) -> Value {
    json!({
        props::TITLE: record.title,
        props::COMPANY: record.company,
        props::COMPANY_ID: record.company_id(),
        props::LOCATION: record.location,
        props::URL: record.url,
        props::BODY: record.body,
        props::SOURCE: record.source.as_str(),
        props::REMOTE: record.remote,
        props::CONTRACT: record.contract,
        props::FOUNDER_POSTED: record.founder_posted,
        props::EMAIL_PRESENT: record.email_present(),
        props::EMAILS: record.emails,
        props::COMP: record.comp,
        props::POSTED_AT: record.posted_at,
        props::DOMAIN: record.company_domain,
        props::EMBEDDING: embedding,
    })
}

/// Insert a node if absent. Returns true if it was newly inserted (for counting
/// distinct companies/skills/sources).
fn insert_node(
    map: &mut HashMap<String, NodeSpec>,
    id: &str,
    label: &str,
    properties: Value,
) -> bool {
    let fresh = !map.contains_key(id);
    map.insert(
        id.to_string(),
        NodeSpec {
            id: id.to_string(),
            labels: vec![label.to_string()],
            properties,
        },
    );
    fresh
}

fn add_edge(map: &mut HashMap<String, EdgeSpec>, from: &str, edge_type: &str, to: &str) {
    let id = edge_id(from, edge_type, to);
    map.entry(id.clone()).or_insert_with(|| EdgeSpec {
        id,
        from_id: from.to_string(),
        to_id: to.to_string(),
        edge_type: edge_type.to_string(),
        properties: json!({}),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_of_matches_vocab_token_exact() {
        let body =
            "We build a Rust graph database with vector search, RAG, and MCP tools for agents.";
        let skills = skills_of(body);
        assert!(skills.contains(&"rust".to_string()));
        assert!(skills.contains(&"graph".to_string()));
        assert!(skills.contains(&"vector".to_string()));
        assert!(skills.contains(&"rag".to_string()));
        assert!(skills.contains(&"mcp".to_string()));
        assert!(
            skills.contains(&"agent".to_string()),
            "plural 'agents' should match 'agent'"
        );
    }

    #[test]
    fn skills_of_avoids_false_substring_matches() {
        // "storage" must not match "rag"; "programming" must not match nothing odd.
        let skills = skills_of("We need storage experience and strong programming.");
        assert!(!skills.contains(&"rag".to_string()));
    }

    #[test]
    fn skills_of_is_vocab_ordered() {
        let skills = skills_of("python rust");
        assert_eq!(skills, vec!["rust", "python"]); // vocab order, not text order
    }

    #[test]
    fn role_round_trips_through_node_props() {
        let record = JobRecord {
            id: "role:hn:1".into(),
            source: crate::model::Source::Hn,
            company: "Qdrant".into(),
            company_domain: Some("qdrant.tech".into()),
            title: "Rust Engineer".into(),
            location: "Remote".into(),
            remote: true,
            comp: Some("$150k".into()),
            url: "https://news.ycombinator.com/item?id=1".into(),
            body: "Build a vector database in Rust.".into(),
            posted_at: None,
            emails: vec!["hiring@qdrant.tech".into()],
            contract: true,
            founder_posted: true,
        };
        let node = json!({
            "id": record.id,
            "labels": ["Role"],
            "properties": role_props(&record, &[0.1, 0.2]),
        });
        let role = role_from_node(&node).unwrap();
        assert_eq!(role.title, "Rust Engineer");
        assert_eq!(role.company, "Qdrant");
        assert!(role.remote && role.contract && role.founder_posted && role.email_present);
        assert_eq!(role.emails, vec!["hiring@qdrant.tech"]);
        assert_eq!(role.company_domain.as_deref(), Some("qdrant.tech"));
    }
}
