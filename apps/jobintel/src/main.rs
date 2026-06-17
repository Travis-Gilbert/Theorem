//! jobintel - job-intelligence + outreach pipeline on RustyRed.
//!
//!   jobintel ingest [--dry-run]      pull HN + ATS, (optionally) write the graph
//!   jobintel rank   --profile <id>   blend semantic + graph + flag signals
//!   jobintel draft  --top <n>        write per-lead context packs + queue.md
//!
//!   jobintel outreach queue          today's work: to-draft, drafted, follow-ups due
//!   jobintel outreach draft --top n  draft the top queued leads into Gmail (never sends)
//!   jobintel outreach sync           detect replies + advance drafted -> sent
//!   jobintel outreach followups      draft the next nudge for leads past their date
//!   jobintel outreach stats          reply rate per template + per lead type
//!
//! `ingest --dry-run` needs no RustyRed (only network for the sources), so the
//! pipeline is inspectable from one command before any server is wired up. The
//! outreach verbs need a running RustyRed; draft/sync/followups also need
//! `GMAIL_TOKEN` (jobintel writes Gmail drafts, never sends).

mod client;
mod config;
mod contacts;
mod draft;
mod embed;
mod error;
mod graph;
mod ingest;
mod model;
mod outreach;
mod profile;
mod rank;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use std::collections::{HashMap, HashSet};

use crate::client::RustyRedClient;
use crate::config::Config;
use crate::contacts::{fill_contacts, HunterFinder};
use crate::embed::{build_embedder, EmbedKind};
use crate::outreach::draft as outreach_draft;
use crate::outreach::gmail::GmailClient;
use crate::outreach::{cadence, outcomes, state, sync, Clock, EventKind, OutreachStatus, SystemClock};
use crate::rank::RankWeights;

#[derive(Parser)]
#[command(
    name = "jobintel",
    version,
    about = "Ingest open job sources into RustyRed, rank roles against a profile, draft a lead queue."
)]
struct Cli {
    /// Embedder backend: hash (offline default) | http | bge (feature-gated).
    #[arg(long, global = true, default_value = "hash")]
    embedder: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch HN "Who is Hiring" + ATS boards into RustyRed as a graph.
    Ingest {
        /// Print the fetched records without writing to RustyRed.
        #[arg(long)]
        dry_run: bool,
        /// Path to the company seed list.
        #[arg(long, default_value = "slugs.toml")]
        slugs: String,
        /// Skip the Hacker News source (ATS only).
        #[arg(long)]
        no_hn: bool,
    },
    /// Rank every Role against a profile and print the shortlist.
    Rank {
        /// Profile handle or path (profiles/<id>.toml).
        #[arg(long)]
        profile: String,
        /// How many leads to print.
        #[arg(long, default_value_t = 20)]
        top: usize,
        #[arg(long, default_value_t = 0.5)]
        w_sem: f32,
        #[arg(long, default_value_t = 0.35)]
        w_graph: f32,
        #[arg(long, default_value_t = 0.15)]
        w_flags: f32,
    },
    /// Write per-lead context packs + out/queue.md for the top leads.
    Draft {
        /// How many leads to draft.
        #[arg(long, default_value_t = 5)]
        top: usize,
        /// Profile handle or path (profiles/<id>.toml).
        #[arg(long, default_value = "travis")]
        profile: String,
        /// Output directory.
        #[arg(long, default_value = "out")]
        out: String,
        #[arg(long, default_value_t = 0.5)]
        w_sem: f32,
        #[arg(long, default_value_t = 0.35)]
        w_graph: f32,
        #[arg(long, default_value_t = 0.15)]
        w_flags: f32,
    },
    /// Outreach engine: triage to a daily queue, draft into Gmail, follow up, learn.
    Outreach {
        #[command(subcommand)]
        action: OutreachCmd,
    },
}

