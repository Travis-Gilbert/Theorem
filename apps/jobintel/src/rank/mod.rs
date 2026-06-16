//! Module 3 - rank. Blends three signals into one score per Role:
//!   semantic : vector search with the Profile embedding (nearest Roles)
//!   structure: PPR seeded on Profile + its Skills, plus PageRank as a
//!              hiring-spike proxy (company posting many roles ranks up)
//!   flags    : founder_posted / email_present / remote / contract boosts
//!
//! The IO (graph reads) is in `rank`; the scoring math is the pure
//! `compute_leads`, unit-tested with synthetic signal maps.

use std::collections::{HashMap, HashSet};

use crate::client::RustyRedClient;
use crate::error::Result;
use crate::graph::{role_from_node, skills_of};
use crate::model::{labels, props, skill_id, Role, ScoredLead};
use crate::profile::ResolvedProfile;

/// Signal weights. `rank --profile travis` exposes these as CLI flags.
#[derive(Debug, Clone, Copy)]
pub struct RankWeights {
    pub sem: f32,
    pub graph: f32,
    pub flags: f32,
}

impl Default for RankWeights {
    fn default() -> Self {
        Self {
            sem: 0.5,
            graph: 0.35,
            flags: 0.15,
        }
    }
}

/// PPR seeds: the Profile node and each of its Skill nodes, equally weighted.
/// Mass flows Profile -> Skill -> Role -> Company over the reverse-traversal
/// edges, warming roles/companies that share the profile's skills.
pub fn profile_seeds(profile: &ResolvedProfile) -> HashMap<String, f64> {
    let mut seeds = HashMap::new();
    seeds.insert(profile.id.clone(), 1.0);
    for skill in &profile.skills {
        seeds.insert(skill_id(skill), 1.0);
    }
    seeds
}

/// Full ranking pass: read Roles, gather the three signals from RustyRed, blend.
pub fn rank(
    client: &RustyRedClient,
    profile: &ResolvedProfile,
    weights: RankWeights,
    top_k: Option<usize>,
) -> Result<Vec<ScoredLead>> {
    // 1. Read every Role node.
    let role_nodes = client.query_nodes(labels::ROLE, None)?;
    let roles: Vec<Role> = role_nodes.iter().filter_map(role_from_node).collect();
    if roles.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Semantic: nearest Roles to the Profile embedding (k = all roles).
    let hits = client.vector_search(
        &profile.embedding,
        roles.len().max(1),
        Some(labels::ROLE),
        props::EMBEDDING,
    )?;
    let distances: HashMap<String, f64> = hits
        .into_iter()
        .map(|h| (h.node_id, h.distance as f64))
        .collect();

    // 3. Structure: PPR (seeded) + PageRank (global hiring-spike proxy).
    let seeds = profile_seeds(profile);
    let ppr: HashMap<String, f64> = client
        .ppr(&seeds, None)?
        .into_iter()
        .map(|s| (s.node_id, s.score))
        .collect();
    let pagerank: HashMap<String, f64> = client
        .pagerank(None)?
        .into_iter()
        .map(|s| (s.node_id, s.score))
        .collect();

    let profile_skills: HashSet<String> = profile.skills.iter().cloned().collect();
    let mut leads = compute_leads(
        &roles,
        &profile_skills,
        &distances,
        &ppr,
        &pagerank,
        weights,
    );
    if let Some(k) = top_k {
        leads.truncate(k);
    }
    Ok(leads)
}

/// Pure scoring: given Roles and the three raw signal maps, produce sorted
/// `ScoredLead`s. Each signal is min-max normalized to [0,1] before weighting so
/// the weights are comparable regardless of the underlying metric.
pub fn compute_leads(
    roles: &[Role],
    profile_skills: &HashSet<String>,
    distances: &HashMap<String, f64>,
    ppr: &HashMap<String, f64>,
    pagerank: &HashMap<String, f64>,
    weights: RankWeights,
) -> Vec<ScoredLead> {
    let role_ids: Vec<&str> = roles.iter().map(|r| r.id.as_str()).collect();

    // Semantic: lower distance = nearer. Convert to a similarity in [0,1].
    let semantic = similarity_from_distances(&role_ids, distances);
    // Structure: normalize PPR over roles and PageRank over their companies.
    let ppr_norm = min_max_over(&role_ids, ppr);
    let company_ids: Vec<&str> = roles.iter().map(|r| r.company_id.as_str()).collect();
    let spike_norm = min_max_over(&company_ids, pagerank);

    let mut leads: Vec<ScoredLead> = roles
        .iter()
        .map(|role| {
            let sem = *semantic.get(role.id.as_str()).unwrap_or(&0.0);
            let ppr_score = *ppr_norm.get(role.id.as_str()).unwrap_or(&0.0);
            let spike = *spike_norm.get(role.company_id.as_str()).unwrap_or(&0.0);
            // PPR (skill-relatedness) dominates; hiring-spike is a lighter nudge.
            let graph = 0.7 * ppr_score + 0.3 * spike;
            let flags = flag_score(role);

            let score = weights.sem as f64 * sem
                + weights.graph as f64 * graph
                + weights.flags as f64 * flags;

            let matched_skills: Vec<String> = skills_of(&role.body)
                .into_iter()
                .filter(|s| profile_skills.contains(s))
                .collect();

            ScoredLead {
                role: role.clone(),
                score: score as f32,
                semantic: sem as f32,
                graph: graph as f32,
                flags: flags as f32,
                matched_skills,
                contact: role.emails.first().cloned(),
                needs_contact: !role.email_present,
            }
        })
        .collect();

    leads.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.role.id.cmp(&b.role.id))
    });
    leads
}

