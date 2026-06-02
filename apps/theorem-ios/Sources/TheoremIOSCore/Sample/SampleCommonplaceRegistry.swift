import Foundation

public enum SampleCommonplaceRegistry {
    public static let registry = CommonplaceRegistry(
        id: "commonplace-v2-registry",
        providers: providers,
        participantBindings: participants,
        machineryBindings: machinery,
        charter: charter
    )

    private static let providers: [CommonplaceProvider] = [
        provider("anthropic", "Anthropic", "https://api.anthropic.com", .anthropicMessages),
        provider("openai", "OpenAI", "https://api.openai.com", .openAIResponses),
        provider("mistral", "Mistral", "https://api.mistral.ai", .openAIChat),
        provider("deepseek", "DeepSeek", "https://api.deepseek.com", .openAIChat),
        provider("zai", "Z.ai", "https://api.z.ai", .openAIChat),
        provider("qwen", "Qwen", "https://dashscope.aliyuncs.com/compatible-mode", .openAIChat),
        provider("ai21", "AI21", "https://api.ai21.com", .openAIChat),
        provider("minimax", "MiniMax", "https://api.minimax.io", .openAIChat),
        provider("google", "Google", "https://generativelanguage.googleapis.com", .openAIChat),
        CommonplaceProvider(
            id: "xlstm-research",
            displayName: "xLSTM Research",
            protocolKind: .localRuntime,
            credentialMode: .localOnly,
            probePath: nil
        ),
        CommonplaceProvider(
            id: "on-device",
            displayName: "On-device",
            protocolKind: .localRuntime,
            credentialMode: .localOnly,
            probePath: nil
        ),
        CommonplaceProvider(
            id: "substrate-hosted",
            displayName: "Hosted substrate",
            protocolKind: .localRuntime,
            credentialMode: .platformManaged,
            probePath: nil
        ),
    ]

    private static let participants: [CommonplaceParticipantBinding] = [
        participant(
            "claude",
            provider: "anthropic",
            model: "claude",
            tags: [.text, .code, .longContext, .toolCalling, .projectedThoughtVector],
            cost: .premium
        ),
        participant(
            "codex",
            provider: "openai",
            model: "codex",
            tags: [.text, .code, .toolCalling, .projectedThoughtVector],
            cost: .premium
        ),
        participant(
            "mistral-medium",
            provider: "mistral",
            model: "mistral-medium-latest",
            tags: [.text, .code, .multimodal, .toolCalling, .projectedThoughtVector],
            context: 256_000,
            cost: .standard
        ),
        participant(
            "deepseek-v4-pro",
            provider: "deepseek",
            model: "deepseek-v4-pro",
            tags: [.text, .code, .longContext, .toolCalling, .projectedThoughtVector],
            cost: .premium
        ),
        participant(
            "glm-5-1",
            provider: "zai",
            model: "glm-5.1",
            tags: [.text, .code, .longContext, .toolCalling, .projectedThoughtVector],
            cost: .premium
        ),
        participant(
            "qwen-coder-next",
            provider: "qwen",
            model: "qwen3-coder-next",
            tags: [.text, .code, .toolCalling, .projectedThoughtVector],
            cost: .cheap
        ),
        participant(
            "qwen-coder-large",
            provider: "qwen",
            model: "qwen3-coder-480b-a35b-instruct",
            tags: [.text, .code, .longContext, .toolCalling, .projectedThoughtVector],
            cost: .premium
        ),
        participant(
            "jamba-large",
            provider: "ai21",
            model: "jamba-large",
            tags: [.text, .longContext, .toolCalling, .projectedThoughtVector],
            context: 256_000,
            cost: .standard
        ),
        participant(
            "minimax-m2-7",
            provider: "minimax",
            model: "minimax-m2.7",
            tags: [.text, .toolCalling, .browserAction, .projectedThoughtVector],
            cost: .standard
        ),
        participant(
            "gemma-26b",
            provider: "google",
            model: "gemma-4-26b-a4b-it",
            tags: [.text, .code, .projectedThoughtVector],
            cost: .standard
        ),
        participant(
            "gemma-4b",
            provider: "google",
            model: "gemma-4b-it",
            tags: [.text, .toolCalling, .projectedThoughtVector],
            cost: .cheap
        ),
        participant(
            "xlstm-research",
            provider: "xlstm-research",
            model: "xlstm-research",
            tags: [.text, .localResearch, .recurrent],
            cost: .research,
            research: true
        ),
    ]

