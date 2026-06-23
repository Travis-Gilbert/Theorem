import Foundation

public struct CommonplaceCreditEstimator: Equatable, Sendable {
    public var policy: CommonplaceCreditPolicy
    public var priceCard: CommonplacePriceCard

    public init(
        policy: CommonplaceCreditPolicy = .commonplaceDefault,
        priceCard: CommonplacePriceCard = .commonplaceDraft
    ) {
        self.policy = policy
        self.priceCard = priceCard
    }

    public func estimate(
        routePlan: CommonplaceRoutePlan,
        registry: CommonplaceRegistry,
        tokenBudgets: CommonplaceTokenBudgets = .preview,
        toolBudget: CommonplaceToolUseBudget = CommonplaceToolUseBudget()
    ) -> CommonplaceCreditEstimate {
        let estimatedParticipantIDs = orderedUnique(routePlan.activeParticipantIDs)
        let worstCaseParticipantIDs = orderedUnique(routePlan.stages.flatMap(\.participantIDs))

        let estimatedModelItems = modelLineItems(
            participantIDs: estimatedParticipantIDs,
            registry: registry,
            tokenBudgets: tokenBudgets
        )
        let worstCaseModelItems = modelLineItems(
            participantIDs: worstCaseParticipantIDs,
            registry: registry,
            tokenBudgets: tokenBudgets
        )
        let toolItems = toolLineItems(toolBudget)

        let estimatedRawUSD = rawUSD(for: estimatedModelItems) + rawUSD(for: toolItems)
        let worstCaseRawUSD = (rawUSD(for: worstCaseModelItems) + rawUSD(for: toolItems)) * policy.worstCaseMultiplier

        let estimatedCredits = credits(
            forRawUSD: estimatedRawUSD,
            participantCount: estimatedParticipantIDs.count,
            toolBudget: toolBudget
        )
        let worstCaseCredits = credits(
            forRawUSD: worstCaseRawUSD,
            participantCount: worstCaseParticipantIDs.count,
            toolBudget: toolBudget
        )

        return CommonplaceCreditEstimate(
            priceCardID: priceCard.id,
            policyID: policy.id,
            estimatedCredits: estimatedCredits,
            worstCaseCredits: worstCaseCredits,
            requiresConfirmation: worstCaseCredits >= policy.confirmationThresholdCredits,
            estimatedRawUSD: estimatedRawUSD,
            worstCaseRawUSD: worstCaseRawUSD,
            estimatedParticipantIDs: estimatedParticipantIDs,
            worstCaseParticipantIDs: worstCaseParticipantIDs,
            estimatedModelLineItems: estimatedModelItems,
            worstCaseModelLineItems: worstCaseModelItems,
            toolLineItems: toolItems
        )
    }

    private func modelLineItems(
        participantIDs: [String],
        registry: CommonplaceRegistry,
        tokenBudgets: CommonplaceTokenBudgets
    ) -> [CommonplaceCreditLineItem] {
        participantIDs.compactMap { participantID in
            guard let binding = registry.participantBindings.first(where: { $0.participantID == participantID }) else {
                return nil
            }
            let budget = tokenBudgets.budget(for: binding)
            let price = priceCard.price(for: binding)
            return CommonplaceCreditLineItem(
                id: "model:\(binding.providerID):\(binding.modelID):\(participantID)",
                label: participantID,
                category: .modelTokens,
                rawUSD: price.rawUSD(for: budget),
                detail: "\(budget.inputTokens) in / \(budget.outputTokens) out"
            )
        }
    }

