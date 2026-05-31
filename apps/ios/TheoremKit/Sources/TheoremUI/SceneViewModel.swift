import Foundation
import Observation
import TheoremKit

/// The brain behind the scene surface: holds the current scene, runs the
/// reprojection sliver to switch projections, and tracks which projections are
/// honestly available. The switcher and renderers read this; nothing else owns
/// scene state.
///
/// `@MainActor` because it is bound to SwiftUI views. The engine is `Sendable`
/// so the view model can be initialised with either the stub (now) or the Rust
/// UniFFI client (once the xcframework lands) with no other change.
@MainActor
@Observable
public final class SceneViewModel {
    public private(set) var scene: ScenePackageV2?
    public private(set) var availableProjections: [ProjectionAvailability] = []
    public private(set) var selectedProjectionID: String = ProjectionID.forceGraph
    /// Laid-out positions for the current projection (nil for force_graph, which
    /// Grape lays out itself, and nil when a projection was rejected).
    public private(set) var layout: ReprojectResult?
    /// Set when the last `select` hit an unavailable projection — the honest
    /// reason, surfaced instead of a fabricated view.
    public private(set) var rejectionReason: String?
    public private(set) var centerNodeID: String?
    /// Streamed push-trace for the fractal projection (nil until the server
    /// streams one). The view never fabricates a trace.
    public private(set) var pushTrace: PushTrace?
    public var selectedAtomID: String?

    private let engine: ReprojectionEngine

    public init(engine: ReprojectionEngine = StubReprojectionEngine()) {
        self.engine = engine
    }

    /// Load a scene: compute availability, the center node, and select the
    /// director's projection (or the first available one).
    public func load(_ scene: ScenePackageV2) {
        self.scene = scene
        self.selectedAtomID = nil
        self.pushTrace = nil
        self.availableProjections = engine.availableProjections(scene)
        self.centerNodeID = engine.centerNodeID(scene, mode: .pprMass)

        let directorChoice = scene.projection.id
        let chosen = availableProjections.first { $0.projectionID == directorChoice && $0.available }?.projectionID
            ?? availableProjections.first { $0.available }?.projectionID
            ?? ProjectionID.forceGraph
        select(chosen)
    }

    /// Switch projections via the sliver. On an unavailable projection the layout
    /// is cleared and `rejectionReason` is set (the honest-shape path); the UI
    /// should have greyed it, so this is a defensive backstop.
    public func select(_ projectionID: String) {
        selectedProjectionID = projectionID
        guard let scene else { layout = nil; return }
        do {
            layout = try engine.reproject(scene, projectionID: projectionID)
            rejectionReason = nil
        } catch let ReprojectError.shapeRejected(_, reason) {
            layout = nil
            rejectionReason = reason
        } catch {
            layout = nil
            rejectionReason = String(describing: error)
        }
    }

    public func availability(for projectionID: String) -> ProjectionAvailability? {
        availableProjections.first { $0.projectionID == projectionID }
    }

    /// Title of the center node ("the center of what you searched") for the
    /// Dynamic Island readout.
    public var centerNodeTitle: String? {
        guard let id = centerNodeID,
              let atom = scene?.atoms.first(where: { $0.id == id }) else { return nil }
        return atom.label ?? atom.id
    }

    public func setPushTrace(_ trace: PushTrace?) {
        pushTrace = trace
    }
}
