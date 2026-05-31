import Foundation

/// Which projection the director chose, plus resolved params.
/// Mirrors `scene_os_core::package::ProjectionBinding`. `params` omitted when empty.
public struct ProjectionBinding: Codable, Hashable, Sendable {
    public let id: String
    public let params: [String: JSONValue]

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

/// Which chrome shell the director chose.
/// Mirrors `scene_os_core::package::ChromeBinding`.
public struct ChromeBinding: Codable, Hashable, Sendable {
    public let id: String
    public let params: [String: JSONValue]

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

/// A transition hint to the choreographer.
/// Mirrors `scene_os_core::package::TransitionDescriptor`. The source-package id
/// is keyed `from` in JSON (serde rename); omitted when absent.
public struct TransitionDescriptor: Codable, Hashable, Sendable {
    public let fromPackageID: String?
    public let choreography: String

    public init(fromPackageID: String? = nil, choreography: String) {
        self.fromPackageID = fromPackageID
        self.choreography = choreography
    }

    enum CodingKeys: String, CodingKey {
        case fromPackageID = "from"
        case choreography
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        fromPackageID = try c.decodeIfPresent(String.self, forKey: .fromPackageID)
        choreography = try c.decode(String.self, forKey: .choreography)
    }
}

/// A frozen artifact emitted at pause / save.
/// Mirrors `scene_os_core::package::TerminalStateArtifact` (camelCase). The JSON
/// payload is keyed `json` (serde rename); `sourceRefs` omitted when empty.
public struct TerminalStateArtifact: Codable, Hashable, Sendable {
    public let svg: String?
    public let jsonPayload: [String: JSONValue]?
    public let sourceRefs: [[String: JSONValue]]

    public init(
        svg: String? = nil,
        jsonPayload: [String: JSONValue]? = nil,
        sourceRefs: [[String: JSONValue]] = []
    ) {
        self.svg = svg
        self.jsonPayload = jsonPayload
        self.sourceRefs = sourceRefs
    }

    enum CodingKeys: String, CodingKey {
        case svg
        case jsonPayload = "json"
        case sourceRefs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        svg = try c.decodeIfPresent(String.self, forKey: .svg)
        jsonPayload = try c.decodeIfPresent([String: JSONValue].self, forKey: .jsonPayload)
        sourceRefs = try c.decodeIfPresent([[String: JSONValue]].self, forKey: .sourceRefs) ?? []
    }
}

/// A proposed user action on the scene (e.g. open-evidence, save, ask-follow-up).
/// Mirrors `scene_os_core::package::ActionDescriptor` (camelCase). `payload`
/// omitted when empty; `requiresConfirmation` omitted when false; `proposalOnly`
/// defaults true.
public struct ActionDescriptor: Codable, Hashable, Sendable, Identifiable {
    public let id: String
    public let label: String
    public let actionType: String
    public let interaction: String
    public let target: String?
    public let payload: [String: JSONValue]
    public let requiresConfirmation: Bool
    public let proposalOnly: Bool

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

/// The typed scene package the director emits and every renderer consumes.
/// Mirrors `scene_os_core::package::ScenePackageV2` (camelCase). Optional
/// `transitions`/`terminalState` omitted when absent; `provenance` omitted when
/// empty; `actions` defaults to [].
public struct ScenePackageV2: Codable, Hashable, Sendable, Identifiable {
    public static let version = "scene-package-v2"

    public let version: String
    public let id: String
    public let manifestRef: String
    public let atoms: [SceneAtom]
    public let relations: [SceneRelation]
    public let projection: ProjectionBinding
    public let chrome: ChromeBinding
    public let actions: [ActionDescriptor]
    public let transitions: TransitionDescriptor?
    public let terminalState: TerminalStateArtifact?
    public let provenance: [String: JSONValue]

    public init(
        version: String = ScenePackageV2.version,
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
        version = try c.decodeIfPresent(String.self, forKey: .version) ?? ScenePackageV2.version
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

public extension ScenePackageV2 {
    /// Decode a package from the wire JSON string the hosted API / UniFFI layer
    /// hands the client.
    static func decode(from json: Data) throws -> ScenePackageV2 {
        try JSONDecoder().decode(ScenePackageV2.self, from: json)
    }
}
