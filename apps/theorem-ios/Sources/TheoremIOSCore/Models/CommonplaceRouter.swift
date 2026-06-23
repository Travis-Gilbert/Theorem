import Foundation

public struct CommonplaceRoutePlan: Equatable, Sendable, Identifiable {
    public var id: String
    public var query: String
    public var features: [CommonplaceRouteFeature]
    public var stages: [CommonplaceRouteStage]
    public var charterPrompt: String

    public init(
        id: String,
        query: String,
        features: [CommonplaceRouteFeature],
        stages: [CommonplaceRouteStage],
        charterPrompt: String
    ) {
        self.id = id
        self.query = query
        self.features = features
        self.stages = stages
        self.charterPrompt = charterPrompt
    }

    public var activeParticipantIDs: [String] {
        stages.first?.participantIDs ?? []
    }

    public var plannedParticipantIDs: [String] {
        Array(stages.dropFirst().flatMap(\.participantIDs))
    }
}

public struct CommonplaceRouteStage: Equatable, Sendable, Identifiable {
    public var id: String
    public var phase: CommonplaceRoutePhase
    public var participantIDs: [String]
    public var trigger: CommonplaceRouteTrigger

    public init(
        id: String,
        phase: CommonplaceRoutePhase,
        participantIDs: [String],
        trigger: CommonplaceRouteTrigger
    ) {
        self.id = id
        self.phase = phase
        self.participantIDs = participantIDs
        self.trigger = trigger
    }
}

public enum CommonplaceRouteFeature: String, Equatable, Sendable, CaseIterable {
    case routine
    case code
    case longContext = "long_context"
    case browserAction = "browser_action"
    case document
    case reasoning
    case research
}

public enum CommonplaceRoutePhase: String, Equatable, Sendable {
    case warmStart = "warm_start"
    case broaden
    case escalate
}

public enum CommonplaceRouteTrigger: String, Equatable, Sendable {
    case immediate
    case lowConfidence = "low_confidence"
    case needsCoverage = "needs_coverage"
}

public struct CommonplaceRouter: Equatable, Sendable {
    public init() {}

    public func plan(query: String, registry: CommonplaceRegistry) -> CommonplaceRoutePlan {
        let clean = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let features = detectFeatures(in: clean)
        let required = requiredCapabilities(for: features)
        let available = registry.participantBindings.filter { binding in
            binding.availability == .available || (features.contains(.research) && binding.researchTrack)
        }

        let warm = selectWarmStart(from: available, required: required, features: features)
        let broaden = selectBroaden(from: available, excluding: warm, required: required, features: features)
        let escalation = selectEscalation(from: available, excluding: warm + broaden, required: required, features: features)

        let stages = [
            CommonplaceRouteStage(
                id: "warm-start",
                phase: .warmStart,
                participantIDs: warm,
                trigger: .immediate
            ),
            CommonplaceRouteStage(
                id: "broaden",
                phase: .broaden,
                participantIDs: broaden,
                trigger: .needsCoverage
            ),
            CommonplaceRouteStage(
                id: "escalate",
                phase: .escalate,
                participantIDs: escalation,
                trigger: .lowConfidence
            ),
        ].filter { !$0.participantIDs.isEmpty }

        return CommonplaceRoutePlan(
            id: stableRouteID(for: clean),
            query: clean,
            features: features,
            stages: stages,
            charterPrompt: registry.charter.composedPrompt
        )
    }

    private func detectFeatures(in query: String) -> [CommonplaceRouteFeature] {
        let lower = query.lowercased()
        var features = Set<CommonplaceRouteFeature>()
        if containsAny(lower, ["code", "repo", "swift", "rust", "python", "bug", "test", "compile", "ios", "rustyweb", "sceneos", "substrate"]) {
            features.insert(.code)
        }
        if containsAny(lower, ["long", "document", "paper", "dossier", "synthesis", "context"]) {
            features.insert(.longContext)
            features.insert(.document)
        }
        if containsAny(lower, ["browser", "click", "form", "site", "web app", "crawl"]) {
            features.insert(.browserAction)
        }
        if containsAny(lower, ["reason", "prove", "plan", "architecture", "strategy", "why"]) {
            features.insert(.reasoning)
        }
        if containsAny(lower, ["xlstm", "research", "recurrent", "experiment"]) {
            features.insert(.research)
        }
        if features.isEmpty {
            features.insert(.routine)
        }
        return CommonplaceRouteFeature.allCases.filter { features.contains($0) }
    }

