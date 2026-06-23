import Foundation

/// The source of participants for the Participants surface. The UI reads the
/// roster through this protocol so the data source is swappable: a recorded
/// roster today, the runtime's live presence later, with no view change. Mirrors
/// `HarnessRunStore`.
///
/// The roster IDENTITY (name, kind, access, note) is a real architectural fact
/// (how Claude / Codex / DeepSeek / a brought agent connect, harness UI spec
/// Part 7), not invented activity. Only the live STATUS is stubbed until a run is
/// active. `RemoteParticipantStore` joins that identity with live presence from
/// the runtime, so the surface goes live without faking who is in the room.
public protocol ParticipantStore: Sendable {
    /// The roster with honest live status, ordered active-first.
    func participants() async throws -> [Participant]
}

/// The default source: the canonical roster with idle status. Honest about there
/// being no active run yet, so every participant reads idle rather than faking
/// "thinking" activity.
public struct SampleParticipantStore: ParticipantStore {
    public init() {}

    public func participants() async throws -> [Participant] {
        SampleRoster.participants
    }
}