    private static let machinery: [CommonplaceMachineryBinding] = [
        machine("mistral-ocr", provider: "mistral", model: "mistral-ocr-latest", kind: .ocr, placement: .hostedSubstrate),
        machine("voxtral-mini-realtime", provider: "on-device", model: "onnx-community/Voxtral-Mini-4B-Realtime-2602-ONNX", kind: .speechToText, placement: .clientOnDevice),
        machine("mistral-tts", provider: "mistral", model: "mistral-tts-latest", kind: .textToSpeech, placement: .clientOnDevice),
        machine("ministral-ocr", provider: "mistral", model: "ministral-ocr", kind: .ocr, placement: .hostedSubstrate, availability: .fallback),
        machine("unixcoder", provider: "substrate-hosted", model: "microsoft/unixcoder-base", kind: .codeEmbedder, placement: .hostedSubstrate),
        machine("all-minilm-l6-v2", provider: "substrate-hosted", model: "sentence-transformers/all-MiniLM-L6-v2", kind: .textEmbedder, placement: .hostedSubstrate),
        machine("ranker", provider: "substrate-hosted", model: "substrate-ranker", kind: .ranker, placement: .hostedSubstrate),
        machine("reranker", provider: "substrate-hosted", model: "substrate-reranker", kind: .reranker, placement: .hostedSubstrate),
    ]

    private static let charter = CommonplaceTeamCharter(
        id: "commonplace-team-charter-v1",
        title: "Team charter",
        promptSeed: "You are a distinct participant in a shared substrate room. You are not the sole answerer.",
        principles: [
            "Work from your own strengths without claiming a fixed role.",
            "Read the shared goal, recent contributions, substrate state, and unresolved tensions before contributing.",
            "Add marginal value instead of repeating what another participant already handled.",
            "Surface disagreements as tensions and let the composition layer synthesize the final answer.",
            "Treat brought agents as peers; only a user's own brought agent can be directly addressed."
        ],
        substrateInstructions: [
            "Browse means express a fetch or crawl intent, then reason over the ingested graph.",
            "OCR, speech, embeddings, ranking, and reranking are machinery, not room voices.",
            "Provider credentials unlock endpoints; model bindings decide which participant or machinery capability is used."
        ]
    )

    private static func provider(
        _ id: String,
        _ name: String,
        _ baseURL: String,
        _ protocolKind: CommonplaceProviderProtocol
    ) -> CommonplaceProvider {
        CommonplaceProvider(
            id: id,
            displayName: name,
            baseURL: URL(string: baseURL),
            protocolKind: protocolKind,
            credentialMode: .platformManaged
        )
    }

    private static func participant(
        _ id: String,
        provider: String,
        model: String,
        tags: [CommonplaceCapabilityTag],
        context: Int? = nil,
        cost: CommonplaceCostTier,
        research: Bool = false
    ) -> CommonplaceParticipantBinding {
        CommonplaceParticipantBinding(
            id: id,
            participantID: id,
            providerID: provider,
            modelID: model,
            availability: research ? .research : .available,
            capabilityTags: tags,
            contextWindowTokens: context,
            costTier: cost,
            researchTrack: research
        )
    }

    private static func machine(
        _ id: String,
        provider: String,
        model: String,
        kind: CommonplaceMachineryKind,
        placement: CommonplaceMachineryPlacement,
        availability: CommonplaceBindingAvailability = .available
    ) -> CommonplaceMachineryBinding {
        CommonplaceMachineryBinding(
            id: id,
            providerID: provider,
            modelID: model,
            kind: kind,
            availability: availability,
            placement: placement
        )
    }
}
