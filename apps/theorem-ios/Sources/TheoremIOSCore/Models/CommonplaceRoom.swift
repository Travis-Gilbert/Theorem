import Foundation

public struct CommonplaceRoom: Equatable, Sendable, Identifiable {
    public var id: String
    public var title: String
    public var mode: CommonplaceRoomMode
    public var ask: String
    public var participants: [CommonplaceParticipant]
    public var contributions: [CommonplaceContribution]
    public var scene: ScenePackageV2
    public var registry: CommonplaceRegistry?
    public var routePlan: CommonplaceRoutePlan?
    public var updatedAt: Date

    public init(
        id: String,
        title: String,
        mode: CommonplaceRoomMode = .ask,
        ask: String,
        participants: [CommonplaceParticipant],
        contributions: [CommonplaceContribution],
        scene: ScenePackageV2,
        registry: CommonplaceRegistry? = nil,
        routePlan: CommonplaceRoutePlan? = nil,
        updatedAt: Date = Date()
    ) {
        self.id = id
        self.title = title
        self.mode = mode
        self.ask = ask
        self.participants = participants
        self.contributions = contributions
        self.scene = scene
        self.registry = registry
        self.routePlan = routePlan
        self.updatedAt = updatedAt
    }

    public var engagedParticipants: [CommonplaceParticipant] {
        participants.filter { $0.status != .idle }
    }

    public func participant(for id: String?) -> CommonplaceParticipant? {
        guard let id else { return nil }
        return participants.first { $0.id == id }
    }

    public func applyingRoutePlan(_ routePlan: CommonplaceRoutePlan?) -> CommonplaceRoom {
        var next = self
        next.routePlan = routePlan
        next.participants = routeAwareParticipants(routePlan: routePlan)
        return next
    }

    public func replacingScene(_ scene: ScenePackageV2, ask: String) -> CommonplaceRoom {
        var next = self
        next.ask = ask
        next.scene = scene
        next.updatedAt = Date()
        next.routePlan = registry.map { CommonplaceRouter().plan(query: ask, registry: $0) }
        next.participants = routeAwareParticipants(routePlan: next.routePlan)
        let refreshID = "\(scene.id)-graph-refresh-\(Int(next.updatedAt.timeIntervalSince1970 * 1000))"
        next.contributions = Array((contributions + [
            CommonplaceContribution(
                id: refreshID,
                authorKind: .substrate,
                authorID: nil,
                state: .resolved,
                summary: "Graph refreshed",
                body: "RustyWeb returned \(scene.atoms.count) nodes and \(scene.relations.count) relations for \"\(ask)\"."
            )
        ]).suffix(6))
        return next
    }

    private func routeAwareParticipants(routePlan: CommonplaceRoutePlan?) -> [CommonplaceParticipant] {
        guard let routePlan else {
            return participants.map { participant in
                switch participant.id {
                case "codex":
                    participant.withStatus(.contributing)
                case "claude", "mistral-medium", "deepseek-v4-pro":
                    participant.withStatus(.thinking)
                default:
                    participant.withStatus(.idle)
                }
            }
        }
        let active = Set(routePlan.activeParticipantIDs)
        let planned = Set(routePlan.plannedParticipantIDs)
        return participants.map { participant in
            if active.contains(participant.id) {
                participant.withStatus(.contributing)
            } else if planned.contains(participant.id) {
                participant.withStatus(.thinking)
            } else {
                participant.withStatus(.idle)
            }
        }
    }
}

public enum CommonplaceRoomMode: String, Equatable, Sendable {
    case ask
    case addressBroughtAgent = "address_brought_agent"
}

public struct CommonplaceParticipant: Equatable, Sendable, Identifiable {
    public var id: String
    public var displayName: String
    public var shortName: String
    public var origin: CommonplaceParticipantOrigin
    public var status: CommonplaceParticipantStatus
    public var endpointLabel: String?

    public init(
        id: String,
        displayName: String,
        shortName: String,
        origin: CommonplaceParticipantOrigin,
        status: CommonplaceParticipantStatus = .idle,
        endpointLabel: String? = nil
    ) {
        self.id = id
        self.displayName = displayName
        self.shortName = shortName
        self.origin = origin
        self.status = status
        self.endpointLabel = endpointLabel
    }

    public var isDirectlyAddressable: Bool {
        origin == .brought
    }

    public func withStatus(_ status: CommonplaceParticipantStatus) -> CommonplaceParticipant {
        var next = self
        next.status = status
        return next
    }
}

public enum CommonplaceParticipantOrigin: String, Equatable, Sendable {
    case roster
    case visiting
    case brought
}

public enum CommonplaceParticipantStatus: String, Equatable, Sendable {
    case thinking
    case contributing
    case idle
}

public struct CommonplaceContribution: Equatable, Sendable, Identifiable {
    public var id: String
    public var authorKind: CommonplaceContributionAuthorKind
    public var authorID: String?
    public var state: CommonplaceContributionState
    public var summary: String
    public var body: String

    public init(
        id: String,
        authorKind: CommonplaceContributionAuthorKind,
        authorID: String?,
        state: CommonplaceContributionState,
        summary: String,
        body: String
    ) {
        self.id = id
        self.authorKind = authorKind
        self.authorID = authorID
        self.state = state
        self.summary = summary
        self.body = body
    }
}

public enum CommonplaceContributionAuthorKind: String, Equatable, Sendable {
    case human
    case participant
    case substrate
}

public enum CommonplaceContributionState: String, Equatable, Sendable {
    case thinking
    case contributing
    case resolved
}