#[derive(Subcommand)]
enum OutreachCmd {
    /// Show today's work: new to draft, drafted-not-sent, follow-ups due.
    Queue,
    /// Draft the top queued leads into Gmail (capped at DAILY_DRAFT_CAP). Never sends.
    Draft {
        /// How many to draft this run (effective count is min(top, DAILY_DRAFT_CAP)).
        #[arg(long, default_value_t = 8)]
        top: usize,
        /// Profile handle or path (profiles/<id>.toml).
        #[arg(long, default_value = "travis")]
        profile: String,
        #[arg(long, default_value_t = 0.5)]
        w_sem: f32,
        #[arg(long, default_value_t = 0.35)]
        w_graph: f32,
        #[arg(long, default_value_t = 0.15)]
        w_flags: f32,
    },
    /// Detect replies and advance drafted -> sent.
    Sync,
    /// Draft the next nudge for leads past their follow-up date; reap the exhausted.
    Followups {
        /// Profile handle or path (proof block for the nudge body).
        #[arg(long, default_value = "travis")]
        profile: String,
    },
    /// Reply rate per template and per lead type over recorded outcomes.
    Stats,
    /// Print a lead's outreach event trail, reconstructed from the graph.
    Trail {
        /// Role node id (e.g. role:hn:123).
        #[arg(long)]
        role: String,
    },
    /// Manually set a lead's status (operator override, e.g. mark a lead dead).
    Mark {
        /// Role node id.
        #[arg(long)]
        role: String,
        /// New status: new | queued | drafted | sent | replied | dead.
        #[arg(long)]
        status: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let kind = EmbedKind::parse(&cli.embedder)?;

    match cli.command {
        Command::Ingest {
            dry_run,
            slugs,
            no_hn,
        } => run_ingest(kind, dry_run, &slugs, no_hn),
        Command::Rank {
            profile,
            top,
            w_sem,
            w_graph,
            w_flags,
        } => run_rank(
            kind,
            &profile,
            top,
            RankWeights {
                sem: w_sem,
                graph: w_graph,
                flags: w_flags,
            },
        ),
        Command::Draft {
            top,
            profile,
            out,
            w_sem,
            w_graph,
            w_flags,
        } => run_draft(
            kind,
            &profile,
            top,
            &out,
            RankWeights {
                sem: w_sem,
                graph: w_graph,
                flags: w_flags,
            },
        ),
        Command::Outreach { action } => run_outreach(kind, action),
    }
}

fn run_ingest(kind: EmbedKind, dry_run: bool, slugs_path: &str, no_hn: bool) -> Result<()> {
    let http = ingest::http_client()?;
    let slugs = ingest::load_slugs(slugs_path)
        .with_context(|| format!("loading slugs from {slugs_path}"))?;
    eprintln!("Ingesting sources...");
    let records = ingest::fetch_all(&http, &slugs, no_hn)?;
    eprintln!("Fetched {} records total.", records.len());

    if dry_run {
        print_dry_run(&records);
        return Ok(());
    }

    let config = Config::from_env()?;
    let client = RustyRedClient::new(&config)?;
    let embedder = build_embedder(kind, &config)?;
    eprintln!(
        "Writing graph to {} (tenant {}, embedder {}, dim {})...",
        config.rustyred_url,
        config.tenant,
        embedder.name(),
        embedder.dim()
    );
    let stats = graph::upsert_records(&client, embedder.as_ref(), &records)?;
    println!(
        "Wrote graph: {} companies, {} roles, {} skills, {} persons, {} sources",
        stats.companies, stats.roles, stats.skills, stats.persons, stats.sources
    );
    println!(
        "  nodes: {} inserted ({} failed); edges: {} inserted ({} failed)",
        stats.nodes_inserted, stats.nodes_failed, stats.edges_inserted, stats.edges_failed
    );
    Ok(())
}

fn run_rank(kind: EmbedKind, profile_handle: &str, top: usize, weights: RankWeights) -> Result<()> {
    let config = Config::from_env()?;
    let client = RustyRedClient::new(&config)?;
    let embedder = build_embedder(kind, &config)?;
    let prof = profile::load_profile(profile_handle, embedder.as_ref())?;
    graph::ensure_profile(&client, embedder.as_ref(), &prof.handle, &prof.text)?;

    let leads = rank::rank(&client, &prof, weights, Some(top))?;
    if leads.is_empty() {
        println!("No roles in the graph. Run `jobintel ingest` first.");
        return Ok(());
    }
    print_ranked(&prof.handle, &leads);
    Ok(())
}

fn run_draft(
    kind: EmbedKind,
    profile_handle: &str,
    top: usize,
    out_dir: &str,
    weights: RankWeights,
) -> Result<()> {
    let config = Config::from_env()?;
    let client = RustyRedClient::new(&config)?;
    let embedder = build_embedder(kind, &config)?;
    let prof = profile::load_profile(profile_handle, embedder.as_ref())?;
    graph::ensure_profile(&client, embedder.as_ref(), &prof.handle, &prof.text)?;

    let mut leads = rank::rank(&client, &prof, weights, Some(top))?;
    if leads.is_empty() {
        println!("No roles in the graph. Run `jobintel ingest` first.");
        return Ok(());
    }

    // Contacts: HN leads already carry emails; ATS leads go through Hunter.io.
    let finder = HunterFinder::from_key(config.hunter_api_key.as_deref());
    if finder.is_none() {
        eprintln!("  HUNTER_API_KEY unset: ATS leads without an in-post email will be marked needs_contact.");
    }
    let resolved = fill_contacts(
        finder.as_ref().map(|f| f as &dyn contacts::ContactFinder),
        &mut leads,
    )?;
    eprintln!("  contacts resolved: {}/{}", resolved, leads.len());

    let stats = draft::draft_queue(&client, &prof, &leads, top, out_dir)?;
    println!(
        "Drafted {} context packs ({} stored server-side) in {}/",
        stats.written, stats.packed, stats.out_dir
    );
    println!("Queue index: {}/queue.md", stats.out_dir);
    Ok(())
}

// ---- outreach (0.2) --------------------------------------------------------

fn run_outreach(kind: EmbedKind, action: OutreachCmd) -> Result<()> {
    match action {
        OutreachCmd::Queue => run_outreach_queue(),
        OutreachCmd::Draft {
            top,
            profile,
            w_sem,
            w_graph,
            w_flags,
        } => run_outreach_draft(
            kind,
            &profile,
            top,
            RankWeights {
                sem: w_sem,
                graph: w_graph,
                flags: w_flags,
            },
        ),
        OutreachCmd::Sync => run_outreach_sync(),
        OutreachCmd::Followups { profile } => run_outreach_followups(kind, &profile),
        OutreachCmd::Stats => run_outreach_stats(),
        OutreachCmd::Trail { role } => run_outreach_trail(&role),
        OutreachCmd::Mark { role, status } => run_outreach_mark(&role, &status),
    }
}

/// Status board: the three bounded lists of today's work. Read-only over RustyRed
/// (no Gmail, no embedder).
fn run_outreach_queue() -> Result<()> {
    let config = Config::from_env()?;
    let client = RustyRedClient::new(&config)?;
    let clock = SystemClock;
    let leads = state::read_leads(&client)?;
    if leads.is_empty() {
        println!("No roles in the graph. Run `jobintel ingest` first.");
        return Ok(());
    }
    let today = clock.today();
    let queue = state::queue(&leads, today);
    print_outreach_queue(&leads, &queue, today);
    Ok(())
}

/// Triage to the daily cap and draft the top queued leads into Gmail.
fn run_outreach_draft(
    kind: EmbedKind,
    profile_handle: &str,
    top: usize,
    weights: RankWeights,
) -> Result<()> {
    let config = Config::from_env()?;
    let client = RustyRedClient::new(&config)?;
    let sink = outreach_draft::GmailDraftSink::new(GmailClient::new(
        config.gmail_token.as_deref(),
        &config.gmail_api_base,
    )?);
    let clock = SystemClock;
    let embedder = build_embedder(kind, &config)?;
    let prof = profile::load_profile(profile_handle, embedder.as_ref())?;
    graph::ensure_profile(&client, embedder.as_ref(), &prof.handle, &prof.text)?;

    // Rank every role (no truncation: selection skips already-drafted leads and
    // still needs to fill the cap from further down the ranking).
    let mut leads = rank::rank(&client, &prof, weights, None)?;
    if leads.is_empty() {
        println!("No roles in the graph. Run `jobintel ingest` first.");
        return Ok(());
    }

    // Resolve contacts; leads still needs_contact are excluded from the queue.
    let finder = HunterFinder::from_key(config.hunter_api_key.as_deref());
    if finder.is_none() {
        eprintln!("  HUNTER_API_KEY unset: ATS leads without an in-post email stay needs_contact and are skipped.");
    }
    fill_contacts(
        finder.as_ref().map(|f| f as &dyn contacts::ContactFinder),
        &mut leads,
    )?;

    // Current outreach statuses (so we never re-draft a drafted/sent/terminal lead).
    let statuses: HashMap<String, OutreachStatus> = state::read_leads(&client)?
        .into_iter()
        .map(|l| (l.role.id, l.status))
        .collect();

    let cap = top.min(config.daily_draft_cap);
    let pick: HashSet<String> = cadence::select_for_draft(&leads, &statuses, cap)
        .into_iter()
        .collect();
    let selected: Vec<model::ScoredLead> =
        leads.into_iter().filter(|l| pick.contains(&l.role.id)).collect();
    if selected.is_empty() {
        println!("Nothing to draft: no draftable lead with a resolvable contact (cap {cap}).");
        return Ok(());
    }

    eprintln!("Drafting {} lead(s) into Gmail (cap {cap})...", selected.len());
    let stats = outreach_draft::draft_top(&client, &sink, &clock, &prof, &selected)?;
    println!("Created {} Gmail draft(s):", stats.drafted);
    for (company, draft_id) in &stats.created {
        println!("  {company}  (draft {draft_id})");
    }
    if stats.skipped_no_contact > 0 {
        println!("  ({} selected lead(s) skipped: no contact)", stats.skipped_no_contact);
    }
    println!(
        "Review in your Gmail Drafts folder and send with one click. jobintel never sends."
    );
    Ok(())
}

/// Detect replies and advance drafted -> sent against Gmail.
fn run_outreach_sync() -> Result<()> {
    let config = Config::from_env()?;
    let client = RustyRedClient::new(&config)?;
    let source = sync::GmailReplySource::new(GmailClient::new(
        config.gmail_token.as_deref(),
        &config.gmail_api_base,
    )?);
    let clock = SystemClock;
    let stats = sync::run_sync(&client, &source, &clock, &config.followup_days)?;
    println!(
        "Synced {} tracked lead(s): {} newly replied, {} advanced drafted -> sent.",
        stats.checked, stats.replied, stats.advanced_sent
    );
    Ok(())
}

/// Draft the next nudge for every lead past its follow-up date; reap the exhausted.
fn run_outreach_followups(kind: EmbedKind, profile_handle: &str) -> Result<()> {
    let config = Config::from_env()?;
    let client = RustyRedClient::new(&config)?;
    let sink = outreach_draft::GmailDraftSink::new(GmailClient::new(
        config.gmail_token.as_deref(),
        &config.gmail_api_base,
    )?);
    let clock = SystemClock;
    // Profile supplies the proof block for the nudge body; the embedding is unused
    // here, so any embedder backend works.
    let embedder = build_embedder(kind, &config)?;
    let prof = profile::load_profile(profile_handle, embedder.as_ref())?;
    let stats = cadence::run_followups(&client, &sink, &clock, &prof, &config.followup_days)?;
    println!(
        "Follow-ups: {} nudge(s) drafted, {} reaped to dead, {} skipped.",
        stats.nudged, stats.reaped, stats.skipped
    );
    for (company, draft_id) in &stats.created {
        println!("  nudge -> {company}  (draft {draft_id})");
    }
    if stats.nudged > 0 {
        println!("Nudges are threaded into the original Gmail conversations. Review and send.");
    }
    Ok(())
}

/// Reply rate per template and per lead type over recorded terminal outcomes.
fn run_outreach_stats() -> Result<()> {
    let config = Config::from_env()?;
    let client = RustyRedClient::new(&config)?;
    let rows = outcomes::read_outcomes(&client)?;
    print!("{}", outcomes::render_stats(&outcomes::compute_stats(&rows)));
    Ok(())
}

/// Reconstruct and print a lead's append-only event trail from the graph
/// (Module 1 acceptance: "the trail reconstructs from the graph").
fn run_outreach_trail(role_id: &str) -> Result<()> {
    let config = Config::from_env()?;
    let client = RustyRedClient::new(&config)?;
    let events = state::events_for_role(&client, role_id)?;
    if events.is_empty() {
        println!("No outreach events for {role_id}.");
        return Ok(());
    }
    println!("\noutreach trail for {role_id} ({} events):\n", events.len());
    for e in &events {
        println!("  {}  {:<16}  {}", e.at, e.kind, e.note);
    }
    Ok(())
}

/// Operator override: set a lead's status directly. A terminal mark (replied/dead)
/// also logs the event and records the outcome, so manual closes feed `stats`.
fn run_outreach_mark(role_id: &str, status_raw: &str) -> Result<()> {
    let status = match status_raw.trim().to_lowercase().as_str() {
        "new" => OutreachStatus::New,
        "queued" => OutreachStatus::Queued,
        "drafted" => OutreachStatus::Drafted,
        "sent" => OutreachStatus::Sent,
        "replied" => OutreachStatus::Replied,
        "dead" => OutreachStatus::Dead,
        other => {
            return Err(anyhow::anyhow!(
                "unknown status '{other}' (expected new|queued|drafted|sent|replied|dead)"
            ))
        }
    };
    let config = Config::from_env()?;
    let client = RustyRedClient::new(&config)?;
    let clock = SystemClock;

    // Read the lead first so a terminal mark can record its outcome.
    let node = state::read_node(&client, role_id)?;
    let lead = state::LeadState::from_node(&node)
        .ok_or_else(|| anyhow::anyhow!("node {role_id} is not a Role"))?;

    state::set_status(&client, role_id, status)?;
    if status.is_terminal() {
        let kind = if status == OutreachStatus::Dead {
            EventKind::MarkedDead
        } else {
            EventKind::ReplyDetected
        };
        state::log_event(&client, &clock, role_id, kind, "manual mark")?;
        outcomes::record_outcome(&client, &clock, &lead, status)?;
    }
    println!("Marked {role_id} -> {}.", status.as_str());
    Ok(())
}

fn print_outreach_queue(
    leads: &[state::LeadState],
    queue: &state::OutreachQueue,
    today: chrono::NaiveDate,
) {
    let by_id: HashMap<&str, &state::LeadState> =
        leads.iter().map(|l| (l.role.id.as_str(), l)).collect();
    println!("\noutreach queue ({}):\n", today.format("%Y-%m-%d"));
    print_queue_section("To draft (new/queued)", &queue.to_draft, &by_id, 15);
    print_queue_section("Drafted, awaiting send", &queue.drafted_unsent, &by_id, 25);
    print_queue_section("Follow-ups due", &queue.followups_due, &by_id, 25);
}

fn print_queue_section(
    title: &str,
    ids: &[String],
    by_id: &HashMap<&str, &state::LeadState>,
    cap: usize,
) {
    println!("{title}: {}", ids.len());
    for id in ids.iter().take(cap) {
        if let Some(lead) = by_id.get(id.as_str()) {
            println!(
                "  {:<26} {:<34} {}",
                truncate(&lead.role.company, 26),
                truncate(&lead.role.title, 34),
                id
            );
        }
    }
    if ids.len() > cap {
        println!("  ... and {} more", ids.len() - cap);
    }
    println!();
}

// ---- printing --------------------------------------------------------------

fn print_dry_run(records: &[model::JobRecord]) {
    use std::collections::BTreeMap;
    let mut by_source: BTreeMap<&str, usize> = BTreeMap::new();
    let mut with_email = 0usize;
    for r in records {
        *by_source.entry(r.source.as_str()).or_default() += 1;
        if r.email_present() {
            with_email += 1;
        }
    }
    println!("\n{} records across sources:", records.len());
    for (src, n) in &by_source {
        println!("  {src:<11} {n}");
    }
    println!("  with in-post email: {with_email}\n");

    for r in records {
        println!(
            "[{}] {} | {} | {} | remote={} contract={} founder={} emails={}",
            r.source.as_str(),
            truncate(&r.company, 28),
            truncate(&r.title, 36),
            truncate(
                if r.location.is_empty() {
                    "-"
                } else {
                    &r.location
                },
                18
            ),
            yn(r.remote),
            yn(r.contract),
            yn(r.founder_posted),
            r.emails.len(),
        );
    }
}

fn print_ranked(handle: &str, leads: &[crate::model::ScoredLead]) {
    println!(
        "\nRanked leads for profile '{handle}' ({} shown):\n",
        leads.len()
    );
    for (i, lead) in leads.iter().enumerate() {
        let r = &lead.role;
        let mut tags = Vec::new();
        if r.remote {
            tags.push("remote");
        }
        if r.contract {
            tags.push("contract");
        }
        if r.founder_posted {
            tags.push("founder");
        }
        if r.email_present {
            tags.push("email");
        }
        println!(
            "{:>2}. {:.3}  {:<26} {:<34} [{}]",
            i + 1,
            lead.score,
            truncate(&r.company, 26),
            truncate(&r.title, 34),
            tags.join(" ")
        );
        println!(
            "      sem={:.2} graph={:.2} flags={:.2}  skills: {}",
            lead.semantic,
            lead.graph,
            lead.flags,
            if lead.matched_skills.is_empty() {
                "-".to_string()
            } else {
                lead.matched_skills.join(",")
            }
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}...",
            s.chars().take(max.saturating_sub(3)).collect::<String>()
        )
    }
}

fn yn(b: bool) -> char {
    if b {
        'Y'
    } else {
        'N'
    }
}
