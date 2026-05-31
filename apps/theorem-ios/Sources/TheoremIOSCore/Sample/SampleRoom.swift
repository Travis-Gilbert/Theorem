import Foundation

public enum SampleRoom {
    public static let room = CommonplaceRoom(
        id: "commonplace-room-sample",
        title: "Commonplace",
        ask: "Search the substrate",
        participants: [
            CommonplaceParticipant(
                id: "claude",
                displayName: "Claude",
                shortName: "CL",
                origin: .visiting,
                status: .thinking,
                endpointLabel: "harness"
            ),
            CommonplaceParticipant(
                id: "codex",
                displayName: "Codex",
                shortName: "CX",
                origin: .visiting,
                status: .contributing,
                endpointLabel: "harness"
            ),
            CommonplaceParticipant(
                id: "deepseek",
                displayName: "DeepSeek",
                shortName: "DS",
                origin: .roster,
                status: .idle,
                endpointLabel: "api"
            ),
            CommonplaceParticipant(
                id: "gemma31b",
                displayName: "Gemma 31B",
                shortName: "31B",
                origin: .roster,
                status: .idle,
                endpointLabel: "native"
            ),
            CommonplaceParticipant(
                id: "brought-agent",
                displayName: "Brought agent",
                shortName: "BYO",
                origin: .brought,
                status: .idle,
                endpointLabel: "custom"
            ),
        ],
        contributions: [
            CommonplaceContribution(
                id: "sample-human-ask",
                authorKind: .human,
                authorID: nil,
                state: .resolved,
                summary: "Ask",
                body: "Where do RustyWeb search, SceneOS, and the iOS room meet?"
            ),
            CommonplaceContribution(
                id: "sample-codex-scene",
                authorKind: .participant,
                authorID: "codex",
                state: .contributing,
                summary: "Scene wiring",
                body: "The room can render the graph while keeping the participant roster as presence, not a picker."
            ),
            CommonplaceContribution(
                id: "sample-claude-design",
                authorKind: .participant,
                authorID: "claude",
                state: .thinking,
                summary: "Design pass",
                body: "The instrument surface should keep the island at the bottom and expose controls only when summoned."
            ),
            CommonplaceContribution(
                id: "sample-substrate-graph",
                authorKind: .substrate,
                authorID: nil,
                state: .resolved,
                summary: "Substrate graph",
                body: "ScenePackageV2 carries the current graph projection, provenance, and search result structure."
            ),
        ],
        scene: SampleScene.package
    )
}
