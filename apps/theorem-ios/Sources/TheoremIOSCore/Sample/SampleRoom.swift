import Foundation

public enum SampleRoom {
    private static let registry = SampleCommonplaceRegistry.registry
    private static let sampleAsk = "Where do RustyWeb search, SceneOS, and the iOS room meet?"
    private static let routePlan = CommonplaceRouter().plan(
        query: sampleAsk,
        registry: registry
    )

    public static let room = baseRoom.applyingRoutePlan(routePlan)

    private static let baseRoom = CommonplaceRoom(
        id: "commonplace-room-sample",
        title: "Commonplace",
        ask: sampleAsk,
        participants: [
            CommonplaceParticipant(
                id: "claude",
                displayName: "Claude",
                shortName: "CL",
                origin: .visiting,
                endpointLabel: "harness"
            ),
            CommonplaceParticipant(
                id: "codex",
                displayName: "Codex",
                shortName: "CX",
                origin: .visiting,
                endpointLabel: "harness"
            ),
            CommonplaceParticipant(
                id: "mistral-medium",
                displayName: "Mistral Medium",
                shortName: "MM",
                origin: .roster,
                endpointLabel: "api"
            ),
            CommonplaceParticipant(
                id: "deepseek-v4-pro",
                displayName: "DeepSeek V4 Pro",
                shortName: "DS",
                origin: .roster,
                status: .idle,
                endpointLabel: "api"
            ),
            CommonplaceParticipant(
                id: "glm-5-1",
                displayName: "GLM 5.1",
                shortName: "GLM",
                origin: .roster,
                status: .idle,
                endpointLabel: "api"
            ),
            CommonplaceParticipant(
                id: "qwen-coder-next",
                displayName: "Qwen Coder Next",
                shortName: "QCN",
                origin: .roster,
                status: .idle,
                endpointLabel: "api"
            ),
            CommonplaceParticipant(
                id: "jamba-large",
                displayName: "Jamba Large",
                shortName: "JAM",
                origin: .roster,
                status: .idle,
                endpointLabel: "api"
            ),
            CommonplaceParticipant(
                id: "xlstm-research",
                displayName: "xLSTM",
                shortName: "XLS",
                origin: .roster,
                status: .idle,
                endpointLabel: "research"
            ),
            CommonplaceParticipant(
                id: "gemma-26b",
                displayName: "Gemma 26B",
                shortName: "G26",
                origin: .roster,
                status: .idle,
                endpointLabel: "api"
            ),
            CommonplaceParticipant(
                id: "gemma-4b",
                displayName: "Gemma 4B",
                shortName: "G4",
                origin: .roster,
                status: .idle,
                endpointLabel: "api"
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
        scene: SampleScene.package,
        registry: registry,
        routePlan: routePlan
    )
}