    private func toolLineItems(_ budget: CommonplaceToolUseBudget) -> [CommonplaceCreditLineItem] {
        var items: [CommonplaceCreditLineItem] = []

        if budget.ocrPages > 0 {
            items.append(
                CommonplaceCreditLineItem(
                    id: "tool:ocr",
                    label: "OCR",
                    category: .ocr,
                    rawUSD: Double(budget.ocrPages) * priceCard.ocrUSDPerPage,
                    detail: "\(budget.ocrPages) pages"
                )
            )
        }

        if budget.speechMinutes > 0 {
            items.append(
                CommonplaceCreditLineItem(
                    id: "tool:speech-to-text",
                    label: "Speech",
                    category: .speechToText,
                    rawUSD: budget.speechMinutes * priceCard.speechUSDPerMinute,
                    detail: "\(trimmed(budget.speechMinutes)) minutes"
                )
            )
        }

        if budget.ttsCharacters > 0 {
            let thousands = Double(budget.ttsCharacters) / 1_000
            items.append(
                CommonplaceCreditLineItem(
                    id: "tool:text-to-speech",
                    label: "TTS",
                    category: .textToSpeech,
                    rawUSD: thousands * priceCard.ttsUSDPerThousandCharacters,
                    detail: "\(budget.ttsCharacters) characters"
                )
            )
        }

        if budget.webFetches > 0 {
            items.append(
                CommonplaceCreditLineItem(
                    id: "tool:web-fetch",
                    label: "Web fetch",
                    category: .webFetch,
                    rawUSD: Double(budget.webFetches) * priceCard.webFetchUSD,
                    detail: "\(budget.webFetches) fetches"
                )
            )
        }

        if budget.substrateSearches > 0 {
            items.append(
                CommonplaceCreditLineItem(
                    id: "tool:substrate-search",
                    label: "Substrate search",
                    category: .substrateSearch,
                    rawUSD: Double(budget.substrateSearches) * priceCard.substrateSearchUSD,
                    detail: "\(budget.substrateSearches) searches"
                )
            )
        }

        return items
    }

    private func credits(
        forRawUSD rawUSD: Double,
        participantCount: Int,
        toolBudget: CommonplaceToolUseBudget
    ) -> Int {
        let meteredCredits = Int(ceil((rawUSD * policy.platformMultiplier) / policy.retailUSDPerCredit))
        let floorCredits = policy.baseFloorCredits
            + (participantCount * policy.floorCreditsPerParticipant)
            + toolFloorCredits(toolBudget)
        return max(meteredCredits, floorCredits)
    }

    private func toolFloorCredits(_ budget: CommonplaceToolUseBudget) -> Int {
        var floor = 0
        if budget.ocrPages > 0 { floor += policy.ocrFloorCredits }
        if budget.speechMinutes > 0 { floor += policy.speechFloorCredits }
        if budget.ttsCharacters > 0 { floor += policy.ttsFloorCredits }
        if budget.webFetches > 0 || budget.substrateSearches > 0 { floor += policy.searchFloorCredits }
        return floor
    }

    private func rawUSD(for items: [CommonplaceCreditLineItem]) -> Double {
        items.reduce(0) { $0 + $1.rawUSD }
    }

    private func orderedUnique(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        return ids.filter { seen.insert($0).inserted }
    }

    private func trimmed(_ value: Double) -> String {
        if value.rounded() == value {
            return "\(Int(value))"
        }
        return String(format: "%.2f", value)
    }
}

public struct CommonplaceCreditPolicy: Equatable, Sendable, Identifiable {
    public var id: String
    public var includedFreeCredits: Int
    public var retailUSDPerCredit: Double
    public var platformMultiplier: Double
    public var worstCaseMultiplier: Double
    public var baseFloorCredits: Int
    public var floorCreditsPerParticipant: Int
    public var ocrFloorCredits: Int
    public var speechFloorCredits: Int
    public var ttsFloorCredits: Int
    public var searchFloorCredits: Int
    public var confirmationThresholdCredits: Int

    public init(
        id: String,
        includedFreeCredits: Int,
        retailUSDPerCredit: Double,
        platformMultiplier: Double,
        worstCaseMultiplier: Double,
        baseFloorCredits: Int,
        floorCreditsPerParticipant: Int,
        ocrFloorCredits: Int,
        speechFloorCredits: Int,
        ttsFloorCredits: Int,
        searchFloorCredits: Int,
        confirmationThresholdCredits: Int
    ) {
        self.id = id
        self.includedFreeCredits = includedFreeCredits
        self.retailUSDPerCredit = retailUSDPerCredit
        self.platformMultiplier = platformMultiplier
        self.worstCaseMultiplier = worstCaseMultiplier
        self.baseFloorCredits = baseFloorCredits
        self.floorCreditsPerParticipant = floorCreditsPerParticipant
        self.ocrFloorCredits = ocrFloorCredits
        self.speechFloorCredits = speechFloorCredits
        self.ttsFloorCredits = ttsFloorCredits
        self.searchFloorCredits = searchFloorCredits
        self.confirmationThresholdCredits = confirmationThresholdCredits
    }

