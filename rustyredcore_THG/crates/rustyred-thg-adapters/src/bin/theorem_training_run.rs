use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::json;

use rustyred_thg_adapters::{
    export_training_snapshot_files, import_gnn_export_dir, run_local_training_smoke,
    seed_training_fixture, writeback_model_artifact_file, GnnExportImportOptions,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_usage();
        return Ok(());
    }

    let command = args.remove(0);
    let opts = parse_options(args)?;
    match command.as_str() {
        "fixture" => {
            let data_dir = required_path(&opts, "data-dir")?;
            let tenant = option_or(&opts, "tenant", "theorem");
            let actor = opts.get("actor").map(String::as_str);
            let fixture = seed_training_fixture(data_dir, tenant, actor).map_err(format_thg)?;
            print_json(json!({
                "ok": true,
                "command": "fixture",
                "tenant_id": fixture.tenant_id,
                "adapter_node_id": fixture.adapter_node_id,
                "training_pack_node_id": fixture.training_pack_node_id,
                "reasoning_trace_node_ids": fixture.reasoning_trace_node_ids,
                "graph_version": fixture.transaction.graph_version
            }))
        }
        "gnn-import" => {
            let data_dir = required_path(&opts, "data-dir")?;
            let export_dir = required_path(&opts, "export-dir")?;
            let tenant = option_or(&opts, "tenant", "theorem");
            let export_id = option_or(&opts, "export-id", "theseus-gnn-export");
            let actor = opts.get("actor").map(String::as_str);
            let import = import_gnn_export_dir(
                data_dir,
                export_dir,
                tenant,
                export_id,
                GnnExportImportOptions {
                    batch_size: option_usize(&opts, "batch-size").unwrap_or(10_000),
                    max_entities: option_usize(&opts, "max-entities"),
                    max_triples: option_usize(&opts, "max-triples"),
                    max_temporal_triples: option_usize(&opts, "max-temporal-triples"),
                },
                actor,
            )
            .map_err(format_thg)?;
            print_json(json!({
                "ok": true,
                "command": "gnn-import",
                "tenant_id": import.tenant_id,
                "export_id": import.export_id,
                "training_pack_node_id": import.training_pack_node_id,
                "gnn_export_node_id": import.gnn_export_node_id,
                "imported_entity_nodes": import.imported_entity_nodes,
                "imported_sha_map_nodes": import.imported_sha_map_nodes,
                "imported_triple_edges": import.imported_triple_edges,
                "imported_temporal_edges": import.imported_temporal_edges,
                "skipped_triples": import.skipped_triples,
                "skipped_temporal_triples": import.skipped_temporal_triples,
                "artifact_nodes": import.artifact_nodes,
                "transaction_count": import.transaction_count,
                "graph_version": import.graph_version
            }))
        }
        "export" => {
            let data_dir = required_path(&opts, "data-dir")?;
            let output_dir = required_path(&opts, "output-dir")?;
            let tenant = option_or(&opts, "tenant", "theorem");
            let export_id = option_or(&opts, "export-id", "training-export");
            let export = export_training_snapshot_files(data_dir, tenant, export_id, output_dir)
                .map_err(format_thg)?;
            print_json(json!({
                "ok": true,
                "command": "export",
                "manifest_path": export.manifest_path,
                "graph_snapshot_path": export.graph_snapshot_path,
                "runpod_input_path": export.runpod_input_path,
                "graph_version": export.manifest.graph_version,
                "snapshot_hash": export.manifest.snapshot_hash,
                "counts": export.manifest.counts
            }))
        }
        "writeback" => {
            let data_dir = required_path(&opts, "data-dir")?;
            let input = required_path(&opts, "input")?;
            let actor = opts.get("actor").map(String::as_str);
            let writeback =
                writeback_model_artifact_file(data_dir, input, actor).map_err(format_thg)?;
            print_json(json!({
                "ok": true,
                "command": "writeback",
                "model_node_id": writeback.model_node_id,
                "evaluation_node_id": writeback.evaluation_node_id,
                "graph_version": writeback.transaction.graph_version
            }))
        }
        "smoke" => {
            let data_dir = required_path(&opts, "data-dir")?;
            let output_dir = required_path(&opts, "output-dir")?;
            let tenant = option_or(&opts, "tenant", "theorem");
            let export_id = option_or(&opts, "export-id", "training-export-smoke");
            let model_id = option_or(&opts, "model-id", "theorem-rustyred-smoke-v1");
            let model_type = option_or(&opts, "model-type", "paraphramer");
            let promotion = option_or(&opts, "promotion-decision", "shadow");
            let s3_uri = opts.get("s3-uri").map(String::as_str);
            let actor = opts.get("actor").map(String::as_str);
            let result = run_local_training_smoke(
                data_dir, output_dir, tenant, export_id, model_id, model_type, s3_uri, promotion,
                actor,
            )
            .map_err(format_thg)?;
            print_json(json!({
                "ok": true,
                "command": "smoke",
                "manifest_path": result.export.manifest_path,
                "graph_snapshot_path": result.export.graph_snapshot_path,
                "runpod_input_path": result.export.runpod_input_path,
                "snapshot_hash": result.export.manifest.snapshot_hash,
                "graph_version": result.writeback.transaction.graph_version,
                "model_node_id": result.writeback.model_node_id,
                "evaluation_node_id": result.writeback.evaluation_node_id,
                "counts": result.export.manifest.counts
            }))
        }
        _ => Err(format!("unknown command: {command}")),
    }
}

fn parse_options(args: Vec<String>) -> Result<BTreeMap<String, String>, String> {
    let mut opts = BTreeMap::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let Some(stripped) = arg.strip_prefix("--") else {
            return Err(format!("unexpected positional argument: {arg}"));
        };
        let Some(value) = iter.next() else {
            return Err(format!("missing value for --{stripped}"));
        };
        opts.insert(stripped.to_string(), value);
    }
    Ok(opts)
}

fn required_path(opts: &BTreeMap<String, String>, key: &str) -> Result<PathBuf, String> {
    opts.get(key)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("--{key} is required"))
}

fn option_or<'a>(opts: &'a BTreeMap<String, String>, key: &str, default: &'a str) -> &'a str {
    opts.get(key)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(default)
}

fn option_usize(opts: &BTreeMap<String, String>, key: &str) -> Option<usize> {
    opts.get(key)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| value.parse::<usize>().ok())
}

fn print_json(value: serde_json::Value) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(&value).map_err(|err| err.to_string())?;
    println!("{raw}");
    Ok(())
}

fn format_thg(err: rustyred_thg_core::ThgError) -> String {
    format!("{}: {}", err.code, err.message)
}

fn print_usage() {
    println!(
        r#"theorem_training_run

Commands:
  fixture   --data-dir DIR [--tenant theorem] [--actor NAME]
  gnn-import --data-dir DIR --export-dir DIR [--tenant theorem]
            [--export-id ID] [--batch-size N]
            [--max-entities N] [--max-triples N]
            [--max-temporal-triples N] [--actor NAME]
  export    --data-dir DIR --output-dir DIR [--tenant theorem] [--export-id ID]
  writeback --data-dir DIR --input model_artifact.json [--actor NAME]
  smoke     --data-dir DIR --output-dir DIR [--tenant theorem] [--export-id ID]
            [--model-id ID] [--model-type paraphramer] [--s3-uri s3://...]
            [--promotion-decision shadow] [--actor NAME]
"#
    );
}
