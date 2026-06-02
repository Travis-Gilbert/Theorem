// napi_build::setup() emits the platform link args a Node addon needs. On macOS
// it sets `-undefined dynamic_lookup` so the cdylib links even though Node's
// symbols are only present at load time. This is why plain `cargo build`
// produces a loadable `.node` without the napi CLI.
fn main() {
    napi_build::setup();
}
