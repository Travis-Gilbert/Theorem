import Foundation

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
}

public struct AtomPosition: Codable, Equatable, Sendable {
    public var x: Double
    public var y: Double
    public var z: Double
    public var space: String
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
}

public struct SourceRef: Codable, Equatable, Sendable {
    public var kind: String
    public var id: String
    public var label: String?
    public var metadata: [String: JSONValue]
}

public struct ProjectionBinding: Codable, Equatable, Sendable {
    public var id: String
    public var params: [String: JSONValue]

    public init(id: String, params: [String: JSONValue] = [:]) {
        self.id = id
        self.params = params
    }
}

public struct ChromeBinding: Codable, Equatable, Sendable {
    public var id: String
    public var params: [String: JSONValue]

    public init(id: String, params: [String: JSONValue] = [:]) {
        self.id = id
        self.params = params
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
}

public struct TransitionDescriptor: Codable, Equatable, Sendable {
    public var from: String?
    public var choreography: String
}

public struct TerminalStateArtifact: Codable, Equatable, Sendable {
    public var svg: String?
    public var json: [String: JSONValue]?
    public var sourceRefs: [[String: JSONValue]]
}
