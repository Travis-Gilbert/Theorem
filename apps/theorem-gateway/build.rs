// Compile the vendored theorem protos into Rust tonic CLIENT bindings at build
// time.
//
// The gateway is a client of theorem-grpc, so it generates client stubs only:
//   build_client(true).build_server(false)
//
// The protos are vendored byte-identical from apps/theorem-grpc/proto (the
// gateway never edits them; theorem-protos / the code-crawler proto remain the
// source of truth). Only the two services the gateway resolves over are
// compiled: theseus_search.v1 (SearchService) and theorem_code.v1
// (CodeCrawlerService). theorem_grpc.AppAffordanceService is intentionally not
// compiled (not needed for v1 of the gateway, per the spec).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let search_proto_path = proto_root
        .join("theseus_search")
        .join("v1")
        .join("search.proto");
    let code_crawler_proto_path = proto_root
        .join("theorem_code")
        .join("v1")
        .join("code_crawler.proto");

    println!("cargo:rerun-if-changed={}", search_proto_path.display());
    println!(
        "cargo:rerun-if-changed={}",
        code_crawler_proto_path.display()
    );

    tonic_build::configure()
        .build_client(true)
        .build_server(false)
        .compile_protos(
            &[&search_proto_path, &code_crawler_proto_path],
            &[&proto_root],
        )?;

    Ok(())
}
