import XCTest
@testable import TheoremIOSCore

/// Verifies the Swift client decodes the runtime's presence contract
/// (theorem-harness-runtime: CoordinationPresenceState, wrapped as
/// `{ "presence": [...], "count": N }`) and joins it with the known roster into
/// live `Participant` rows. The JSON below matches that contract; final
/// byte-parity against a real runtime response is a follow-up once the transport
/// ships in production.
final class ParticipantPresenceDecodeTests: XCTestCase {
    private let payload = """
    {
      "tenant": "default",
      "count": 2,
      "presence": [
        { "tenant_slug": "default", "actor_id": "codex", "status": "active",
          "surface": "Runs", "branch": "main", "changed_files": ["a.rs", "b.rs"],
          "refreshed_at": "2026-06-01T16:01:00Z", "expires_at": "2026-06-01T16:01:00Z",
          "ttl_seconds": 120 },
        { "tenant_slug": "default", "actor_id": "claude-code", "status": "working",
          "surface": "", "branch": "", "changed_files": [],
          "refreshed_at": "2026-06-01T16:02:00Z", "ttl_seconds": 120 }
      ]
    }
    """

    func testDecodesPresenceContract() throws {
        let response = try JSONDecoder().decode(PresenceListResponse.self, from: Data(payload.utf8))
        XCTAssertEqual(response.count, 2)
        XCTAssertEqual(response.presence.count, 2)
        XCTAssertEqual(response.presence.first?.actorID, "codex")
        XCTAssertEqual(response.presence.first?.status, "active")
        XCTAssertEqual(response.presence.first?.changedFiles, ["a.rs", "b.rs"])
    }

    func testJoinOverlaysLiveStatusOntoRoster() throws {
        let response = try JSONDecoder().decode(PresenceListResponse.self, from: Data(payload.utf8))
        let merged = ParticipantPresenceJoin.merge(
            roster: SampleRoster.participants,
            presence: response.presence
        )

        // codex matches by exact id -> engaged, live note from presence detail.
        let codex = try XCTUnwrap(merged.first { $0.id == "codex" })
        XCTAssertEqual(codex.status, .engaged)
        XCTAssertTrue(codex.note.contains("on Runs"))
        XCTAssertTrue(codex.note.contains("main"))
        XCTAssertTrue(codex.note.contains("2 files touched"))

        // claude-code matches roster "claude" by the delimiter rule -> contributing.
        // No live detail, so the honest fallback is the roster's static note.
        let claude = try XCTUnwrap(merged.first { $0.id == "claude" })
        XCTAssertEqual(claude.status, .contributing)
        let rosterClaudeNote = SampleRoster.participants.first { $0.id == "claude" }?.note
        XCTAssertEqual(claude.note, rosterClaudeNote)

        // Roster members with no presence keep their honest idle default.
        XCTAssertEqual(merged.first { $0.id == "deepseek" }?.status, .idle)
        XCTAssertEqual(merged.first { $0.id == "byo" }?.status, .idle)
    }

    func testPresenceActorNotInRosterIsAppendedAsDiscovered() {
        let entries = [
            PresenceEntryDTOFixture.make(actorID: "gemini-cli", status: "active", surface: "Maps")
        ]
        let merged = ParticipantPresenceJoin.merge(
            roster: SampleRoster.participants,
            presence: entries
        )
        let discovered = merged.first { $0.id == "gemini-cli" }
        XCTAssertNotNil(discovered)
        XCTAssertEqual(discovered?.name, "gemini-cli")
        XCTAssertEqual(discovered?.kind, .roster)
        XCTAssertEqual(discovered?.status, .engaged)
        XCTAssertEqual(discovered?.note, "on Maps")
    }

    func testStatusMappingReflectsServerVerbatim() {
        XCTAssertEqual(PresenceEntryDTOFixture.make(status: "active").participantStatus, .engaged)
        XCTAssertEqual(PresenceEntryDTOFixture.make(status: "working").participantStatus, .contributing)
        XCTAssertEqual(PresenceEntryDTOFixture.make(status: "inactive").participantStatus, .idle)
        XCTAssertEqual(PresenceEntryDTOFixture.make(status: "").participantStatus, .idle)
        XCTAssertEqual(PresenceEntryDTOFixture.make(status: "unreachable").participantStatus, .unreachable)
        // Unknown status is reported as idle, never invented as active.
        XCTAssertEqual(PresenceEntryDTOFixture.make(status: "bogus").participantStatus, .idle)
    }

    func testActorMatchRuleIsDeterministic() {
        XCTAssertTrue(ParticipantPresenceJoin.actorMatches(participantID: "codex", actorID: "codex"))
        XCTAssertTrue(ParticipantPresenceJoin.actorMatches(participantID: "claude", actorID: "claude-code"))
        XCTAssertTrue(ParticipantPresenceJoin.actorMatches(participantID: "claude", actorID: "Claude"))
        // A shared prefix that is not a delimiter boundary must not match.
        XCTAssertFalse(ParticipantPresenceJoin.actorMatches(participantID: "deep", actorID: "deepseek"))
        XCTAssertFalse(ParticipantPresenceJoin.actorMatches(participantID: "claude", actorID: "claudia"))
    }
}

/// Builds `PresenceEntryDTO` values for tests by routing through the real decoder,
/// since the DTO's initializer is the synthesized `Decodable` one.
private enum PresenceEntryDTOFixture {
    static func make(
        actorID: String = "actor",
        status: String = "active",
        surface: String? = nil
    ) -> PresenceEntryDTO {
        var fields: [String] = [
            "\"actor_id\": \(quoted(actorID))",
            "\"status\": \(quoted(status))"
        ]
        if let surface { fields.append("\"surface\": \(quoted(surface))") }
        let json = "{ \(fields.joined(separator: ", ")) }"
        // Force-try is acceptable in a test fixture over a fixed, valid shape.
        return try! JSONDecoder().decode(PresenceEntryDTO.self, from: Data(json.utf8))
    }

    private static func quoted(_ value: String) -> String {
        "\"\(value)\""
    }
}
