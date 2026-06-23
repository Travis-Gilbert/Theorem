import Foundation

// Wire models for the ScenePackageV2 contract (scene-os-core, camelCase JSON,
// kebab-case lifecycle/space strings).
//
// IMPORTANT — omission safety: the Rust serde layer OMITS empty maps, empty
// arrays, and None options (`skip_serializing_if`). So `metadata`, `sourceRefs`,
// `actions`, `provenance`, `params`, and `AtomPosition.z` are frequently ABSENT
// from a real payload. Synthesized `Codable` would throw `keyNotFound` on those
// non-optional fields the moment a real `ScenePackageV2` is decoded (the sample
// scene is built in Swift, so this stays latent until the streaming client
// lands). Each struct therefore decodes via `decodeIfPresent ?? default`. Ported
// from the converged Swift lane's verified decoders.

public struct ScenePackageV2: Codable, Equatable, Sendable {
    public var version: String
    public var id: String
    public var manifestRef: String
    public var atoms: [SceneAtom]
    public var relations: [SceneRelation]
    public var projection: ProjectionBinding
    public var chrome: ChromeBinding
    public var actions: [ActionDescriptor]
    public var transitions: TransitionDescriptor?
    public var terminalState: TerminalStateArtifact?
    public var provenance: [String: JSONValue]

    public init(
        version: String = "scene-package-v2",
        id: String,
        manifestRef: String,
        atoms: [SceneAtom],
        relations: [SceneRelation],
        projection: ProjectionBinding,
        chrome: ChromeBinding,
        actions: [ActionDescriptor] = [],
        transitions: TransitionDescriptor? = nil,
        terminalState: TerminalStateArtifact? = nil,
        provenance: [String: JSONValue] = [:]
    ) {
        self.version = version
        self.id = id
        self.manifestRef = manifestRef
        self.atoms = atoms
        self.relations = relations
        self.projection = projection
        self.chrome = chrome
        self.actions = actions
        self.transitions = transitions
        self.terminalState = terminalState
        self.provenance = provenance
    }

    enum CodingKeys: String, CodingKey {
        case version, id, manifestRef, atoms, relations, projection, chrome
        case actions, transitions, terminalState, provenance
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        version = try c.decodeIfPresent(String.self, forKey: .version) ?? "scene-package-v2"
        id = try c.decode(String.self, forKey: .id)
        manifestRef = try c.decode(String.self, forKey: .manifestRef)
        atoms = try c.decode([SceneAtom].self, forKey: .atoms)
        relations = try c.decode([SceneRelation].self, forKey: .relations)
        projection = try c.decode(ProjectionBinding.self, forKey: .projection)
        chrome = try c.decode(ChromeBinding.self, forKey: .chrome)
        actions = try c.decodeIfPresent([ActionDescriptor].self, forKey: .actions) ?? []
        transitions = try c.decodeIfPresent(TransitionDescriptor.self, forKey: .transitions)
        terminalState = try c.decodeIfPresent(TerminalStateArtifact.self, forKey: .terminalState)
        provenance = try c.decodeIfPresent([String: JSONValue].self, forKey: .provenance) ?? [:]
    }
}

