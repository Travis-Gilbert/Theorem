// Local uniffi-bindgen entry point so Swift bindings generate from this crate's
// built library without a separately-installed tool:
//   cargo run --bin uniffi-bindgen -- generate \
//     --library target/debug/libtheorem_harness_swift.dylib \
//     --language swift --out-dir generated
fn main() {
    uniffi::uniffi_bindgen_main()
}
