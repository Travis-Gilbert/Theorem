import Foundation

public struct CommonplaceRegistry: Equatable, Sendable, Identifiable {
    public var id: String
    public var providers: [CommonplaceProvider]
    public var participantBindings: [CommonplaceParticipantBinding]
    public var machineryBindings: [CommonplaceMachineryBinding]
    public var charter: CommonplaceTeamCharter

    public init(
        id: String,
        providers: [CommonplaceProvider],
        participantBindings: [CommonplaceParticipantBinding],
        machineryBindings: [CommonplaceMachineryBinding],
        charter: CommonplaceTeamCharter
    ) {
        self.id = id
        self.providers = providers
        self.participantBindings = participantBindings
        self.machineryBindings = machineryBindings
        self.charter = charter
    }

    public func participantBinding(for id: String?) -> CommonplaceParticipantBinding? {
        guard let id else { return nil }
        return participantBindings.first { $0.id == id }
    }

    public var platformCredentialProviders: [CommonplaceProvider] {
        providers.filter { $0.credentialMode == .platformManaged }
    }
}

public struct CommonplaceProvider: Equatable, Sendable, Identifiable {
    public var id: String
    public var displayName: String
    public var baseURL: URL?
    public var protocolKind: CommonplaceProviderProtocol
    public var credentialMode: CommonplaceCredentialMode
    public var probePath: String?

    public init(
        id: String,
        displayName: String,
        baseURL: URL? = nil,
        protocolKind: CommonplaceProviderProtocol,
        credentialMode: CommonplaceCredentialMode,
        probePath: String? = "/v1/models"
    ) {
        self.id = id
        self.displayName = displayName
        self.baseURL = baseURL
        self.protocolKind = protocolKind
        self.credentialMode = credentialMode
        self.probePath = probePath
    }
}

public enum CommonplaceProviderProtocol: String, Equatable, Sendable {
    case openAIChat = "openai_chat"
    case openAIResponses = "openai_responses"
    case anthropicMessages = "anthropic_messages"
    case harness
    case localRuntime = "local_runtime"
}

public enum CommonplaceCredentialMode: String, Equatable, Sendable {
    case platformManaged = "platform_managed"
    case userSupplied = "user_supplied"
    case broughtEndpoint = "brought_endpoint"
    case localOnly = "local_only"
}

public struct CommonplaceParticipantBinding: Equatable, Sendable, Identifiable {
    public var id: String
    public var participantID: String
    public var providerID: String
    public var modelID: String
    public var availability: CommonplaceBindingAvailability
    public var capabilityTags: [CommonplaceCapabilityTag]
    public var contextWindowTokens: Int?
    public var costTier: CommonplaceCostTier
    public var researchTrack: Bool

    public init(
        id: String,
        participantID: String,
        providerID: String,
        modelID: String,
        availability: CommonplaceBindingAvailability = .available,
        capabilityTags: [CommonplaceCapabilityTag],
        contextWindowTokens: Int? = nil,
        costTier: CommonplaceCostTier,
        researchTrack: Bool = false
    ) {
        self.id = id
        self.participantID = participantID
        self.providerID = providerID
        self.modelID = modelID
        self.availability = availability
        self.capabilityTags = capabilityTags
        self.contextWindowTokens = contextWindowTokens
        self.costTier = costTier
        self.researchTrack = researchTrack
    }
}

public struct CommonplaceMachineryBinding: Equatable, Sendable, Identifiable {
    public var id: String
    public var providerID: String
    public var modelID: String
    public var kind: CommonplaceMachineryKind
    public var availability: CommonplaceBindingAvailability
    public var placement: CommonplaceMachineryPlacement

    public init(
        id: String,
        providerID: String,
        modelID: String,
        kind: CommonplaceMachineryKind,
        availability: CommonplaceBindingAvailability = .available,
        placement: CommonplaceMachineryPlacement
    ) {
        self.id = id
        self.providerID = providerID
        self.modelID = modelID
        self.kind = kind
        self.availability = availability
        self.placement = placement
    }
}

public enum CommonplaceBindingAvailability: String, Equatable, Sendable {
    case available
    case needsKey = "needs_key"
    case research
    case fallback
}

public enum CommonplaceCapabilityTag: String, Equatable, Sendable, CaseIterable {
    case text
    case code
    case longContext = "long_context"
    case toolCalling = "tool_calling"
    case multimodal
    case browserAction = "browser_action"
    case localResearch = "local_research"
    case recurrent
    case projectedThoughtVector = "projected_thought_vector"
    case nativeHiddenState = "native_hidden_state"
}

public enum CommonplaceCostTier: String, Equatable, Sendable {
    case cheap
    case standard
    case premium
    case local
    case research
}

public enum CommonplaceMachineryKind: String, Equatable, Sendable {
    case ocr
    case speechToText = "speech_to_text"
    case textToSpeech = "text_to_speech"
    case textEmbedder = "text_embedder"
    case codeEmbedder = "code_embedder"
    case ranker
    case reranker
}

public enum CommonplaceMachineryPlacement: String, Equatable, Sendable {
    case clientOnDevice = "client_on_device"
    case hostedSubstrate = "hosted_substrate"
    case userEndpoint = "user_endpoint"
}

public struct CommonplaceTeamCharter: Equatable, Sendable, Identifiable {
    public var id: String
    public var title: String
    public var promptSeed: String
    public var principles: [String]
    public var substrateInstructions: [String]

    public init(
        id: String,
        title: String,
        promptSeed: String,
        principles: [String],
        substrateInstructions: [String]
    ) {
        self.id = id
        self.title = title
        self.promptSeed = promptSeed
        self.principles = principles
        self.substrateInstructions = substrateInstructions
    }

    public var composedPrompt: String {
        let principleBlock = principles.map { "- \($0)" }.joined(separator: "\n")
        let substrateBlock = substrateInstructions.map { "- \($0)" }.joined(separator: "\n")
        return [promptSeed, principleBlock, substrateBlock]
            .filter { !$0.isEmpty }
            .joined(separator: "\n\n")
    }
}