public struct SceneAtom: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var kind: String
    public var label: String?
    public var position: AtomPosition?
    public var weight: Double?
    public var color: String?
    public var opacity: Double?
    public var glyph: String?
    public var scale: Double?
    public var lifecycle: String
    public var metadata: [String: JSONValue]
    public var sourceRefs: [SourceRef]

    public init(
        id: String,
        kind: String = "evidence",
        label: String? = nil,
        position: AtomPosition? = nil,
        weight: Double? = nil,
        color: String? = nil,
        opacity: Double? = nil,
        glyph: String? = nil,
        scale: Double? = nil,
        lifecycle: String = "present",
        metadata: [String: JSONValue] = [:],
        sourceRefs: [SourceRef] = []
    ) {
        self.id = id
        self.kind = kind
        self.label = label
        self.position = position
        self.weight = weight
        self.color = color
        self.opacity = opacity
        self.glyph = glyph
        self.scale = scale
        self.lifecycle = lifecycle
        self.metadata = metadata
        self.sourceRefs = sourceRefs
    }

    enum CodingKeys: String, CodingKey {
        case id, kind, label, position, weight, color, opacity, glyph, scale
        case lifecycle, metadata, sourceRefs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? "evidence"
        label = try c.decodeIfPresent(String.self, forKey: .label)
        position = try c.decodeIfPresent(AtomPosition.self, forKey: .position)
        weight = try c.decodeIfPresent(Double.self, forKey: .weight)
        color = try c.decodeIfPresent(String.self, forKey: .color)
        opacity = try c.decodeIfPresent(Double.self, forKey: .opacity)
        glyph = try c.decodeIfPresent(String.self, forKey: .glyph)
        scale = try c.decodeIfPresent(Double.self, forKey: .scale)
        lifecycle = try c.decodeIfPresent(String.self, forKey: .lifecycle) ?? "present"
        metadata = try c.decodeIfPresent([String: JSONValue].self, forKey: .metadata) ?? [:]
        sourceRefs = try c.decodeIfPresent([SourceRef].self, forKey: .sourceRefs) ?? []
    }
}

public struct AtomPosition: Codable, Equatable, Sendable {
    public var x: Double
    public var y: Double
    public var z: Double
    public var space: String

    public init(x: Double, y: Double, z: Double = 0, space: String = "freeform") {
        self.x = x
        self.y = y
        self.z = z
        self.space = space
    }

    enum CodingKeys: String, CodingKey { case x, y, z, space }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        x = try c.decode(Double.self, forKey: .x)
        y = try c.decode(Double.self, forKey: .y)
        z = try c.decodeIfPresent(Double.self, forKey: .z) ?? 0
        space = try c.decodeIfPresent(String.self, forKey: .space) ?? "freeform"
    }
}

public struct SceneRelation: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var sourceId: String
    public var targetId: String
    public var kind: String
    public var weight: Double?
    public var color: String?
    public var opacity: Double?
    public var glyph: String?
    public var lifecycle: String
    public var metadata: [String: JSONValue]
    public var sourceRefs: [SourceRef]

    public init(
        id: String,
        sourceId: String,
        targetId: String,
        kind: String = "related",
        weight: Double? = nil,
        color: String? = nil,
        opacity: Double? = nil,
        glyph: String? = nil,
        lifecycle: String = "present",
        metadata: [String: JSONValue] = [:],
        sourceRefs: [SourceRef] = []
    ) {
        self.id = id
        self.sourceId = sourceId
        self.targetId = targetId
        self.kind = kind
        self.weight = weight
        self.color = color
        self.opacity = opacity
        self.glyph = glyph
        self.lifecycle = lifecycle
        self.metadata = metadata
        self.sourceRefs = sourceRefs
    }

    enum CodingKeys: String, CodingKey {
        case id, sourceId, targetId, kind, weight, color, opacity, glyph
        case lifecycle, metadata, sourceRefs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        sourceId = try c.decode(String.self, forKey: .sourceId)
        targetId = try c.decode(String.self, forKey: .targetId)
        kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? "related"
        weight = try c.decodeIfPresent(Double.self, forKey: .weight)
        color = try c.decodeIfPresent(String.self, forKey: .color)
        opacity = try c.decodeIfPresent(Double.self, forKey: .opacity)
        glyph = try c.decodeIfPresent(String.self, forKey: .glyph)
        lifecycle = try c.decodeIfPresent(String.self, forKey: .lifecycle) ?? "present"
        metadata = try c.decodeIfPresent([String: JSONValue].self, forKey: .metadata) ?? [:]
        sourceRefs = try c.decodeIfPresent([SourceRef].self, forKey: .sourceRefs) ?? []
    }
}

public struct SourceRef: Codable, Equatable, Sendable {
    public var kind: String
    public var id: String
    public var label: String?
    public var metadata: [String: JSONValue]