    private func requiredCapabilities(for features: [CommonplaceRouteFeature]) -> Set<CommonplaceCapabilityTag> {
        var required: Set<CommonplaceCapabilityTag> = [.text]
        if features.contains(.code) { required.insert(.code) }
        if features.contains(.longContext) || features.contains(.document) { required.insert(.longContext) }
        if features.contains(.browserAction) { required.insert(.browserAction) }
        if features.contains(.research) {
            required.insert(.localResearch)
            required.insert(.recurrent)
        }
        return required
    }

    private func selectWarmStart(
        from bindings: [CommonplaceParticipantBinding],
        required: Set<CommonplaceCapabilityTag>,
        features: [CommonplaceRouteFeature]
    ) -> [String] {
        let candidates = ranked(bindings, required: required, preferCheap: true)
        let limit = features.contains(.routine) ? 2 : 3
        return Array(candidates.prefix(limit)).map(\.participantID)
    }

    private func selectBroaden(
        from bindings: [CommonplaceParticipantBinding],
        excluding ids: [String],
        required: Set<CommonplaceCapabilityTag>,
        features: [CommonplaceRouteFeature]
    ) -> [String] {
        let excluded = Set(ids)
        let candidates = ranked(bindings, required: required, preferCheap: false)
            .filter { !excluded.contains($0.participantID) && $0.costTier != .premium }
        return Array(candidates.prefix(features.contains(.routine) ? 1 : 2)).map(\.participantID)
    }

    private func selectEscalation(
        from bindings: [CommonplaceParticipantBinding],
        excluding ids: [String],
        required: Set<CommonplaceCapabilityTag>,
        features: [CommonplaceRouteFeature]
    ) -> [String] {
        guard !features.contains(.routine) || features.contains(.reasoning) else { return [] }
        let excluded = Set(ids)
        let candidates = ranked(bindings, required: required, preferCheap: false)
            .filter { !excluded.contains($0.participantID) && ($0.costTier == .premium || $0.researchTrack) }
        return Array(candidates.prefix(4)).map(\.participantID)
    }

    private func ranked(
        _ bindings: [CommonplaceParticipantBinding],
        required: Set<CommonplaceCapabilityTag>,
        preferCheap: Bool
    ) -> [CommonplaceParticipantBinding] {
        bindings
            .map { binding in
                (binding, score(binding, required: required, preferCheap: preferCheap))
            }
            .filter { $0.1 > 0 }
            .sorted {
                if $0.1 == $1.1 {
                    return $0.0.id < $1.0.id
                }
                return $0.1 > $1.1
            }
            .map(\.0)
    }

    private func score(
        _ binding: CommonplaceParticipantBinding,
        required: Set<CommonplaceCapabilityTag>,
        preferCheap: Bool
    ) -> Int {
        let tags = Set(binding.capabilityTags)
        guard tags.contains(.text) else { return 0 }
        let missingRequired = required.subtracting(tags)
        let capabilityScore = required.intersection(tags).count * 10 - missingRequired.count * 6
        let costScore = preferCheap ? cheapBias(binding.costTier) : coverageBias(binding.costTier)
        let toolScore = tags.contains(.toolCalling) ? 3 : 0
        let researchScore = binding.researchTrack ? (required.contains(.localResearch) ? 8 : -8) : 0
        return capabilityScore + costScore + toolScore + researchScore
    }

    private func cheapBias(_ tier: CommonplaceCostTier) -> Int {
        switch tier {
        case .cheap:
            8
        case .standard, .local:
            4
        case .premium:
            1
        case .research:
            -4
        }
    }

    private func coverageBias(_ tier: CommonplaceCostTier) -> Int {
        switch tier {
        case .premium:
            8
        case .standard:
            6
        case .cheap, .local:
            3
        case .research:
            0
        }
    }

    private func containsAny(_ text: String, _ needles: [String]) -> Bool {
        needles.contains { text.contains($0) }
    }

    private func stableRouteID(for query: String) -> String {
        let checksum = query.unicodeScalars.reduce(UInt64(0)) { partial, scalar in
            (partial &* 31) &+ UInt64(scalar.value)
        }
        return "route-\(checksum)"
    }
}
