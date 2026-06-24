use rustyred_thg_ml::{run_fixture_benchmark, MultiVectorBenchmarkConfig};

fn main() {
    let report = run_fixture_benchmark(MultiVectorBenchmarkConfig::default())
        .expect("multi-vector fixture benchmark");

    println!("# Multi-vector Fixture Benchmark");
    println!();
    println!(
        "Config: corpus={}, queries={}, vectors_per_document={}, dim={}, exact_top_k={}",
        report.config.corpus_size,
        report.config.query_count,
        report.config.vectors_per_document,
        report.config.dim,
        report.config.exact_top_k
    );
    println!();
    println!("## Binary Recall vs Exact MaxSim");
    println!();
    println!(
        "| candidate_top_k | exact_top_k | recall | overlap | missing | exact_ms | binary_ms |"
    );
    println!("|---:|---:|---:|---:|---:|---:|---:|");
    for row in &report.recall_rows {
        println!(
            "| {} | {} | {:.3} | {} | {} | {:.3} | {:.3} |",
            row.candidate_top_k,
            row.exact_top_k,
            row.recall,
            row.overlap_count,
            row.missing_count,
            row.exact_elapsed_ms,
            row.binary_elapsed_ms
        );
    }
    println!();
    println!("## Storage Shape");
    println!();
    println!("| vectors | dim | exact_f32_bytes | exact_f16_bytes | binary_bytes | f32_binary_ratio | f16_binary_ratio |");
    println!("|---:|---:|---:|---:|---:|---:|---:|");
    let storage = &report.storage_cost;
    println!(
        "| {} | {} | {} | {} | {} | {:.1} | {:.1} |",
        storage.vector_count,
        storage.dim,
        storage.exact_f32_bytes,
        storage.exact_f16_bytes,
        storage.binary_projection_bytes,
        storage.exact_f32_to_binary_ratio(),
        storage.exact_f16_to_binary_ratio()
    );
    println!();
    println!("## Backends");
    println!();
    println!("| backend | status | notes |");
    println!("|---|---|---|");
    for row in &report.backend_rows {
        println!("| {} | {} | {} |", row.backend, row.status, row.notes);
    }
}