    public static let commonplaceDefault = CommonplaceCreditPolicy(
        id: "commonplace-credit-policy-v1",
        includedFreeCredits: 50,
        retailUSDPerCredit: 0.01,
        platformMultiplier: 1.75,
        worstCaseMultiplier: 1.35,
        baseFloorCredits: 2,
        floorCreditsPerParticipant: 2,
        ocrFloorCredits: 1,
        speechFloorCredits: 1,
        ttsFloorCredits: 1,
        searchFloorCredits: 1,
        confirmationThresholdCredits: 25
    )
}

public struct CommonplaceTokenBudget: Equatable, Sendable {
    public var inputTokens: Int
    public var outputTokens: Int

    public init(inputTokens: Int, outputTokens: Int) {
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
    }
}

public struct CommonplaceTokenBudgets: Equatable, Sendable {
    public var defaultBudget: CommonplaceTokenBudget
    public var participantBudgets: [String: CommonplaceTokenBudget]
    public var modelBudgets: [String: CommonplaceTokenBudget]

    public init(
        defaultBudget: CommonplaceTokenBudget,
        participantBudgets: [String: CommonplaceTokenBudget] = [:],
        modelBudgets: [String: CommonplaceTokenBudget] = [:]
    ) {
        self.defaultBudget = defaultBudget
        self.participantBudgets = participantBudgets
        self.modelBudgets = modelBudgets
    }

    public func budget(for binding: CommonplaceParticipantBinding) -> CommonplaceTokenBudget {
        participantBudgets[binding.participantID]
            ?? participantBudgets[binding.id]
            ?? modelBudgets[binding.modelID]
            ?? defaultBudget
    }

    public static let preview = CommonplaceTokenBudgets(
        defaultBudget: CommonplaceTokenBudget(inputTokens: 6_000, outputTokens: 1_000),
        participantBudgets: [
            "claude": CommonplaceTokenBudget(inputTokens: 10_000, outputTokens: 1_800),
            "codex": CommonplaceTokenBudget(inputTokens: 8_000, outputTokens: 1_400),
            "mistral-medium": CommonplaceTokenBudget(inputTokens: 8_000, outputTokens: 1_200),
            "qwen-coder-next": CommonplaceTokenBudget(inputTokens: 6_000, outputTokens: 1_000),
            "qwen-coder-large": CommonplaceTokenBudget(inputTokens: 12_000, outputTokens: 1_800),
            "jamba-large": CommonplaceTokenBudget(inputTokens: 16_000, outputTokens: 1_600),
        ]
    )
}

public struct CommonplaceToolUseBudget: Equatable, Sendable {
    public var ocrPages: Int
    public var speechMinutes: Double
    public var ttsCharacters: Int
    public var webFetches: Int
    public var substrateSearches: Int

    public init(
        ocrPages: Int = 0,
        speechMinutes: Double = 0,
        ttsCharacters: Int = 0,
        webFetches: Int = 0,
        substrateSearches: Int = 0
    ) {
        self.ocrPages = ocrPages
        self.speechMinutes = speechMinutes
        self.ttsCharacters = ttsCharacters
        self.webFetches = webFetches
        self.substrateSearches = substrateSearches
    }

    public static func preview(for query: String, features: [CommonplaceRouteFeature]) -> CommonplaceToolUseBudget {
        let lower = query.lowercased()
        let documentLike = features.contains(.document)
            || containsAny(lower, ["ocr", "pdf", "paper", "document", "scan", "image"])
        let webLike = features.contains(.browserAction)
            || features.contains(.research)
            || containsAny(lower, ["web", "site", "crawl", "browser", "internet", "source"])

        return CommonplaceToolUseBudget(
            ocrPages: documentLike ? 6 : 0,
            speechMinutes: containsAny(lower, ["transcribe", "audio", "voice", "meeting"]) ? 3 : 0,
            ttsCharacters: containsAny(lower, ["read aloud", "voiceover", "tts"]) ? 1_200 : 0,
            webFetches: webLike ? 4 : 0,
            substrateSearches: 1
        )
    }

    private static func containsAny(_ text: String, _ needles: [String]) -> Bool {
        needles.contains { text.contains($0) }
    }
}

public struct CommonplacePriceCard: Equatable, Sendable, Identifiable {
    public var id: String
    public var sourceLabel: String
    public var modelPrices: [CommonplaceModelPrice]
    public var cheapFallbackPrice: CommonplaceModelPrice
    public var standardFallbackPrice: CommonplaceModelPrice
    public var premiumFallbackPrice: CommonplaceModelPrice
    public var localFallbackPrice: CommonplaceModelPrice
    public var researchFallbackPrice: CommonplaceModelPrice
    public var ocrUSDPerPage: Double
    public var speechUSDPerMinute: Double
    public var ttsUSDPerThousandCharacters: Double
    public var webFetchUSD: Double
    public var substrateSearchUSD: Double

