import Foundation

/// One step of the ACL local-push PPR walk: the algorithm dequeued `nodeID` and
/// captured `capturedMass` (= alpha * residual) at position `order` in the FIFO.
/// Each event is one animation frame for `fractal_expansion` (spec algo 4) — the
/// walk order IS the animation.
public struct PushTraceEvent: Codable, Sendable, Hashable, Identifiable {
    public let nodeID: String
    public let capturedMass: Double
    public let order: Int

    public var id: Int { order }

    enum CodingKeys: String, CodingKey {
        case nodeID = "node_id"
        case capturedMass = "captured_mass"
        case order
    }

    public init(nodeID: String, capturedMass: Double, order: Int) {
        self.nodeID = nodeID
        self.capturedMass = capturedMass
        self.order = order
    }
}

/// The streamed push-trace for a fractal-expansion animation.
///
/// This is a SEAM, like the UniFFI surface: the SERVER runs the real `push_ppr`
/// (spec — heavy walk stays server-side) and streams these events; the client
/// replays them as a wavefront over the base layout. The client never runs the
/// walk. `alpha`/`epsilon` are surfaced read-only in the UI (spec: user-tunable
/// alpha is a v2 nerd-knob). Field names are snake_case to match the server's
/// serde output.
public struct PushTrace: Codable, Sendable, Hashable {
    public let seedIDs: [String]
    public let events: [PushTraceEvent]
    public let alpha: Double
    public let epsilon: Double

    enum CodingKeys: String, CodingKey {
        case seedIDs = "seed_ids"
        case events
        case alpha
        case epsilon
    }

    public init(seedIDs: [String], events: [PushTraceEvent], alpha: Double = 0.15, epsilon: Double = 1e-4) {
        self.seedIDs = seedIDs
        self.events = events
        self.alpha = alpha
        self.epsilon = epsilon
    }

    /// Captured mass per node, summed across the walk (a node can be pushed more
    /// than once). Drives final node brightness/size.
    public func massByNode() -> [String: Double] {
        var out: [String: Double] = [:]
        for event in events {
            out[event.nodeID, default: 0] += event.capturedMass
        }
        return out
    }
}
