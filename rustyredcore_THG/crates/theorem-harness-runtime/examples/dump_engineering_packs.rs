//! Throwaway: print the three engineering packs as JSON for live registry seeding.
//! Run with `cargo run -p theorem-harness-runtime --example dump_engineering_packs`.

use theorem_harness_runtime::engineering_packs::engineering_capability_packs;

fn main() {
    let packs: Vec<_> = engineering_capability_packs()
        .into_iter()
        .map(|pack| {
            serde_json::json!({
                "name": pack.name,
                "pack_content_hash": pack.pack_content_hash,
                "status": pack.status,
                "pack": pack.pack,
            })
        })
        .collect();
    println!("{}", serde_json::to_string(&packs).unwrap());
}