    public init(
        id: String,
        sourceLabel: String,
        modelPrices: [CommonplaceModelPrice],
        cheapFallbackPrice: CommonplaceModelPrice,
        standardFallbackPrice: CommonplaceModelPrice,
        premiumFallbackPrice: CommonplaceModelPrice,
        localFallbackPrice: CommonplaceModelPrice,
        researchFallbackPrice: CommonplaceModelPrice,
        ocrUSDPerPage: Double,
        speechUSDPerMinute: Double,
        ttsUSDPerThousandCharacters: Double,
        webFetchUSD: Double,
        substrateSearchUSD: Double
    ) {
        self.id = id
        self.sourceLabel = sourceLabel
        self.modelPrices = modelPrices
        self.cheapFallbackPrice = cheapFallbackPrice
        self.standardFallbackPrice = standardFallbackPrice
        self.premiumFallbackPrice = premiumFallbackPrice
        self.localFallbackPrice = localFallbackPrice
        self.researchFallbackPrice = researchFallbackPrice
        self.ocrUSDPerPage = ocrUSDPerPage
        self.speechUSDPerMinute = speechUSDPerMinute
        self.ttsUSDPerThousandCharacters = ttsUSDPerThousandCharacters
        self.webFetchUSD = webFetchUSD
        self.substrateSearchUSD = substrateSearchUSD
    }

    public func price(for binding: CommonplaceParticipantBinding) -> CommonplaceModelPrice {
        modelPrices.first {
            $0.providerID == binding.providerID && $0.modelID == binding.modelID
        } ?? fallbackPrice(for: binding.costTier)
    }

    public func fallbackPrice(for tier: CommonplaceCostTier) -> CommonplaceModelPrice {
        switch tier {
        case .cheap:
            cheapFallbackPrice
        case .standard:
            standardFallbackPrice
        case .premium:
            premiumFallbackPrice
        case .local:
            localFallbackPrice
        case .research:
            researchFallbackPrice
        }
    }

    public static let commonplaceDraft = CommonplacePriceCard(
        id: "commonplace-draft-price-card-v1",
        sourceLabel: "Draft preview price card; refresh from hosted pricing before billing.",
        modelPrices: [
            CommonplaceModelPrice(
                providerID: "mistral",
                modelID: "mistral-medium-latest",
                inputUSDPerMillionTokens: 0.40,
                outputUSDPerMillionTokens: 2.00
            ),
            CommonplaceModelPrice(
                providerID: "qwen",
                modelID: "qwen3-coder-next",
                inputUSDPerMillionTokens: 0.10,
                outputUSDPerMillionTokens: 0.30
            ),
            CommonplaceModelPrice(
                providerID: "qwen",
                modelID: "qwen3-coder-480b-a35b-instruct",
                inputUSDPerMillionTokens: 2.40,
                outputUSDPerMillionTokens: 7.20
            ),
            CommonplaceModelPrice(
                providerID: "ai21",
                modelID: "jamba-large",
                inputUSDPerMillionTokens: 2.00,
                outputUSDPerMillionTokens: 8.00
            ),
        ],
        cheapFallbackPrice: CommonplaceModelPrice(
            providerID: "*",
            modelID: "cheap-fallback",
            inputUSDPerMillionTokens: 0.10,
            outputUSDPerMillionTokens: 0.30
        ),
        standardFallbackPrice: CommonplaceModelPrice(
            providerID: "*",
            modelID: "standard-fallback",
            inputUSDPerMillionTokens: 0.40,
            outputUSDPerMillionTokens: 2.00
        ),
        premiumFallbackPrice: CommonplaceModelPrice(
            providerID: "*",
            modelID: "premium-fallback",
            inputUSDPerMillionTokens: 2.00,
            outputUSDPerMillionTokens: 8.00
        ),
        localFallbackPrice: CommonplaceModelPrice(
            providerID: "*",
            modelID: "local-fallback",
            inputUSDPerMillionTokens: 0.00,
            outputUSDPerMillionTokens: 0.00
        ),
        researchFallbackPrice: CommonplaceModelPrice(
            providerID: "*",
            modelID: "research-fallback",
            inputUSDPerMillionTokens: 0.00,
            outputUSDPerMillionTokens: 0.00
        ),
        ocrUSDPerPage: 0.001,
        speechUSDPerMinute: 0.006,
        ttsUSDPerThousandCharacters: 0.015,
        webFetchUSD: 0.002,
        substrateSearchUSD: 0.001
    )
}

