import Foundation

/// A participant in the room (harness UI spec, Part 4: "a team working, not a
/// menu"). Participants are addressed collectively through Ask; the router engages
/// them by capability prior, not by the user picking a model. The one exception is
/// a brought agent, which the user supplied and can address directly.
///
/// This models the participant ROSTER and how each connects (the three access
/// modes from Part 7), not live activity. Live status (engaged, contributing)
/// requires an active run from the runtime; until then the honest state is idle.

public enum ParticipantKind: String, Sendable {
    /// A hosted frontier agent reached through the user's own authenticated
    /// session (Claude, Codex).
    case visiting
    /// The user's own model endpoint (bring-your-own).
    case brought
    /// A standing roster peer the harness provides (e.g. the key-less default).
    case roster

    public var label: String {
        switch self {
        case .visiting: "visiting"
        case .brought: "brought"
        case .roster: "roster"
        }
    }
}

/// The three access modes, honestly labeled (Part 7). Determines how directly the
/// harness can reach the participant and what trace tier it can offer.
public enum ParticipantAccess: String, Sendable {
    /// Hosted models reached through hooks / MCP / tool results. Observable,
    /// rationale, output-projected trace.
    case mediated
    /// Local / open models reached directly with compiled context in tighter
    /// loops. Adds local scratchpad to the trace.
    case resident
    /// Future substrate-native models reading affordances during inference.
    /// Hidden-state / thought-vector trace; research layer.
    case fused

    public var label: String {
        switch self {
        case .mediated: "mediated"
        case .resident: "resident"
        case .fused: "fused"
        }
    }
}

/// The honest live state. Without an active run the team is idle; the richer
/// states arrive with the runtime's event stream.
public enum ParticipantStatus: String, Sendable {
    case idle
    case engaged
    case contributing
    case unreachable

    public var label: String { rawValue }
}

public struct Participant: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let kind: ParticipantKind
    public let access: ParticipantAccess
    public let status: ParticipantStatus
    /// An honest one-line descriptor of how this participant connects.
    public let note: String

    public init(
        id: String,
        name: String,
        kind: ParticipantKind,
        access: ParticipantAccess,
        status: ParticipantStatus,
        note: String
    ) {
        self.id = id
        self.name = name
        self.kind = kind
        self.access = access
        self.status = status
        self.note = note
    }
}
