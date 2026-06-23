import Foundation

/// A 2D point in the projection's virtual layout space. The renderer fits the
/// bounding box of all points to the canvas (the substrate camera), so absolute
/// values only matter relative to each other.
public struct LayoutPoint: Hashable, Sendable {
    public var x: Double
    public var y: Double
    public init(x: Double, y: Double) {
        self.x = x
        self.y = y
    }
}

/// The four v1 projection ids (spec "The algorithms to ship in v1"). These are
/// the catalog ids the Rust director and the Swift renderers agree on.
public enum ProjectionID {
    public static let forceGraph = "force_graph"
    public static let radialRings = "radial_rings"
    public static let treeLayout = "tree_layout"
    public static let fractalExpansion = "fractal_expansion"

    public static let all = [forceGraph, radialRings, treeLayout, fractalExpansion]

    public static func label(_ id: String) -> String {
        switch id {
        case forceGraph: return "Force graph"
        case radialRings: return "Radial rings"
        case treeLayout: return "Tree"
        case fractalExpansion: return "Fractal expansion"
        default: return id
        }
    }
}

/// Result of laying out the current scene through a projection: a position per
/// atom in the projection's coordinate space. Layout-only — it never adds or
/// removes atoms (the on-device sliver is a layout-swapper, not a
/// shape-fabricator; spec "Architecture decision").
public struct ReprojectResult: Sendable, Hashable {
    public let projectionID: String
    public let coordinateSpace: CoordinateSpace
    public let positions: [String: LayoutPoint]

    public init(projectionID: String, coordinateSpace: CoordinateSpace, positions: [String: LayoutPoint]) {
        self.projectionID = projectionID
        self.coordinateSpace = coordinateSpace
        self.positions = positions
    }
}

/// Whether a projection is honestly available for the current scene, and if not,
/// the one-line reason the switcher shows on long-press ("links form a cycle —
/// no tree"). This is the honest-shape feature surfaced as data.
public struct ProjectionAvailability: Sendable, Hashable, Identifiable {
    public let projectionID: String
    public let label: String
    public let available: Bool
    /// Why the projection is unavailable. `nil` when available.
    public let reason: String?

    public var id: String { projectionID }

    public init(projectionID: String, label: String, available: Bool, reason: String?) {
        self.projectionID = projectionID
        self.label = label
        self.available = available
        self.reason = reason
    }
}

/// Centrality mode for naming "the center of what you searched" (spec
/// "The Dynamic Island"). PPR mass is the substrate signal already in the
/// payload; degree is the cheap fallback.
public enum CentralityMode: Sendable {
    case pprMass
    case degree
}

/// Errors a reprojection can raise. `shapeRejected` is the honest-shape path:
/// the requested projection's detect-shape test fails for this data, so the
/// projection cannot be laid out (and the UI should have greyed it).
public enum ReprojectError: Error, Sendable, Equatable {
    case unknownProjection(String)
    case shapeRejected(projectionID: String, reason: String)
    case emptyScene
}

/// The on-device reprojection sliver's contract — the Swift seam over the Rust
/// UniFFI surface (spec "The Rust ↔ Swift bridge"). The UniFFI-generated client
/// is one implementation; `StubReprojectionEngine` is the pure-Swift dev
/// implementation so the renderers build before the `.xcframework` lands.
///
/// All three operations are bounded (they touch only the current scene's atoms,
/// tens to low hundreds) and never reach the corpus.
public protocol ReprojectionEngine: Sendable {
    /// Lay out the current scene into a different projection. Throws
    /// `ReprojectError.shapeRejected` if the projection's data shape is absent.
    func reproject(_ scene: ScenePackageV2, projectionID: String) throws -> ReprojectResult

    /// Which projections this scene's data honestly supports (runs each
    /// projection's detect-shape test).
    func availableProjections(_ scene: ScenePackageV2) -> [ProjectionAvailability]

    /// The center node id by PPR mass (or degree fallback). `nil` for an empty
    /// scene.
    func centerNodeID(_ scene: ScenePackageV2, mode: CentralityMode) -> String?
}
