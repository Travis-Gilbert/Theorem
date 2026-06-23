#!/usr/bin/env bash
# Build the TheoremHarnessFFI.xcframework + stage the generated Swift API for the
# SwiftPM package, so the iOS app can `import TheoremHarness`.
#
# Produces (all gitignored build artifacts):
#   target/<triple>/release/libtheorem_harness_swift.a   (per-platform static libs)
#   generated/                                           (uniffi Swift + FFI header)
#   TheoremHarnessFFI.xcframework                         (device + simulator slices)
#   Sources/TheoremHarness/theorem_harness_swift.swift    (the generated API)
set -euo pipefail
cd "$(dirname "$0")"

LIB=theorem_harness_swift
A=lib${LIB}.a
TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)

echo "==> ensuring iOS rust targets"
rustup target add "${TARGETS[@]}" >/dev/null 2>&1 || true

echo "==> building static libs (release) for: ${TARGETS[*]}"
for t in "${TARGETS[@]}"; do
  cargo build --release --lib --target "$t"
done

echo "==> building host lib + generating Swift bindings"
cargo build --release --lib
cargo run --release --quiet --bin uniffi-bindgen -- generate \
  --library "target/release/lib${LIB}.dylib" --language swift --out-dir generated

echo "==> assembling headers dir (FFI header + module.modulemap)"
rm -rf headers && mkdir -p headers
cp "generated/${LIB}FFI.h" headers/
cp "generated/${LIB}FFI.modulemap" headers/module.modulemap

echo "==> creating TheoremHarnessFFI.xcframework"
rm -rf TheoremHarnessFFI.xcframework
xcodebuild -create-xcframework \
  -library "target/aarch64-apple-ios/release/${A}" -headers headers \
  -library "target/aarch64-apple-ios-sim/release/${A}" -headers headers \
  -output TheoremHarnessFFI.xcframework

echo "==> staging generated Swift API into the SwiftPM source target"
rm -rf Sources/TheoremHarness && mkdir -p Sources/TheoremHarness
cp "generated/${LIB}.swift" Sources/TheoremHarness/

echo "done: TheoremHarnessFFI.xcframework + Sources/TheoremHarness/ ready for swift build"
