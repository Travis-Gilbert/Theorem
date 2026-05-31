import Foundation

/// Coordinate system a placement maps atoms into.
/// Mirrors `scene_os_core::atoms::CoordinateSpace` (kebab-case; all single-word
/// so the raw values equal the case names).
public enum CoordinateSpace: String, Codable, Sendable, CaseIterable {
    case graph, geo, timeline, rank, matrix, diagram, frame, gallery, freeform
}

/// Phase an atom is in within a substrate frame.
/// Mirrors `scene_os_core::atoms::AtomLifecycle` (kebab-case).
public enum AtomLifecycle: String, Codable, Sendable {
    case entering, present, leaving, terminal
}

/// Placement of one atom in a coordinate space.
/// Mirrors `scene_os_core::atoms::AtomPosition`. `z` is omitted from JSON when
/// zero; `space` defaults to `freeform`.
public struct AtomPosition: Codable, Hashable, Sendable {
    public let x: Double
    public let y: Double
    public let z: Double
    public let space: CoordinateSpace

    public init(x: Double, y: Double, z: Double = 0, space: CoordinateSpace = .freeform) {
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
        space = try c.decodeIfPresent(CoordinateSpace.self, forKey: .space) ?? .freeform
    }
}

/// Pointer back to the canonical record an atom was projected from.
/// Mirrors `scene_os_core::atoms::SourceRef` (camelCase). `metadata` is omitted
/// from JSON when empty.
public struct SourceRef: Codable, Hashable, Sendable {
    public let kind: String
    public let id: String
    public let label: String?
    public let metadata: [String: JSONValue]

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

/// A single visual element on the substrate.
/// Mirrors `scene_os_core::atoms::SceneAtom` (camelCase). Optional visual
/// attributes are omitted from JSON when `None`; `metadata`/`sourceRefs` are
/// omitted when empty; `kind` defaults to "evidence", `lifecycle` to "present".
public struct SceneAtom: Codable, Hashable, Sendable, Identifiable {
    public let id: String
    public let kind: String
    public let label: String?
    public let position: AtomPosition?
    public let weight: Double?
    public let color: String?
    public let opacity: Double?
    public let glyph: String?
    public let scale: Double?
    public let lifecycle: AtomLifecycle
    public let metadata: [String: JSONValue]
    public let sourceRefs: [SourceRef]

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
        lifecycle: AtomLifecycle = .present,
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
        lifecycle = try c.decodeIfPresent(AtomLifecycle.self, forKey: .lifecycle) ?? .present
        metadata = try c.decodeIfPresent([String: JSONValue].self, forKey: .metadata) ?? [:]
        sourceRefs = try c.decodeIfPresent([SourceRef].self, forKey: .sourceRefs) ?? []
    }

    /// PPR mass / weight for radius scaling, read from `weight` then a
    /// `match_score` carried in metadata (the search-derived path), else 1.
    public var magnitude: Double {
        if let weight, weight > 0 { return weight }
        if let score = metadata["match_score"]?.doubleValue, score > 0 { return score }
        if let scale, scale > 0 { return scale }
        return 1
    }

    /// Ring (hop distance from match) when this atom came from a SubstrateSearch
    /// hit, read from metadata. `nil` when the scene carries no ring structure.
    public var ring: Int? {
        if let value = metadata["ring"]?.doubleValue { return Int(value) }
        return nil
    }
}

/// An edge between two atoms.
/// Mirrors `scene_os_core::atoms::SceneRelation` (camelCase: `sourceId`,
/// `targetId`, `sourceRefs`). `kind` defaults to "related".
public struct SceneRelation: Codable, Hashable, Sendable, Identifiable {
    public let id: String
    public let sourceId: String
    public let targetId: String
    public let kind: String
    public let weight: Double?
    public let color: String?
    public let opacity: Double?
    public let glyph: String?
    public let lifecycle: AtomLifecycle
    public let metadata: [String: JSONValue]
    public let sourceRefs: [SourceRef]

    public init(
        id: String,
        sourceId: String,
        targetId: String,
        kind: String = "related",
        weight: Double? = nil,
        color: String? = nil,
        opacity: Double? = nil,
        glyph: String? = nil,
        lifecycle: AtomLifecycle = .present,
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
        lifecycle = try c.decodeIfPresent(AtomLifecycle.self, forKey: .lifecycle) ?? .present
        metadata = try c.decodeIfPresent([String: JSONValue].self, forKey: .metadata) ?? [:]
        sourceRefs = try c.decodeIfPresent([SourceRef].self, forKey: .sourceRefs) ?? []
    }
}
