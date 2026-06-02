import Foundation

/// Decoder for the runtime's presence contract
/// (`GET /harness/rooms/{room}/presence` -> theorem-harness-runtime's
/// `CoordinationPresenceState`, wrapped as `{ "presence": [...], "count": N }`).
///
/// Presence carries live actor STATUS only. Participant IDENTITY (name, kind,
/// access, note) comes from the known roster, because how Claude / Codex /
/// DeepSeek / a brought agent connect is a real architectural fact (Part 7) the
/// presence feed does not know. `ParticipantPresenceJoin` joins the two:
/// roster identity x live status. See
/// docs/plans/harness-rust-port/ios-transport-handoff.md.

public struct PresenceListResponse: Decodable, Sendable {
    public let presence: [PresenceEntryDTO]
    public let count: Int
}

public struct PresenceEntryDTO: Decodable, Sendable {
    public let actorID: String
    public let status: String
    public let surface: String?
    public let branch: String?
    public let changedFiles: [String]?

    enum CodingKeys: String, CodingKey {
        case actorID = "actor_id"
        case status
        case surface
        case branch
        case changedFiles = "changed_files"
    }
}

public extension PresenceEntryDTO {
    /// Map the runtime presence status string to the UI's honest live status.
    /// Reflects the server's `status` verbatim. It deliberately does NOT derive
    /// freshness from the presence timestamps: the runtime sets
    /// `expires_at == refreshed_at` when expiry is unset (coordination.rs
    /// heartbeat_presence), so those fields are not a reliable freshness signal,
    /// and inferring "idle" from them would manufacture a false status.
    var participantStatus: ParticipantStatus {
        switch status.lowercased() {
        case "active", "engaged": .engaged
        case "working", "contributing": .contributing
        case "inactive", "idle", "": .idle
        case "unreachable", "gone", "lost": .unreachable
        default: .idle
        }
    }

    /// An honest one-line live descriptor built only from what presence reports,
    /// e.g. "on Runs · main · 3 files touched". `nil` when presence carries no
    /// detail, so the caller can fall back to the roster's static note.
    var liveNote: String? {
        var parts: [String] = []
        if let surface, !surface.isEmpty { parts.append("on \(surface)") }
        if let branch, !branch.isEmpty { parts.append(branch) }
        if let changedFiles, !changedFiles.isEmpty {
            let noun = changedFiles.count == 1 ? "file" : "files"
            parts.append("\(changedFiles.count) \(noun) touched")
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}

/// Join a known roster (identity + access model, Part 7) with live presence
/// (status). Roster participants get their live status overlaid; presence actors
/// with no roster identity are appended as honestly-minimal discovered peers, so
/// the surface neither hides a real participant nor fabricates one. Pure function:
/// unit-testable without a live server.
public enum ParticipantPresenceJoin {
    public static func merge(
        roster: [Participant],
        presence: [PresenceEntryDTO]
    ) -> [Participant] {
        var result: [Participant] = []
        var consumed: Set<String> = []

        // Each roster member adopts the first presence entry that identifies it,
        // overlaying live status; otherwise it keeps its honest idle default.
        for participant in roster {
            if let entry = presence.first(where: {
                !consumed.contains($0.actorID.lowercased())
                    && actorMatches(participantID: participant.id, actorID: $0.actorID)
            }) {
                consumed.insert(entry.actorID.lowercased())
                result.append(
                    Participant(
                        id: participant.id,
                        name: participant.name,
                        kind: participant.kind,
                        access: participant.access,
                        status: entry.participantStatus,
                        note: entry.liveNote ?? participant.note
                    )
                )
            } else {
                result.append(participant)
            }
        }

        // Presence actors not adopted by any roster member: real participants the
        // roster does not describe. Append them with honestly-minimal metadata.
        for entry in presence where !consumed.contains(entry.actorID.lowercased()) {
            consumed.insert(entry.actorID.lowercased())
            result.append(
                Participant(
                    id: entry.actorID,
                    name: entry.actorID,
                    kind: .roster,
                    access: .mediated,
                    status: entry.participantStatus,
                    note: entry.liveNote ?? "Seen in room presence."
                )
            )
        }

        return result
    }

    /// A roster id matches an actor id when they are equal (case-insensitive) or
    /// the actor id is a delimiter-separated specialization of it (e.g. "claude"
    /// matches "claude-code", "codex" matches "codex"). Deterministic, no fuzzy
    /// matching, so the join is explainable and testable.
    static func actorMatches(participantID: String, actorID: String) -> Bool {
        let participant = participantID.lowercased()
        let actor = actorID.lowercased()
        if participant == actor { return true }
        let head = actor.split(whereSeparator: { "-_.: /".contains($0) }).first.map(String.init)
        return head == participant
    }
}
