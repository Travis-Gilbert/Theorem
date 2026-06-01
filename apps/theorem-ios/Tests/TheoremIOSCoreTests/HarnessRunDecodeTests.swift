import XCTest
@testable import TheoremIOSCore

/// Verifies the Swift client decodes the runtime's run/event JSON contract
/// (theorem-harness-runtime: RunState + EventState serde shapes) into the UI's
/// HarnessRun. The JSON below matches that contract; final byte-parity against a
/// real runtime response is a follow-up once the transport ships.
final class HarnessRunDecodeTests: XCTestCase {
    func testDecodesRuntimeContractIntoHarnessRun() throws {
        let json = """
        {
          "run": {
            "run_id": "run-fixture-0001",
            "task": "port harness to rust",
            "actor": "claude-code",
            "status": "closed",
            "last_event_seq": 3
          },
          "events": [
            { "seq": 1, "type": "RUN.CREATED", "state_hash_after": "aaaa", "payload": {} },
            { "seq": 2, "type": "CONTEXT.PACKED", "state_hash_after": "bbbb",
              "payload": {
                "artifact_id": "art-1", "budget_tokens": 1000, "capsule_tokens": 200,
                "included_atom_count": 5, "excluded_atom_count": 2,
                "token_ledger": { "saved": 300 }
              } },
            { "seq": 3, "type": "OUTCOME.RECORDED", "state_hash_after": "cccc",
              "payload": {
                "accepted": true, "tests_passed": true,
                "files_changed": ["state_machine.rs"],
                "validator_results": [ { "id": "v1", "status": "passed" } ],
                "summary": "ported"
              } },
            { "seq": 4, "type": "RUN.CLOSED", "state_hash_after": "dddd",
              "payload": { "summary": "done", "closed_by": "claude-code" } }
          ]
        }
        """
        let response = try JSONDecoder().decode(RunDetailResponse.self, from: Data(json.utf8))
        let run = response.toHarnessRun()

        XCTAssertEqual(run.runID, "run-fixture-0001")
        XCTAssertEqual(run.task, "port harness to rust")
        XCTAssertEqual(run.actor, "claude-code")
        XCTAssertEqual(run.events.count, 4)

        // Status derived from event type when the server does not attach it.
        XCTAssertEqual(run.events.first?.status, "created")
        XCTAssertEqual(run.events.last?.status, "closed")
        XCTAssertEqual(run.status, "closed")
        XCTAssertTrue(run.isTerminal)

        let ledger = try XCTUnwrap(run.ledger)
        XCTAssertEqual(ledger.artifactID, "art-1")
        XCTAssertEqual(ledger.budgetTokens, 1000)
        XCTAssertEqual(ledger.capsuleTokens, 200)
        XCTAssertEqual(ledger.includedAtoms, 5)
        XCTAssertEqual(ledger.excludedAtoms, 2)
        XCTAssertEqual(ledger.savedTokens, 300)

        let outcome = try XCTUnwrap(run.outcome)
        XCTAssertTrue(outcome.accepted)
        XCTAssertTrue(outcome.testsPassed)
        XCTAssertEqual(outcome.filesChanged, ["state_machine.rs"])
        XCTAssertEqual(outcome.validators.count, 1)
        XCTAssertEqual(outcome.validators.first?.id, "v1")
        XCTAssertTrue(outcome.validators.first?.passed ?? false)
        XCTAssertEqual(outcome.summary, "ported")

        // The trace export round-trips the decoded events.
        let jsonl = run.traceJSONL()
        XCTAssertEqual(jsonl.split(separator: "\n").count, 4)
        XCTAssertTrue(jsonl.contains("\"type\":\"CONTEXT.PACKED\""))
        XCTAssertTrue(jsonl.contains("\"state_hash_after\":\"cccc\""))
    }

    func testServerAttachedStatusIsPreferredOverDerivation() throws {
        let json = """
        { "run": { "run_id": "r", "task": "t", "actor": "a", "status": "agent_acting" },
          "events": [ { "seq": 1, "type": "AGENT.ACTING", "status": "live_override",
                        "state_hash_after": "h", "payload": {} } ] }
        """
        let response = try JSONDecoder().decode(RunDetailResponse.self, from: Data(json.utf8))
        let run = response.toHarnessRun()
        XCTAssertEqual(run.events.first?.status, "live_override")
    }

    func testStatusMapMirrorsKernelLifecycle() {
        XCTAssertEqual(HarnessStatusMap.status(forEventType: "CONTEXT.PACKED"), "context_packed")
        XCTAssertEqual(HarnessStatusMap.status(forEventType: "FEDERATION.SIGNAL_PREPARED"), "federation_signal_prepared")
        XCTAssertEqual(HarnessStatusMap.status(forEventType: "RUN.CLOSED"), "closed")
        // Unknown types fall back to a snake_case of the type.
        XCTAssertEqual(HarnessStatusMap.status(forEventType: "CUSTOM.THING"), "custom_thing")
    }
}