    public init(kind: String, id: String, label: String? = nil, metadata: [String: JSONValue] = [:]) {
        self.kind = kind
        self.id = id
        self.label = label
        self.metadata = metadata
    }

    enum CodingKeys: String, CodingKey { case kind, id, label, metadata }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decode(String.self, forKey: .kind)
        id = try c.decode(String.self, forKey: .id)
        label = try c.decodeIfPresent(String.self, forKey: .label)
        metadata = try c.decodeIfPresent([String: JSONValue].self, forKey: .metadata) ?? [:]
    }
}

public struct ProjectionBinding: Codable, Equatable, Sendable {
    public var id: String
    public var params: [String: JSONValue]

    public init(id: String, params: [String: JSONValue] = [:]) {
        self.id = id
        self.params = params
    }

    enum CodingKeys: String, CodingKey { case id, params }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        params = try c.decodeIfPresent([String: JSONValue].self, forKey: .params) ?? [:]
    }
}

public struct ChromeBinding: Codable, Equatable, Sendable {
    public var id: String
    public var params: [String: JSONValue]

    public init(id: String, params: [String: JSONValue] = [:]) {
        self.id = id
        self.params = params
    }

    enum CodingKeys: String, CodingKey { case id, params }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        params = try c.decodeIfPresent([String: JSONValue].self, forKey: .params) ?? [:]
    }
}

public struct ActionDescriptor: Codable, Equatable, Sendable {
    public var id: String
    public var label: String
    public var actionType: String
    public var interaction: String
    public var target: String?
    public var payload: [String: JSONValue]
    public var requiresConfirmation: Bool
    public var proposalOnly: Bool

    public init(
        id: String,
        label: String,
        actionType: String,
        interaction: String,
        target: String? = nil,
        payload: [String: JSONValue] = [:],
        requiresConfirmation: Bool = false,
        proposalOnly: Bool = true
    ) {
        self.id = id
        self.label = label
        self.actionType = actionType
        self.interaction = interaction
        self.target = target
        self.payload = payload
        self.requiresConfirmation = requiresConfirmation
        self.proposalOnly = proposalOnly
    }

    enum CodingKeys: String, CodingKey {
        case id, label, actionType, interaction, target, payload
        case requiresConfirmation, proposalOnly
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        label = try c.decode(String.self, forKey: .label)
        actionType = try c.decode(String.self, forKey: .actionType)
        interaction = try c.decode(String.self, forKey: .interaction)
        target = try c.decodeIfPresent(String.self, forKey: .target)
        payload = try c.decodeIfPresent([String: JSONValue].self, forKey: .payload) ?? [:]
        requiresConfirmation = try c.decodeIfPresent(Bool.self, forKey: .requiresConfirmation) ?? false
        proposalOnly = try c.decodeIfPresent(Bool.self, forKey: .proposalOnly) ?? true
    }
}

public struct TransitionDescriptor: Codable, Equatable, Sendable {
    public var from: String?
    public var choreography: String

    public init(from: String? = nil, choreography: String) {
        self.from = from
        self.choreography = choreography
    }

    enum CodingKeys: String, CodingKey { case from, choreography }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        from = try c.decodeIfPresent(String.self, forKey: .from)
        choreography = try c.decode(String.self, forKey: .choreography)
    }
}

public struct TerminalStateArtifact: Codable, Equatable, Sendable {
    public var svg: String?
    public var json: [String: JSONValue]?
    public var sourceRefs: [[String: JSONValue]]

    public init(svg: String? = nil, json: [String: JSONValue]? = nil, sourceRefs: [[String: JSONValue]] = []) {
        self.svg = svg
        self.json = json
        self.sourceRefs = sourceRefs
    }

    enum CodingKeys: String, CodingKey { case svg, json, sourceRefs }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        svg = try c.decodeIfPresent(String.self, forKey: .svg)
        json = try c.decodeIfPresent([String: JSONValue].self, forKey: .json)
        sourceRefs = try c.decodeIfPresent([[String: JSONValue]].self, forKey: .sourceRefs) ?? []
    }
}
