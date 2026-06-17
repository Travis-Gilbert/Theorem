//! Module 5 - learning signal.
//!
//! On each terminal state (replied / dead) jobintel writes an append-only
//! `OutcomeRecord` carrying the template used, the lead type, the touch count,
//! and the outcome. `outreach stats` aggregates them into reply rate per template
//! and per lead type. Feeding reply-rate back into the 0.1 rank weights is named
//! for 0.3, not built here; 0.2 only records the signal.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::client::{EdgeSpec, NodeSpec, RustyRedClient};
use crate::error::Result;
use crate::model::{edge_id, edges, labels, props};

use super::state::LeadState;
use super::{fmt_timestamp, Clock, OutreachStatus};

/// Write the terminal outcome for a lead (one `OutcomeRecord` per role). Called
/// when a lead reaches `replied` (sync) or `dead` (cadence reap).
pub fn record_outcome(
    client: &RustyRedClient,
    clock: &dyn Clock,
    lead: &LeadState,
    status: OutreachStatus,
) -> Result<()> {
    let role_id = &lead.role.id;
    let lt = lead.lead_type();
    // Template was stamped at draft time; fall back to lead type if a lead became
    // terminal without a recorded template (e.g. an externally-set status).
    let template = lead
        .template
        .clone()
        .unwrap_or_else(|| lt.as_str().to_string());
    let outcome_id = format!("outcome:{role_id}");

    client.upsert_node(&NodeSpec {
        id: outcome_id.clone(),
        labels: vec![labels::OUTCOME_RECORD.to_string()],
        properties: json!({
            props::ROLE_ID: role_id,
            props::TEMPLATE: template,
            props::STATUS: status.as_str(),
            props::TOUCHES: lead.touch_count,
            props::LEAD_TYPE: lt.as_str(),
            props::AT: fmt_timestamp(clock.now()),
        }),
    })?;
    client.upsert_edge(&EdgeSpec {
        id: edge_id(role_id, edges::HAS_OUTCOME, &outcome_id),
        from_id: role_id.clone(),
        to_id: outcome_id,
        edge_type: edges::HAS_OUTCOME.to_string(),
        properties: json!({ props::STATUS: status.as_str() }),
    })?;
    Ok(())
}

/// One terminal outcome row read back from the graph.
#[derive(Debug, Clone)]
pub struct OutcomeRow {
    pub template: String,
    pub status: String,
    pub touches: u32,
    pub lead_type: String,
}

/// Read every `OutcomeRecord` node.
pub fn read_outcomes(client: &RustyRedClient) -> Result<Vec<OutcomeRow>> {
    let nodes = client.query_nodes(labels::OUTCOME_RECORD, None)?;
    Ok(nodes.iter().filter_map(row_from_node).collect())
}

fn row_from_node(node: &Value) -> Option<OutcomeRow> {
    let p = node.get("properties")?;
    let get = |k: &str| {
        p.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Some(OutcomeRow {
        template: get(props::TEMPLATE),
        status: get(props::STATUS),
        touches: p.get(props::TOUCHES).and_then(Value::as_u64).unwrap_or(0) as u32,
        lead_type: get(props::LEAD_TYPE),
    })
}

/// Replied / dead counts (and touch total) for one group.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GroupStat {
    pub replied: usize,
    pub dead: usize,
    /// Sum of touches across the group's outcomes (for average touches-to-outcome).
    pub touch_sum: usize,
}

impl GroupStat {
    pub fn total(self) -> usize {
        self.replied + self.dead
    }

    /// Reply rate in [0,1]; 0 for an empty group.
    pub fn reply_rate(self) -> f64 {
        if self.total() == 0 {
            0.0
        } else {
            self.replied as f64 / self.total() as f64
        }
    }

    /// Mean outbound touches per outcome; 0 for an empty group.
    pub fn avg_touches(self) -> f64 {
        if self.total() == 0 {
            0.0
        } else {
            self.touch_sum as f64 / self.total() as f64
        }
    }

    fn record(&mut self, status: &str, touches: u32) {
        match status {
            "replied" => self.replied += 1,
            "dead" => self.dead += 1,
            _ => return,
        }
        self.touch_sum += touches as usize;
    }
}

