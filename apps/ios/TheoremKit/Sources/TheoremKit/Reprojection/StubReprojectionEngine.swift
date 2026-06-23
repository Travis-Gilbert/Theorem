import Foundation

/// Pure-Swift reprojection engine: the dev implementation of the sliver that
/// lets the renderers, switcher, and Dynamic Island build and run before the
/// Rust UniFFI `.xcframework` lands. It computes the SAME honest-shape decisions
/// the Rust `detect_shape` arms will (cyclic links -> tree unavailable, etc.),
/// so swapping in the UniFFI client later changes the implementation, not the
/// behaviour the UI was built against.
///
/// Layout-only: it never adds or removes atoms. `detect` is the single source of
/// truth for availability, so `reproject` and `availableProjections` can never
/// disagree (the switcher greys exactly what `reproject` would reject).
public struct StubReprojectionEngine: ReprojectionEngine {
    public init() {}

    // MARK: Availability (the honest-shape feature)

    public func availableProjections(_ scene: ScenePackageV2) -> [ProjectionAvailability] {
        let topology = GraphTopology(atoms: scene.atoms, relations: scene.relations)
        return ProjectionID.all.map { detect(scene, topology: topology, projectionID: $0) }
    }

    // MARK: Reprojection (layout-only)

    public func reproject(_ scene: ScenePackageV2, projectionID: String) throws -> ReprojectResult {
        guard ProjectionID.all.contains(projectionID) else {
            throw ReprojectError.unknownProjection(projectionID)
        }
        guard !scene.atoms.isEmpty else { throw ReprojectError.emptyScene }

        let topology = GraphTopology(atoms: scene.atoms, relations: scene.relations)
        let availability = detect(scene, topology: topology, projectionID: projectionID)
        guard availability.available else {
            throw ReprojectError.shapeRejected(
                projectionID: projectionID,
                reason: availability.reason ?? "data shape not present"
            )
        }

        switch projectionID {
        case ProjectionID.forceGraph:
            return ReprojectResult(
                projectionID: projectionID, coordinateSpace: .graph,
                positions: Layouts.forceSeed(atoms: scene.atoms))

        case ProjectionID.radialRings:
            return ReprojectResult(
                projectionID: projectionID, coordinateSpace: .graph,
                positions: Layouts.radialRings(atoms: scene.atoms))

        case ProjectionID.treeLayout:
            let root = centerNodeID(scene, mode: .pprMass) ?? scene.atoms[0].id
            return ReprojectResult(
                projectionID: projectionID, coordinateSpace: .diagram,
                positions: Layouts.tidyTree(topology: topology, root: root))

        case ProjectionID.fractalExpansion:
            // Base layout; the push-trace wavefront animates over it (the trace
            // is streamed from the server, where the real push_ppr runs).
            return ReprojectResult(
                projectionID: projectionID, coordinateSpace: .graph,
                positions: Layouts.forceSeed(atoms: scene.atoms))

        default:
            throw ReprojectError.unknownProjection(projectionID)
        }
    }

    // MARK: Centrality (the Dynamic Island readout)

    public func centerNodeID(_ scene: ScenePackageV2, mode: CentralityMode) -> String? {
        guard !scene.atoms.isEmpty else { return nil }
        switch mode {
        case .pprMass:
            // Highest PPR mass; tie-break on the smallest id for determinism.
            return scene.atoms.max { lhs, rhs in
                lhs.magnitude != rhs.magnitude ? lhs.magnitude < rhs.magnitude : lhs.id > rhs.id
            }?.id
        case .degree:
            let topology = GraphTopology(atoms: scene.atoms, relations: scene.relations)
            return scene.atoms.map(\.id).max { lhs, rhs in
                let dl = topology.degree(of: lhs)
                let dr = topology.degree(of: rhs)
                return dl != dr ? dl < dr : lhs > rhs
            }
        }
    }

    // MARK: detect_shape (mirrors the Rust arms)

    private func detect(
        _ scene: ScenePackageV2,
        topology: GraphTopology,
        projectionID: String
    ) -> ProjectionAvailability {
        let label = ProjectionID.label(projectionID)
        let atomCount = scene.atoms.count
        let edgeCount = topology.edgeCount

        func result(_ available: Bool, _ reason: String) -> ProjectionAvailability {
            ProjectionAvailability(
                projectionID: projectionID, label: label,
                available: available, reason: available ? nil : reason)
        }

        switch projectionID {
        case ProjectionID.forceGraph:
            // Any scene with >= 2 atoms and >= 1 relation (spec algo 1).
            return result(atomCount >= 2 && edgeCount >= 1, "needs at least 2 nodes and a link")

        case ProjectionID.radialRings:
            // Any scene whose atoms carry a ring (a SubstrateSearch-derived
            // scene; spec algo 2).
            let hasRing = scene.atoms.contains { $0.ring != nil }
            return result(hasRing, "no ring data — not a search neighbourhood")

        case ProjectionID.treeLayout:
            // BFS from the PPR-center must yield a valid tree/forest: the link
            // graph must be acyclic (spec algo 3, the honest-shape rule).
            if edgeCount == 0 { return result(false, "no links to form a tree") }
            return result(topology.isForest(), "links form a cycle — no tree")

        case ProjectionID.fractalExpansion:
            // Relations + at least one seed (the matched nodes; spec algo 4).
            return result(edgeCount >= 1 && atomCount >= 2, "no links to expand through")

        default:
            return result(false, "unknown projection")
        }
    }
}
