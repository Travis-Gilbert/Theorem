//! jobintel - job-intelligence pipeline on RustyRed.
//!
//!   jobintel ingest [--dry-run]      pull HN + ATS, (optionally) write the graph
//!   jobintel rank   --profile <id>   blend semantic + graph + flag signals
//!   jobintel draft  --top <n>        write per-lead context packs + queue.md
//!
//! `ingest --dry-run` needs no RustyRed (only network for the sources), so the
//! pipeline is inspectable from one command before any server is wired up.

mod client;
mod config;
mod contacts;
mod draft;
mod embed;
mod error;
mod graph;
mod ingest;
mod model;
mod profile;
mod rank;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::client::RustyRedClient;
use crate::config::Config;
use crate::contacts::{fill_contacts, HunterFinder};
use crate::embed::{build_embedder, EmbedKind};
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
