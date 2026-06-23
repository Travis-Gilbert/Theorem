import Foundation

/// Pure placement functions. Each maps the current scene's atoms to positions in
/// a virtual layout space; the renderer fits the bounding box to the canvas.
/// Every function is deterministic (total orderings, no randomness) so a
/// reprojection is byte-reproducible — the same honest layout every time.
enum Layouts {

    /// Concentric orbits by `ring` (spec algo 2). Ring-0 (matches) at the
    /// center, ring-1 on the first orbit, etc. Within a ring, atoms are ordered
    /// by `match_score` (magnitude) descending and distributed angularly.
    /// Ring-less atoms (no `ring` metadata) occupy an outer "browse" orbit so
    /// they remain visible and stable rather than dropped.
    static func radialRings(atoms: [SceneAtom], orbitSpacing: Double = 150) -> [String: LayoutPoint] {
        guard !atoms.isEmpty else { return [:] }

        let knownRings = atoms.compactMap(\.ring)
        let outerRing = (knownRings.max() ?? 0) + 1

        var byRing: [Int: [SceneAtom]] = [:]
        for atom in atoms {
            byRing[atom.ring ?? outerRing, default: []].append(atom)
        }

        var positions: [String: LayoutPoint] = [:]
        for (ring, group) in byRing {
            // Total order: magnitude desc, then id asc (deterministic without
            // relying on sort stability).
            let ordered = group.sorted { lhs, rhs in
                lhs.magnitude != rhs.magnitude ? lhs.magnitude > rhs.magnitude : lhs.id < rhs.id
            }
            // A lone center node sits at the origin; otherwise everything orbits.
            if ring == 0, ordered.count == 1 {
                positions[ordered[0].id] = LayoutPoint(x: 0, y: 0)
                continue
            }
            let radius = ring == 0 ? orbitSpacing * 0.45 : Double(ring) * orbitSpacing
            let count = ordered.count
            for (index, atom) in ordered.enumerated() {
                let angle = (2 * Double.pi * Double(index) / Double(count)) - (Double.pi / 2)
                positions[atom.id] = LayoutPoint(x: radius * cos(angle), y: radius * sin(angle))
            }
        }
        return positions
    }

    /// Tidy rooted-tree layout (spec algo 3), Reingold-Tilford style: depth maps
    /// to y, an in-order leaf sweep maps to x (internal nodes centered over their
    /// children). The caller has already verified the link graph is a forest;
    /// `root` is the PPR-center node. Atoms in other components (or unreachable)
    /// are placed in a stable fallback row beneath the tree so they stay visible.
    static func tidyTree(
        topology: GraphTopology,
        root: String,
        xGap: Double = 130,
        yGap: Double = 130
    ) -> [String: LayoutPoint] {
        let parents = topology.bfsParents(root: root)

        var children: [String: [String]] = [:]
        for (child, parent) in parents {
            children[parent, default: []].append(child)
        }
        // Deterministic child order.
        for key in children.keys { children[key]?.sort() }

        var positions: [String: LayoutPoint] = [:]
        var nextLeafX = 0.0
        var maxDepth = 0

        func assign(_ node: String, depth: Int) -> Double {
            maxDepth = max(maxDepth, depth)
            let kids = children[node] ?? []
            let x: Double
            if kids.isEmpty {
                x = nextLeafX
                nextLeafX += 1
            } else {
                let childXs = kids.map { assign($0, depth: depth + 1) }
                x = childXs.reduce(0, +) / Double(childXs.count)
            }
            positions[node] = LayoutPoint(x: x * xGap, y: Double(depth) * yGap)
            return x
        }
        _ = assign(root, depth: 0)

        // Other forest components / unreachable atoms: a stable fallback row.
        let reached = Set(positions.keys)
        let orphans = topology.atomIDs.filter { !reached.contains($0) }
        if !orphans.isEmpty {
            let rowY = Double(maxDepth + 1) * yGap
            for (index, id) in orphans.enumerated() {
                positions[id] = LayoutPoint(x: Double(index) * xGap, y: rowY)
            }
        }
        return positions
    }

    /// Deterministic ring seed for the force graph (spec algo 1). The renderer's
    /// Grape force simulation re-settles from this seed, so the sliver only needs
    /// to provide stable, non-degenerate starting positions (not a final layout).
    static func forceSeed(atoms: [SceneAtom], radius: Double = 220) -> [String: LayoutPoint] {
        let ids = atoms.map(\.id).sorted()
        guard !ids.isEmpty else { return [:] }
        var positions: [String: LayoutPoint] = [:]
        let count = ids.count
        for (index, id) in ids.enumerated() {
            let angle = 2 * Double.pi * Double(index) / Double(count)
            positions[id] = LayoutPoint(x: radius * cos(angle), y: radius * sin(angle))
        }
        return positions
    }
}