public struct CommonplaceModelPrice: Equatable, Sendable, Identifiable {
    public var id: String { "\(providerID):\(modelID)" }
    public var providerID: String
    public var modelID: String
    public var inputUSDPerMillionTokens: Double
    public var outputUSDPerMillionTokens: Double
    public var fixedUSDPerCall: Double

    public init(
        providerID: String,
        modelID: String,
        inputUSDPerMillionTokens: Double,
        outputUSDPerMillionTokens: Double,
        fixedUSDPerCall: Double = 0
    ) {
        self.providerID = providerID
        self.modelID = modelID
        self.inputUSDPerMillionTokens = inputUSDPerMillionTokens
        self.outputUSDPerMillionTokens = outputUSDPerMillionTokens
        self.fixedUSDPerCall = fixedUSDPerCall
    }

    public func rawUSD(for budget: CommonplaceTokenBudget) -> Double {
        fixedUSDPerCall
            + (Double(budget.inputTokens) / 1_000_000 * inputUSDPerMillionTokens)
            + (Double(budget.outputTokens) / 1_000_000 * outputUSDPerMillionTokens)
    }
}

public struct CommonplaceCreditEstimate: Equatable, Sendable {
    public var priceCardID: String
    public var policyID: String
    public var estimatedCredits: Int
    public var worstCaseCredits: Int
    public var requiresConfirmation: Bool
    public var estimatedRawUSD: Double
    public var worstCaseRawUSD: Double
    public var estimatedParticipantIDs: [String]
    public var worstCaseParticipantIDs: [String]
    public var estimatedModelLineItems: [CommonplaceCreditLineItem]
    public var worstCaseModelLineItems: [CommonplaceCreditLineItem]
    public var toolLineItems: [CommonplaceCreditLineItem]

    public init(
        priceCardID: String,
        policyID: String,
        estimatedCredits: Int,
        worstCaseCredits: Int,
        requiresConfirmation: Bool,
        estimatedRawUSD: Double,
        worstCaseRawUSD: Double,
        estimatedParticipantIDs: [String],
        worstCaseParticipantIDs: [String],
        estimatedModelLineItems: [CommonplaceCreditLineItem],
        worstCaseModelLineItems: [CommonplaceCreditLineItem],
        toolLineItems: [CommonplaceCreditLineItem]
    ) {
        self.priceCardID = priceCardID
        self.policyID = policyID
        self.estimatedCredits = estimatedCredits
        self.worstCaseCredits = worstCaseCredits
        self.requiresConfirmation = requiresConfirmation
        self.estimatedRawUSD = estimatedRawUSD
        self.worstCaseRawUSD = worstCaseRawUSD
        self.estimatedParticipantIDs = estimatedParticipantIDs
        self.worstCaseParticipantIDs = worstCaseParticipantIDs
        self.estimatedModelLineItems = estimatedModelLineItems
        self.worstCaseModelLineItems = worstCaseModelLineItems
        self.toolLineItems = toolLineItems
    }

    public var creditRangeLabel: String {
        if estimatedCredits == worstCaseCredits {
            return "\(estimatedCredits) credits"
        }
        return "\(estimatedCredits)-\(worstCaseCredits) credits"
    }
}

public struct CommonplaceCreditLineItem: Equatable, Sendable, Identifiable {
    public var id: String
    public var label: String
    public var category: CommonplaceCreditLineItemCategory
    public var rawUSD: Double
    public var detail: String

    public init(
        id: String,
        label: String,
        category: CommonplaceCreditLineItemCategory,
        rawUSD: Double,
        detail: String
    ) {
        self.id = id
        self.label = label
        self.category = category
        self.rawUSD = rawUSD
        self.detail = detail
    }
}

public enum CommonplaceCreditLineItemCategory: String, Equatable, Sendable {
    case modelTokens = "model_tokens"
    case ocr
    case speechToText = "speech_to_text"
    case textToSpeech = "text_to_speech"
    case webFetch = "web_fetch"
    case substrateSearch = "substrate_search"
}
