import Foundation

/// The tap/hold intent carried on a coordination message. Mirrors the runtime's
/// `delivery` field (theorem-harness-runtime `normalize_delivery`: passive|wake)
/// and the harness-server push `Delivery` enum. Tap leaves a note; hold queues
/// the agents.
public enum MessageDelivery: String, Codable, Sendable, Equatable {
    case passive
    case wake

    /// Tolerant parse: an unknown server value reads as passive (the safe "left a
    /// note" default), never invented as a wake. The Swift twin of the runtime's
    /// `Delivery::from_core`.
    public static func parse(_ raw: String) -> MessageDelivery {
        MessageDelivery(rawValue: raw.trimmingCharacters(in: .whitespaces).lowercased()) ?? .passive
    }
}

/// A coordination-room message as the UI renders it: who said it, the body, the
/// tap/hold intent, and who it addresses. Built from either the persisted message
/// shape (POST response, mentions feed) or the live SSE event.
public struct RoomMessage: Identifiable, Equatable, Sendable {
    public let messageID: String
    public let author: String
    public let body: String
    public let delivery: MessageDelivery
    public let urgency: String
    public let mentions: [String]
    public let createdAt: String

    public var id: String { messageID }

    public init(
        messageID: String,
        author: String,
        body: String,
        delivery: MessageDelivery,
        urgency: String,
        mentions: [String],
        createdAt: String
    ) {
        self.messageID = messageID
        self.author = author
        self.body = body
        self.delivery = delivery
        self.urgency = urgency
        self.mentions = mentions
        self.createdAt = createdAt
    }

    init(dto: RoomMessageDTO) {
        self.init(
            messageID: dto.messageID,
            author: dto.actorID,
            body: dto.message,
            delivery: .parse(dto.delivery ?? "passive"),
            urgency: dto.urgency,
            mentions: dto.mentions,
            createdAt: dto.createdAt
        )
    }

    init(event: RoomMessageEventDTO) {
        self.init(
            messageID: event.messageID,
            author: event.author,
            body: event.message,
            delivery: .parse(event.delivery ?? "passive"),
            urgency: event.urgency,
            mentions: event.mentions,
            createdAt: event.createdAt
        )
    }
}

/// Decoder for the persisted message shape (theorem-harness-runtime
/// `CoordinationMessageState`): the POST write response's `message` and the actor
/// mentions feed. Extra keys (consumed_by, metadata, tenant_slug) are ignored.
/// `delivery` is optional so an older server without the field still decodes
/// (it then reads as passive).
public struct RoomMessageDTO: Decodable, Sendable {
    public let messageID: String
    public let actorID: String
    public let message: String
    public let delivery: String?
    public let urgency: String
    public let mentions: [String]
    public let createdAt: String
    public let roomID: String

    enum CodingKeys: String, CodingKey {
        case messageID = "message_id"
        case actorID = "actor_id"
        case message
        case delivery
        case urgency
        case mentions
        case createdAt = "created_at"
        case roomID = "room_id"
    }
}

/// Decoder for the live push event (harness-server `RoomMessageEvent`, streamed
/// over SSE). Uses `author` where the persisted message uses `actor_id`.
public struct RoomMessageEventDTO: Decodable, Sendable {
    public let messageID: String
    public let author: String
    public let message: String
    public let delivery: String?
    public let urgency: String
    public let mentions: [String]
    public let createdAt: String
    public let roomID: String

    enum CodingKeys: String, CodingKey {
        case messageID = "message_id"
        case author
        case message
        case delivery
        case urgency
        case mentions
        case createdAt = "created_at"
        case roomID = "room_id"
    }
}

/// The POST write response shape: `{ "message": {...}, "event": {...} }`.
public struct SendMessageResponseDTO: Decodable, Sendable {
    public let message: RoomMessageDTO
    public let event: RoomMessageEventDTO
}
