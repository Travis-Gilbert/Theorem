import Foundation

/// A recorded reference run for the run-detail surface (harness UI spec, Part 2).
///
/// This is NOT invented UI data: it is the `full_lifecycle_to_closed` scenario
/// from the harness parity corpus
/// (`docs/plans/harness-rust-port/parity/fixtures.json`), transcribed verbatim.
/// Every status and `stateHashAfter` is the real Python-reference output that the
/// Rust kernel reproduces byte-for-byte. The ledger and outcome are the actual
/// `CONTEXT.PACKED` and `OUTCOME.RECORDED` payloads from that run.
///
/// It plays the role `SampleRoom` / `SampleScene` play for their surfaces: a real,
/// honest fixture the view renders during development and as the "past run" case.
/// When the runtime crate persists live events, `RunDetailView` renders a live
/// `HarnessRun` decoded from the event log with no view change.
public enum SampleRun {
    public static let fullLifecycle = HarnessRun(
        runID: "run-fixture-0001",
        task: "port harness to rust",
        actor: "claude-code",
        events: [
            HarnessRunEvent(seq: 1, type: "RUN.CREATED", status: "created", stateHashAfter: "d6f5e8d0636d5e859ea8528e09b11e751de24800d46b9f90a93c4e2d9883c434"),
            HarnessRunEvent(seq: 2, type: "HOST.OBSERVED", status: "observed", stateHashAfter: "e2b0dd9d5c907fcc0884e5bd65b60a7d2b90c274d296df2e29b7761dca53e9ef"),
            HarnessRunEvent(seq: 3, type: "TASK.RESOLVED", status: "resolved", stateHashAfter: "d58797f7185a5ada00491477c2684835378734ec3fc1b2240becd6d7d780912f"),
            HarnessRunEvent(seq: 4, type: "PROFILE.SELECTED", status: "profile_selected", stateHashAfter: "d16c705e500b88597852d5d52b5f11c53debbce68305877a257b9209e1594eed"),
            HarnessRunEvent(seq: 5, type: "TOOLKIT.COMPILED", status: "toolkit_compiled", stateHashAfter: "c2f65651cd3ac0889c11b06618fb1fdbb44a3737da0dc66d4e08c2a3d41d019f"),
            HarnessRunEvent(seq: 6, type: "MAPS.LOADED", status: "maps_loaded", stateHashAfter: "4d7a4a1ad9d6d5f028c6cfdcd1850d3fcacebafc3328da36b228195e54aa9888"),
            HarnessRunEvent(seq: 7, type: "CONTEXT.PLANNED", status: "context_planned", stateHashAfter: "3343c7796eeeb4581dd88faed7c19e46b2d94eb9727c21d53e413f734d47a208"),
            HarnessRunEvent(seq: 8, type: "CONTEXT.PACKED", status: "context_packed", stateHashAfter: "7253285019dfe48e0563671117233f7c70ff459b560c353a62ab33fc58c0fe5a"),
            HarnessRunEvent(seq: 9, type: "CONTEXT.INJECTED", status: "context_injected", stateHashAfter: "1913561aa2653f5cbeb3ffa99b01ef69fb8adad84f6ad9ece5f460d8a50660a5"),
            HarnessRunEvent(seq: 10, type: "AGENT.ACTING", status: "agent_acting", stateHashAfter: "aa49cf6a030a0f51ce3b60dc485bef939a0b4e45dab48bee0bca144a77aa1279"),
            HarnessRunEvent(seq: 11, type: "OUTCOME.RECORDED", status: "outcome_recorded", stateHashAfter: "c79cfbec2d41e25a6f68822ff8e7072d2d717b67101656b6c17e4c75d51fb195"),
            HarnessRunEvent(seq: 12, type: "LEARNING.PROPOSED", status: "learning_proposed", stateHashAfter: "2baa8745cfe467c8cae5a2b62ed63be23590dbeaff78f4f62bf66b8e56d9518f"),
            HarnessRunEvent(seq: 13, type: "REVIEW.QUEUED", status: "review_queued", stateHashAfter: "f880bc1255485c4ac7c9b5abf82c160a7f7e9ab9e8e7f34dd00dd30b7f1a8edc"),
            HarnessRunEvent(seq: 14, type: "FEDERATION.SIGNAL_PREPARED", status: "federation_signal_prepared", stateHashAfter: "9e6c10531578a32ca6ea1d041ccd33b2b488ac59298779defa6b9ee3cbd83b42"),
            HarnessRunEvent(seq: 15, type: "RUN.CLOSED", status: "closed", stateHashAfter: "1c2cd8b2f06b83ea469b24ec6a8ae663e490772e86e33bfb079319666abe421f"),
        ],
        ledger: HarnessRunLedger(
            artifactID: "art-1",
            budgetTokens: 1000,
            capsuleTokens: 200,
            includedAtoms: 5,
            excludedAtoms: 2,
            savedTokens: 300
        ),
        outcome: HarnessRunOutcome(
            accepted: true,
            testsPassed: true,
            filesChanged: ["state_machine.rs"],
            validators: [HarnessRunValidator(id: "v1", status: "passed")],
            summary: "ported"
        ),
        // The `art-1` context pack, computed through the live Python
        // ContextWebPack.bounded: 5 atoms ranked within budget, 2 generated
        // artifacts quarantined. raw 500 / packed 200 / saved 300 matches the
        // run's CONTEXT.PACKED ledger exactly.
        contextAtoms: [
            HarnessContextAtom(id: "atom-1", title: "kernel state machine", tokens: 40, decision: .included, reason: "ranked_within_budget"),
            HarnessContextAtom(id: "atom-2", title: "guard table", tokens: 40, decision: .included, reason: "ranked_within_budget"),
            HarnessContextAtom(id: "atom-3", title: "parity corpus", tokens: 40, decision: .included, reason: "ranked_within_budget"),
            HarnessContextAtom(id: "atom-4", title: "state hash", tokens: 40, decision: .included, reason: "ranked_within_budget"),
            HarnessContextAtom(id: "atom-5", title: "replay/fork", tokens: 40, decision: .included, reason: "ranked_within_budget"),
            HarnessContextAtom(id: "file:dist/bundle.js", title: "generated bundle", tokens: 150, decision: .excluded, reason: "generated_artifact_quarantined"),
            HarnessContextAtom(id: "file:build/out.js", title: "generated build", tokens: 150, decision: .excluded, reason: "generated_artifact_quarantined"),
        ]
    )
}
