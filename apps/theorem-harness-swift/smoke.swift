// Swift smoke for the theorem-harness Swift (UniFFI) binding.
//
// Proves the round-trip Swift -> generated bindings -> Rust SDK -> RedCore -> Swift,
// the same lifecycle the Node smoke exercises, against the SAME Rust core. Build:
//   cargo build
//   cargo run --bin uniffi-bindgen -- generate \
//     --library target/debug/libtheorem_harness_swift.dylib --language swift --out-dir generated
//   swiftc -parse-as-library -emit-executable -o smoke_swift \
//     -L target/debug -ltheorem_harness_swift \
//     -Xcc -fmodule-map-file=generated/theorem_harness_swiftFFI.modulemap -I generated \
//     generated/theorem_harness_swift.swift smoke.swift
//   DYLD_LIBRARY_PATH=target/debug ./smoke_swift

import Foundation

func kinds(_ json: String) -> [String] {
    guard let data = json.data(using: .utf8),
        let arr = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]]
    else { return [] }
    return arr.map { ($0["kind"] as? String) ?? "?" }
}

@main
struct Smoke {
    static func main() {
        let dataDir = (NSTemporaryDirectory() as NSString)
            .appendingPathComponent("theorem-harness-swift-" + UUID().uuidString)
        try? FileManager.default.createDirectory(atPath: dataDir, withIntermediateDirectories: true)
        print("data dir:", dataDir)

        do {
            let harness = try Harness(dataDir: dataDir)

            let runId = try harness.startRun(
                task: "demo from swift", actor: "swift-smoke", idempotencyKey: "k-create")
            print("started run:", runId)

            let afterStart = kinds(try harness.eventsJson(runId: runId))
            print("after start:", afterStart)

            try harness.cancel(
                runId: runId, reason: "stopping from swift", idempotencyKey: "k-cancel")
            let afterCancel = kinds(try harness.eventsJson(runId: runId))
            print("after cancel:", afterCancel)

            let status = try harness.runStatus(runId: runId)
            print("status:", status)

            _ = try harness.remember(
                agentId: "swift-smoke", kind: "belief", title: "binding is durable",
                content: "The Swift binding persists to RedCore.")
            let recalledJson = try harness.recall(agentId: "swift-smoke", query: "binding", limit: 10)
            let recalledCount =
                ((try? JSONSerialization.jsonObject(with: recalledJson.data(using: .utf8)!)) as? [Any])?
                .count ?? 0
            print("recalled:", recalledCount)

            let ok =
                afterStart == ["Created"] && afterCancel == ["Created", "Cancelled"]
                && status == "cancelled" && recalledCount >= 1
            print(ok ? "SMOKE PASS" : "SMOKE FAIL")
            exit(ok ? 0 : 1)
        } catch {
            print("SMOKE FAIL:", error)
            exit(1)
        }
    }
}
