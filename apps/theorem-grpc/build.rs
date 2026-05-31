// Compile theseus_search.v1 into Rust tonic bindings at build time.
//
// Single in-tree proto root: `proto/theseus_search/v1/search.proto`, copied
// byte-identical from the canonical source at
// `RustyRed-Graph-Database/proto/theseus_search/v1/search.proto` (which is in
// turn byte-identical to the vendored copy the civic backend compiles). No
// submodule fallback is needed: the proto ships inside this crate.
//
// build_client(false): this binary is a server only. The civic-atlas-server
// owns the client side; we just need wire-compatible server stubs.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let proto_path = proto_root
        .join("theseus_search")
        .join("v1")
        .join("search.proto");

    println!("cargo:rerun-if-changed={}", proto_path.display());

    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(&[&proto_path], &[&proto_root])?;

    Ok(())
}
