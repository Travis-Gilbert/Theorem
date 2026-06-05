import XCTest
@testable import TheoremIOSCore

/// Verifies the Swift client decodes the harness-server push contracts
/// (theorem-harness-server: the POST `{ "message": ..., "event": ... }` response
/// and the SSE `RoomMessageEvent`), maps them to the UI `RoomMessage`, parses the
/// tap/hold `delivery` tolerantly, and reads SSE `data:` lines. The JSON matches
/// the runtime/server shapes; byte-parity against a live response is a follow-up.
final class RoomMessageDecodeTests: XCTestCase {
    func testDecodesSendResponseContract() throws {
        let payload = """
        {
          "message": {
            "tenant_slug": "smoke", "room_id": "r1", "message_id": "msg_1",
            "actor_id": "travis", "urgency": "ask", "delivery": "wake",
            "message": "@codex go", "mentions": ["codex"], "consumed_by": [],
            "metadata": {}, "created_at": "unix_ms:1"
          },
          "event": {
            "tenant_slug": "smoke", "room_id": "r1", "message_id": "msg_1",
            "author": "travis", "urgency": "ask", "message": "@codex go",
            "mentions": ["codex"], "delivery": "wake", "created_at": "unix_ms:1"
          }
        }
        """
        let decoded = try JSONDecoder().decode(SendMessageResponseDTO.self, from: Data(payload.utf8))
        XCTAssertEqual(decoded.message.messageID, "msg_1")
        XCTAssertEqual(decoded.message.actorID, "travis")
        XCTAssertEqual(decoded.message.delivery, "wake")
        XCTAssertEqual(decoded.event.author, "travis")

        let fromMessage = RoomMessage(dto: decoded.message)
        XCTAssertEqual(fromMessage.author, "travis")
        XCTAssertEqual(fromMessage.delivery, .wake)
        XCTAssertEqual(fromMessage.mentions, ["codex"])
        XCTAssertEqual(fromMessage.body, "@codex go")

        let fromEvent = RoomMessage(event: decoded.event)
        XCTAssertEqual(fromEvent.author, "travis")
        XCTAssertEqual(fromEvent.delivery, .wake)
        XCTAssertEqual(fromEvent.id, "msg_1")
    }

    func testDeliveryParseIsTolerant() {
        XCTAssertEqual(MessageDelivery.parse("wake"), .wake)
        XCTAssertEqual(MessageDelivery.parse("WAKE"), .wake)
        XCTAssertEqual(MessageDelivery.parse("passive"), .passive)
        // Unknown delivery reads as passive, never invented as a wake.
        XCTAssertEqual(MessageDelivery.parse("bogus"), .passive)
        XCTAssertEqual(MessageDelivery.parse(""), .passive)
    }

    func testMissingDeliveryDefaultsToPassive() throws {
        // An older server without the `delivery` field still decodes (tolerant),
        // reading as passive rather than failing the whole message.
        let payload = """
        {
          "tenant_slug": "smoke", "room_id": "r1", "message_id": "m2",
          "actor_id": "travis", "urgency": "info", "message": "note",
          "mentions": [], "created_at": ""
        }
        """
        let dto = try JSONDecoder().decode(RoomMessageDTO.self, from: Data(payload.utf8))
        XCTAssertNil(dto.delivery)
        XCTAssertEqual(RoomMessage(dto: dto).delivery, .passive)
    }

    func testSSEDataLineParsing() {
        XCTAssertEqual(RemoteRoomChannel.sseData(from: "data: {\"a\":1}"), "{\"a\":1}")
        XCTAssertEqual(RemoteRoomChannel.sseData(from: "data:{\"a\":1}"), "{\"a\":1}")
        // Non-data field lines, comments, and blanks carry no payload.
        XCTAssertNil(RemoteRoomChannel.sseData(from: "event: room_message"))
        XCTAssertNil(RemoteRoomChannel.sseData(from: ": keep-alive comment"))
        XCTAssertNil(RemoteRoomChannel.sseData(from: ""))
    }

    func testSampleChannelEchoesSendAndStreamsNothing() async throws {
        let channel = SampleRoomChannel(actorID: "travis")
        let wake = try await channel.send("@codex build", delivery: .wake)
        XCTAssertEqual(wake.author, "travis")
        XCTAssertEqual(wake.delivery, .wake)
        XCTAssertEqual(wake.urgency, "ask")

        // The offline stream finishes immediately - honest, no faked live feed.
        var received = 0
        for try await _ in channel.stream() { received += 1 }
        XCTAssertEqual(received, 0)
    }
}