/// Each of the four flags contributes equally; result in [0,1].
fn flag_score(role: &Role) -> f64 {
    let mut n = 0.0;
    if role.founder_posted {
        n += 1.0;
    }
    if role.email_present {
        n += 1.0;
    }
    if role.remote {
        n += 1.0;
    }
    if role.contract {
        n += 1.0;
    }
    n / 4.0
}

/// Min-max normalize the values for `ids` into [0,1]. Missing ids score 0; an
/// all-equal set scores a neutral 0.5 (no signal to differentiate).
fn min_max_over(ids: &[&str], values: &HashMap<String, f64>) -> HashMap<String, f64> {
    let present: Vec<f64> = ids
        .iter()
        .filter_map(|id| values.get(*id).copied())
        .collect();
    let (min, max) = match (
        present.iter().cloned().fold(f64::INFINITY, f64::min),
        present.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    ) {
        (mn, mx) if mn.is_finite() && mx.is_finite() => (mn, mx),
        _ => return HashMap::new(),
    };
    let span = max - min;
    ids.iter()
        .map(|id| {
            let raw = values.get(*id).copied().unwrap_or(min);
            let norm = if span > 0.0 { (raw - min) / span } else { 0.5 };
            (id.to_string(), norm)
        })
        .collect()
}

/// Turn vector distances into similarities in [0,1] (nearer = higher). Roles
/// with no hit are treated as the farthest.
fn similarity_from_distances(
    ids: &[&str],
    distances: &HashMap<String, f64>,
) -> HashMap<String, f64> {
    let present: Vec<f64> = ids
        .iter()
        .filter_map(|id| distances.get(*id).copied())
        .collect();
    if present.is_empty() {
        return ids.iter().map(|id| (id.to_string(), 0.0)).collect();
    }
    let min = present.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = present.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;
    ids.iter()
        .map(|id| {
            // Missing => farthest (max distance) => similarity 0.
            let d = distances.get(*id).copied().unwrap_or(max);
            let sim = if span > 0.0 {
                1.0 - (d - min) / span
            } else {
                1.0
            };
            (id.to_string(), sim)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn role(
        id: &str,
        company: &str,
        remote: bool,
        contract: bool,
        founder: bool,
        email: bool,
        body: &str,
    ) -> Role {
        Role {
            id: id.into(),
            company: company.into(),
            company_id: format!("company:{company}"),
            title: "Engineer".into(),
            location: "Remote".into(),
            url: "https://x".into(),
            body: body.into(),
            source: Source::Hn.as_str().into(),
            remote,
            contract,
            founder_posted: founder,
            email_present: email,
            emails: if email {
                vec!["a@b.com".into()]
            } else {
                vec![]
            },
            comp: None,
            company_domain: None,
        }
    }

    #[test]
    fn min_max_normalizes_and_handles_constants() {
        let ids = ["a", "b", "c"];
        let mut v = HashMap::new();
        v.insert("a".to_string(), 10.0);
        v.insert("b".to_string(), 20.0);
        v.insert("c".to_string(), 30.0);
        let n = min_max_over(&ids, &v);
        assert!((n["a"] - 0.0).abs() < 1e-9);
        assert!((n["c"] - 1.0).abs() < 1e-9);

        let mut flat = HashMap::new();
        flat.insert("a".to_string(), 5.0);
        flat.insert("b".to_string(), 5.0);
        let n2 = min_max_over(&["a", "b"], &flat);
        assert!(
            (n2["a"] - 0.5).abs() < 1e-9,
            "constant signal is neutral 0.5"
        );
    }

    #[test]
    fn flags_boost_surfaces_contract_remote_founder_email() {
        let loaded = role("r1", "acme", true, true, true, true, "rust graph");
        let bare = role("r2", "acme", false, false, false, false, "rust graph");
        assert!((flag_score(&loaded) - 1.0).abs() < 1e-9);
        assert!((flag_score(&bare) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn contract_remote_founder_role_outranks_bare_role() {
        // Two roles, identical semantic + graph signal; flags must break the tie
        // and lift the contract/remote/founder/email role to the top - the
        // shape the spec's acceptance describes.
        let roles = vec![
            role(
                "r_bare",
                "bigco",
                false,
                false,
                false,
                false,
                "rust vector graph",
            ),
            role(
                "r_hot",
                "startup",
                true,
                true,
                true,
                true,
                "rust vector graph",
            ),
        ];
        let skills: HashSet<String> = ["rust", "vector", "graph"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let mut distances = HashMap::new();
        distances.insert("r_bare".to_string(), 0.5);
        distances.insert("r_hot".to_string(), 0.5); // equal semantic
        let mut ppr = HashMap::new();
        ppr.insert("r_bare".to_string(), 1.0);
        ppr.insert("r_hot".to_string(), 1.0); // equal graph
        let pagerank = HashMap::new();

        let leads = compute_leads(
            &roles,
            &skills,
            &distances,
            &ppr,
            &pagerank,
            RankWeights::default(),
        );
        assert_eq!(
            leads[0].role.id, "r_hot",
            "flag-rich role must surface first"
        );
        assert!(leads[0].matched_skills.contains(&"rust".to_string()));
        assert!(
            leads[1].needs_contact,
            "bare role has no email => needs_contact"
        );
    }
}