/// Aggregated reply rates (spec Module 5: "reply rate per template and per lead
/// type"). BTreeMaps so the printed output is stably ordered.
#[derive(Debug, Default, Clone)]
pub struct OutcomeStats {
    pub by_template: BTreeMap<String, GroupStat>,
    pub by_lead_type: BTreeMap<String, GroupStat>,
    pub total: GroupStat,
}

/// Aggregate outcome rows into per-template and per-lead-type reply rates. Pure.
pub fn compute_stats(rows: &[OutcomeRow]) -> OutcomeStats {
    let mut stats = OutcomeStats::default();
    for row in rows {
        stats
            .by_template
            .entry(row.template.clone())
            .or_default()
            .record(&row.status, row.touches);
        stats
            .by_lead_type
            .entry(row.lead_type.clone())
            .or_default()
            .record(&row.status, row.touches);
        stats.total.record(&row.status, row.touches);
    }
    stats
}

/// Render the stats as a plain-text report.
pub fn render_stats(stats: &OutcomeStats) -> String {
    let mut s = String::new();
    s.push_str("jobintel outreach stats (reply rate over recorded terminal outcomes)\n\n");
    if stats.total.total() == 0 {
        s.push_str("No terminal outcomes yet. Run the loop (draft -> sync -> followups) first.\n");
        return s;
    }
    s.push_str(&section("By template", &stats.by_template));
    s.push('\n');
    s.push_str(&section("By lead type", &stats.by_lead_type));
    s.push('\n');
    s.push_str(&format!(
        "Overall: {}/{} replied ({:.0}%), {} dead.\n",
        stats.total.replied,
        stats.total.total(),
        stats.total.reply_rate() * 100.0,
        stats.total.dead,
    ));
    s
}

fn section(title: &str, groups: &BTreeMap<String, GroupStat>) -> String {
    let mut s = format!("{title}:\n");
    s.push_str("  group                 replied  dead  reply_rate  avg_touches\n");
    for (name, g) in groups {
        s.push_str(&format!(
            "  {:<20}  {:>7}  {:>4}  {:>9.0}%  {:>11.1}\n",
            name,
            g.replied,
            g.dead,
            g.reply_rate() * 100.0,
            g.avg_touches(),
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(template: &str, lead_type: &str, status: &str) -> OutcomeRow {
        OutcomeRow {
            template: template.into(),
            status: status.into(),
            touches: 1,
            lead_type: lead_type.into(),
        }
    }

    #[test]
    fn group_stat_reply_rate_and_avg_touches() {
        let mut g = GroupStat::default();
        g.record("replied", 1);
        g.record("replied", 3);
        g.record("dead", 2);
        assert_eq!(g.total(), 3);
        assert!((g.reply_rate() - 2.0 / 3.0).abs() < 1e-9);
        assert!((g.avg_touches() - 2.0).abs() < 1e-9); // (1+3+2)/3
        assert_eq!(GroupStat::default().reply_rate(), 0.0);
        assert_eq!(GroupStat::default().avg_touches(), 0.0);
    }

    #[test]
    fn compute_stats_groups_by_template_and_lead_type() {
        let rows = vec![
            row("hn_founder", "hn_founder", "replied"),
            row("hn_founder", "hn_founder", "dead"),
            row("ats_role", "ats_role", "dead"),
            row("contract_explicit", "contract_explicit", "replied"),
        ];
        let stats = compute_stats(&rows);
        assert_eq!(stats.by_template["hn_founder"].replied, 1);
        assert_eq!(stats.by_template["hn_founder"].dead, 1);
        assert!((stats.by_template["hn_founder"].reply_rate() - 0.5).abs() < 1e-9);
        assert_eq!(stats.by_template["ats_role"].reply_rate(), 0.0);
        assert_eq!(stats.by_template["contract_explicit"].reply_rate(), 1.0);
        assert_eq!(stats.total.replied, 2);
        assert_eq!(stats.total.dead, 2);

        let report = render_stats(&stats);
        assert!(report.contains("By template"));
        assert!(report.contains("By lead type"));
        assert!(report.contains("hn_founder"));
        assert!(report.contains("Overall: 2/4"));
    }

    #[test]
    fn render_stats_handles_empty() {
        let report = render_stats(&OutcomeStats::default());
        assert!(report.contains("No terminal outcomes yet"));
    }
}
