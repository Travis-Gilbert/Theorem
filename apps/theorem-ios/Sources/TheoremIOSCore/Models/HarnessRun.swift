import Foundation

/// The run as the kernel object (harness UI spec, Part 2): a run is a governed
/// state machine with content-addressed state, not a chat transcript. This model
/// mirrors the Rust kernel's run contract (`theorem-harness-core`): an ordered,
/// guard-enforced event stream where every transition carries a status and a
/// `state_hash_after`, plus the readouts the run-detail rails render (the context
/// ledger and the recorded outcome).
///
/// It is a pure value type with no backend coupling. Today it is fed a recorded
/// reference run (see `SampleRun`, transcribed from the parity corpus). When the
/// runtime crate persists live events, the same model is decoded from the run's
/// event log unchanged: the view does not know or care which source it came from.

public struct HarnessRunEvent: Identifiable, Equatable, Sendable {
    /// 1-based event sequence inside the run (the kernel's `last_event_seq`).
    public let seq: Int
    /// The transition type, e.g. `CONTEXT.PACKED`, `OUTCOME.RECORDED`.
    public let type: String
    /// The run status after the transition, e.g. `context_packed`, `closed`.
    public let status: String
    /// The content-addressed run-state digest after the transition.
    public let stateHashAfter: String

    public var id: Int { seq }

    public init(seq: Int, type: String, status: String, stateHashAfter: String) {
        self.seq = seq
        self.type = type
        self.status = status
        self.stateHashAfter = stateHashAfter
    }

    /// A human label for the status (the kernel status is snake_case).
    public var statusLabel: String {
        status.replacingOccurrences(of: "_", with: " ").uppercased()
    }
}

/// The context artifact's token accounting (the Cost rail's data: the budget
/// governor made visible). Mirrors the kernel `CONTEXT.PACKED` payload.
public struct HarnessRunLedger: Equatable, Sendable {
    public let artifactID: String
    public let budgetTokens: Int
    public let capsuleTokens: Int
    public let includedAtoms: Int
    public let excludedAtoms: Int
    public let savedTokens: Int

    public init(
        artifactID: String,
        budgetTokens: Int,
        capsuleTokens: Int,
        includedAtoms: Int,
        excludedAtoms: Int,
        savedTokens: Int
    ) {
        self.artifactID = artifactID
        self.budgetTokens = budgetTokens
        self.capsuleTokens = capsuleTokens
        self.includedAtoms = includedAtoms
        self.excludedAtoms = excludedAtoms
        self.savedTokens = savedTokens
    }

    /// Fraction of the token budget the packed capsule used (0...1).
    public var budgetFraction: Double {
        guard budgetTokens > 0 else { return 0 }
        return min(1, Double(capsuleTokens) / Double(budgetTokens))
    }
}

/// A single validator result inside the outcome.
public struct HarnessRunValidator: Identifiable, Equatable, Sendable {
    public let id: String
    public let status: String
    public init(id: String, status: String) {
        self.id = id
        self.status = status
    }
    public var passed: Bool { status.lowercased() == "passed" }
}

/// What the run changed and what it learned (the Outcome rail's data). Mirrors the
/// kernel `OUTCOME.RECORDED` payload.
public struct HarnessRunOutcome: Equatable, Sendable {
    public let accepted: Bool
    public let testsPassed: Bool
    public let filesChanged: [String]
    public let validators: [HarnessRunValidator]
    public let summary: String

    public init(
        accepted: Bool,
        testsPassed: Bool,
        filesChanged: [String],
        validators: [HarnessRunValidator],
        summary: String
    ) {
        self.accepted = accepted
        self.testsPassed = testsPassed
        self.filesChanged = filesChanged
        self.validators = validators
        self.summary = summary
    }
}

public struct HarnessRun: Identifiable, Equatable, Sendable {
    public let runID: String
    public let task: String
    public let actor: String
    public let events: [HarnessRunEvent]
    public let ledger: HarnessRunLedger?
    public let outcome: HarnessRunOutcome?

    public var id: String { runID }

    public init(
        runID: String,
        task: String,
        actor: String,
        events: [HarnessRunEvent],
        ledger: HarnessRunLedger?,
        outcome: HarnessRunOutcome?
    ) {
        self.runID = runID
        self.task = task
        self.actor = actor
        self.events = events
        self.ledger = ledger
        self.outcome = outcome
    }

    /// The run's current status (the last event's status, or `created`).
    public var status: String { events.last?.status ?? "created" }

    /// The kernel treats closed/failed/cancelled as terminal.
    public var isTerminal: Bool {
        ["closed", "failed", "cancelled"].contains(status)
    }

    /// The content-addressed digest of the final run state.
    public var finalStateHash: String { events.last?.stateHashAfter ?? "" }
}
