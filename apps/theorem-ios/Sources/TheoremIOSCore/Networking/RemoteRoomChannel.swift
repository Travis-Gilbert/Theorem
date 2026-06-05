import Foundation

/// The coordination room as a send/receive channel. The app is a member of the
/// room (docs/plans/coordination-room-push): it posts messages with a tap/hold
/// delivery and subscribes to the room's live event stream. Swappable like the
/// other stores: a sample channel offline, the harness-server-backed channel when
/// `-remote <url>` is set.
public protocol RoomChannel: Sendable {
    /// Post a message to the room. Tap -> `.passive` (leave a note), hold ->
    /// `.wake` (queue the agents). Returns the persisted message.
    func send(_ body: String, delivery: MessageDelivery, mentions: [String]) async throws -> RoomMessage
    /// The room's live message stream (harness-server SSE). Finishes when the
    /// connection closes; throws on transport/status failure.
    func stream() -> AsyncThrowingStream<RoomMessage, Error>
}

public extension RoomChannel {
    func send(_ body: String, delivery: MessageDelivery) async throws -> RoomMessage {
        try await send(body, delivery: delivery, mentions: [])
    }
}

/// Offline channel: echoes a sent message back locally and streams nothing.
/// Honest about there being no server - a sent message appears (the local echo)
/// but no peer activity is invented, and `stream()` finishes immediately rather
/// than faking a live feed.
public struct SampleRoomChannel: RoomChannel {
    public let actorID: String

    public init(actorID: String = "travis") {
        self.actorID = actorID
    }

    public func send(
        _ body: String,
        delivery: MessageDelivery,
        mentions: [String]
    ) async throws -> RoomMessage {
        RoomMessage(
            messageID: "local-\(UInt64(Date().timeIntervalSince1970 * 1000))",
            author: actorID,
            body: body,
            delivery: delivery,
            urgency: delivery == .wake ? "ask" : "info",
            mentions: mentions,
            createdAt: ""
        )
    }

    public func stream() -> AsyncThrowingStream<RoomMessage, Error> {
        AsyncThrowingStream { $0.finish() }
    }
}

/// Request body for the room write endpoint (the single send button's payload).
/// Encodes to the harness-server `MessagePost` shape; `delivery` serializes as
/// "passive"/"wake".
private struct SendMessageRequest: Encodable {
    let tenantSlug: String
    let actorID: String
    let message: String
    let delivery: MessageDelivery
    let mentions: [String]

    enum CodingKeys: String, CodingKey {
        case tenantSlug = "tenant_slug"
        case actorID = "actor_id"
        case message
        case delivery
        case mentions
    }
}

/// A `RoomChannel` backed by theorem-harness-server's push endpoints:
///   POST /harness/rooms/{room}/messages  -> write a message + emit (the send button)
///   GET  /harness/rooms/{room}/stream     -> SSE of this room's live messages
/// Reuses `HarnessRunStoreError` so transport/status/decoding failures surface the
/// same way as the runs and presence paths.
public struct RemoteRoomChannel: RoomChannel {
    public let baseURL: URL
    public let roomID: String
    public let tenantSlug: String
    public let actorID: String
    private let session: URLSession

    public init(
        baseURL: URL,
        roomID: String = "default",
        tenantSlug: String = "default",
        actorID: String = "travis",
        session: URLSession = .shared
    ) {
        self.baseURL = baseURL
        self.roomID = roomID
        self.tenantSlug = tenantSlug
        self.actorID = actorID
        self.session = session
    }

    public func send(
        _ body: String,
        delivery: MessageDelivery,
        mentions: [String]
    ) async throws -> RoomMessage {
        let url = baseURL.appendingPathComponent("harness/rooms/\(roomID)/messages")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload = SendMessageRequest(
            tenantSlug: tenantSlug,
            actorID: actorID,
            message: body,
            delivery: delivery,
            mentions: mentions
        )
        do {
            request.httpBody = try JSONEncoder().encode(payload)
        } catch {
            throw HarnessRunStoreError.decoding(String(describing: error))
        }

        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw HarnessRunStoreError.transport("No HTTP response")
        }
        guard (200..<300).contains(http.statusCode) else {
            throw HarnessRunStoreError.status(http.statusCode)
        }
        do {
            let decoded = try JSONDecoder().decode(SendMessageResponseDTO.self, from: data)
            return RoomMessage(dto: decoded.message)
        } catch {
            throw HarnessRunStoreError.decoding(String(describing: error))
        }
    }

    public func stream() -> AsyncThrowingStream<RoomMessage, Error> {
        let url = baseURL.appendingPathComponent("harness/rooms/\(roomID)/stream")
        let session = self.session
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    var request = URLRequest(url: url)
                    request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    let (bytes, response) = try await session.bytes(for: request)
                    if let http = response as? HTTPURLResponse,
                       !(200..<300).contains(http.statusCode) {
                        throw HarnessRunStoreError.status(http.statusCode)
                    }
                    let decoder = JSONDecoder()
                    for try await line in bytes.lines {
                        guard let payload = Self.sseData(from: line) else { continue }
                        if let event = try? decoder.decode(
                            RoomMessageEventDTO.self,
                            from: Data(payload.utf8)
                        ) {
                            continuation.yield(RoomMessage(event: event))
                        }
                    }
                    continuation.finish()
                } catch is CancellationError {
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    /// Extract the JSON payload from an SSE `data:` line. Field lines other than
    /// `data:` (the `event:` name, blank separators, `:` comments) yield nil.
    /// Each push event carries exactly one `data:` line, so per-line decoding is
    /// sufficient. Internal for unit testing.
    static func sseData(from line: String) -> String? {
        guard line.hasPrefix("data:") else { return nil }
        let payload = line.dropFirst("data:".count)
        let trimmed = payload.hasPrefix(" ") ? payload.dropFirst() : payload
        return trimmed.isEmpty ? nil : String(trimmed)
    }
}
