import Foundation

/// The participant roster (harness UI spec, Part 4). These are the real
/// participants the harness can engage and how each connects, grounded in the
/// architecture doc's access model (Part 7), not invented activity. Status is
/// `idle` because there is no active run yet: the honest state, not faked
/// "thinking" theater. When the runtime drives a run, the same rows show live
/// engaged / contributing status from the event stream.
public enum SampleRoster {
    public static let participants: [Participant] = [
        Participant(
            id: "claude",
            name: "Claude",
            kind: .visiting,
            access: .mediated,
            status: .idle,
            note: "Your authenticated session. Observable + rationale trace."
        ),
        Participant(
            id: "codex",
            name: "Codex",
            kind: .visiting,
            access: .mediated,
            status: .idle,
            note: "Your authenticated session. Observable + rationale trace."
        ),
        Participant(
            id: "deepseek",
            name: "DeepSeek V4",
            kind: .roster,
            access: .mediated,
            status: .idle,
            note: "The router's capability prior for hard reasoning. A full peer, not a messenger."
        ),
        Participant(
            id: "byo",
            name: "Bring your own",
            kind: .brought,
            access: .resident,
            status: .idle,
            note: "Your own endpoint (local or hosted). Addressable directly; resident context where local."
        ),
    ]
}
