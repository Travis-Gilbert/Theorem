import Foundation

/// Decoders for the runtime's run/event JSON contract
/// (`theorem-harness-runtime`): the run is persisted as a `HarnessRun`-labelled
/// node (the `RunState` serde shape + `state_hash`) and an ordered list of
/// `HarnessEvent` nodes (the `EventState` serde shape). These DTOs decode that
/// contract and map it to the UI's `HarnessRun`, so a `RemoteHarnessRunStore`
/// renders live runs through the exact same surfaces as the recorded sample.
///
/// See docs/plans/harness-rust-port/ios-transport-handoff.md for the endpoints.

/// `GET /harness/runs` -> a list of run nodes (most recent first).
public struct RunsListResponse: Decodable, Sendable {
    public let runs: [RunNodeDTO]
}

/// `GET /harness/runs/{run_id}` -> one run plus its ordered event log.
public struct RunDetailResponse: Decodable, Sendable {
    public let run: RunNodeDTO
    public let events: [EventNodeDTO]
}

public struct RunNodeDTO: Decodable, Sendable {
    public let runID: String
    public let task: String
    public let actor: String
    public let status: String
    public let lastEventSeq: Int?

    enum CodingKeys: String, CodingKey {
        case runID = "run_id"
        case task
        case actor
        case status
        case lastEventSeq = "last_event_seq"
    }
}

public struct EventNodeDTO: Decodable, Sendable {
    public let seq: Int
    public let type: String
    /// Optional: the runtime may attach the post-transition status server-side
    /// (preferred). When absent, the status is derived from the event type.
    public let status: String?
    public let stateHashAfter: String
    public let payload: JSONValue?

    enum CodingKeys: String, CodingKey {
        case seq
        case type
        case status
        case stateHashAfter = "state_hash_after"
        case payload
    }
}

public extension RunDetailResponse {
    /// Map the runtime contract to the UI's `HarnessRun`. The timeline comes from
    /// the events; the ledger and outcome are derived from the `CONTEXT.PACKED`
    /// and `OUTCOME.RECORDED` event payloads. Per-atom context provenance is a
    /// separate artifact and is left empty here (the Evidence rail degrades to
    /// counts, which is honest).
    func toHarnessRun() -> HarnessRun {
        let runEvents = events.map { event in
            HarnessRunEvent(
                seq: event.seq,
                type: event.type,
                status: event.status ?? HarnessStatusMap.status(forEventType: event.type),
                stateHashAfter: event.stateHashAfter
            )
        }
        let ledger = events
            .first { $0.type == "CONTEXT.PACKED" }?.payload
            .map(RunDetailResponse.ledger(from:))
        let outcome = events
            .first { $0.type == "OUTCOME.RECORDED" }?.payload
            .map(RunDetailResponse.outcome(from:))
        return HarnessRun(
            runID: run.runID,
            task: run.task,
            actor: run.actor,
            events: runEvents,
            ledger: ledger,
            outcome: outcome
        )
    }

    static func ledger(from payload: JSONValue) -> HarnessRunLedger {
        HarnessRunLedger(
            artifactID: payload["artifact_id"]?.stringValue ?? "",
            budgetTokens: payload["budget_tokens"]?.intValue ?? 0,
            capsuleTokens: payload["capsule_tokens"]?.intValue ?? 0,
            includedAtoms: payload["included_atom_count"]?.intValue ?? 0,
            excludedAtoms: payload["excluded_atom_count"]?.intValue ?? 0,
            savedTokens: payload["token_ledger"]?["saved"]?.intValue ?? 0
        )
    }

    static func outcome(from payload: JSONValue) -> HarnessRunOutcome {
        let files = (payload["files_changed"]?.arrayValue ?? []).compactMap { $0.stringValue }
        let validators = (payload["validator_results"]?.arrayValue ?? []).compactMap { value -> HarnessRunValidator? in
            guard let id = value["id"]?.stringValue else { return nil }
            return HarnessRunValidator(id: id, status: value["status"]?.stringValue ?? "")
        }
        return HarnessRunOutcome(
            accepted: payload["accepted"]?.boolValue ?? false,
            testsPassed: payload["tests_passed"]?.boolValue ?? false,
            filesChanged: files,
            validators: validators,
            summary: payload["summary"]?.stringValue ?? ""
        )
    }
}

/// Mirrors the kernel's event-type -> post-transition status. Used only as a
/// fallback when the runtime does not attach `status` per event. Kept in sync
/// with theorem-harness-core's transition table; prefer server-attached status.
public enum HarnessStatusMap {
    public static func status(forEventType type: String) -> String {
        switch type {
        case "RUN.CREATED": "created"
        case "HOST.OBSERVED": "observed"
        case "TASK.RESOLVED": "resolved"
        case "PROFILE.SELECTED": "profile_selected"
        case "DOMAIN.RESOLVED": "domain_resolved"
        case "TOOLKIT.COMPILED": "toolkit_compiled"
        case "TOOLPACK.COMPILED": "toolpack_compiled"
        case "MAPS.LOADED": "maps_loaded"
        case "CONTEXT.PLANNED": "context_planned"
        case "CONTEXT.PACKED": "context_packed"
        case "CONTEXT.COMPILED": "context_compiled"
        case "CONTEXT.INJECTED": "context_injected"
        case "AGENT.ACTING": "agent_acting"
        case "VALIDATION.STARTED", "VALIDATION.RUNNING": "validating"
        case "OUTCOME.RECORDED": "outcome_recorded"
        case "LEARNING.PROPOSED": "learning_proposed"
        case "MEMORY.PATCHED": "memory_patched"
        case "MAPS.UPDATED": "maps_updated"
        case "REVIEW.QUEUED": "review_queued"
        case "FEDERATION.SIGNAL_PREPARED": "federation_signal_prepared"
        case "RUN.CLOSED": "closed"
        case "RUN.FAILED": "failed"
        case "RUN.CANCELLED": "cancelled"
        default: type.lowercased().replacingOccurrences(of: ".", with: "_")
        }
    }
}

// Object / bool / array accessors JSONValue does not expose publicly. Scoped to
// this file so the shared JSONValue surface stays minimal.
fileprivate extension JSONValue {
    subscript(key: String) -> JSONValue? {
        if case let .object(dict) = self { return dict[key] }
        return nil
    }

    var boolValue: Bool? {
        if case let .bool(value) = self { return value }
        return nil
    }

    var arrayValue: [JSONValue]? {
        if case let .array(value) = self { return value }
        return nil
    }
}
