import Foundation

/// A spanning-tree backbone extracted from a general scene graph so the force
/// simulation relaxes into a legible *tree* instead of a hairball.
///
/// Real scene graphs are not trees. A live substrate search links the query to
/// every ring-0 result and the substrate adds result<->result cross-links, so a
/// plain force layout collapses into an unreadable knot (and the static
/// `tree_layout` projection refuses the graph outright, since it demands exactly
/// n-1 links). We extract a maximum-weight spanning tree (Prim's, grown from the
/// highest-centrality atom) and drive the simulation with those backbone edges
/// only. The result is d3's force-directed-tree shape: a clear center, branches
/// that read, leaves that do not pile up. Cross-links are dropped from the
/// *force* (the tree is the skeleton); they can return later as faint,
/// non-structural context strokes.
///
/// It degrades cleanly:
///  - graph already a tree (the sample scene)  -> backbone equals its edges;
///  - graph a pure star (query -> many results) -> backbone is that star (depth 1);
///  - disconnected atoms (frontier with no shown edge) -> attached to the root,
///    so nothing floats off the canvas.
public struct ForceTreeBackbone: Equatable, Sendable {
    /// A directed parent -> child edge of the spanning tree.
    public struct Edge: Equatable, Sendable, Identifiable {
        public var parent: String
        public var child: String
        public init(parent: String, child: String) {
            self.parent = parent
            self.child = child
        }
        public var id: String { "\(parent)\u{2192}\(child)" }
    }

    /// The center the tree grows from (highest match score / weight), or nil for
    /// an empty scene.
    public var rootID: String?
    /// Parent -> child backbone edges, in the order Prim's added them.
    public var edges: [Edge]
    /// Depth of each atom from the root (root = 0).
    public var depth: [String: Int]
    /// Number of backbone children each atom has (0 == leaf). Used as the
    /// provenance-absent fallback for the hollow/filled node encoding.
    public var childCount: [String: Int]

    public init(
        rootID: String?,
        edges: [Edge],
        depth: [String: Int],
        childCount: [String: Int]
    ) {
        self.rootID = rootID
        self.edges = edges
        self.depth = depth
        self.childCount = childCount
    }

    public static let empty = ForceTreeBackbone(rootID: nil, edges: [], depth: [:], childCount: [:])
}

public enum ForceTree {
    /// Build the backbone for a scene. Display graphs are tiny (<= ~14 atoms after
    /// the search cap), so a plain O(V*E) Prim's is more than fast enough and keeps
    /// the logic auditable.
    public static func backbone(atoms: [SceneAtom], relations: [SceneRelation]) -> ForceTreeBackbone {
        guard !atoms.isEmpty else { return .empty }

        let scoreByID = Dictionary(uniqueKeysWithValues: atoms.map { ($0.id, score($0)) })

        // Root = the highest-scoring atom (corpus hub / query seed). Deterministic
        // tie-break: lower id wins, so the same scene always grows the same tree.
        guard let root = atoms.max(by: { left, right in
            let l = scoreByID[left.id] ?? 0
            let r = scoreByID[right.id] ?? 0
            return l == r ? left.id > right.id : l < r
        }) else { return .empty }

        let ids = Set(atoms.map(\.id))

        // Undirected weighted adjacency. Missing backend weights default to 1 so a
        // plain link still counts; self-loops and dangling endpoints are dropped.
        var adjacency: [String: [(neighbor: String, weight: Double)]] = [:]
        for relation in relations
        where relation.sourceId != relation.targetId
            && ids.contains(relation.sourceId)
            && ids.contains(relation.targetId) {
            let w = relation.weight ?? 1.0
            adjacency[relation.sourceId, default: []].append((relation.targetId, w))
            adjacency[relation.targetId, default: []].append((relation.sourceId, w))
        }

        var inTree: Set<String> = [root.id]
        var depth: [String: Int] = [root.id: 0]
        var childCount: [String: Int] = [:]
        var edges: [ForceTreeBackbone.Edge] = []

        // Prim's: repeatedly attach the outside atom joined by the strongest edge.
        while true {
            var best: (parent: String, child: String, weight: Double)?
            for parent in inTree {
                for (child, weight) in adjacency[parent] ?? [] where !inTree.contains(child) {
                    let candidate = (parent: parent, child: child, weight: weight)
                    if best == nil || isStronger(candidate, than: best!, scoreByID: scoreByID) {
                        best = candidate
                    }
                }
            }
            guard let pick = best else { break }
            inTree.insert(pick.child)
            depth[pick.child] = (depth[pick.parent] ?? 0) + 1
            childCount[pick.parent, default: 0] += 1
            edges.append(ForceTreeBackbone.Edge(parent: pick.parent, child: pick.child))
        }

        // Atoms unreachable through any relation (a frontier node with no shown
        // edge) attach straight to the root, sorted for determinism, so the tree
        // stays connected and nothing drifts off-canvas.
        for atom in atoms.sorted(by: { $0.id < $1.id }) where !inTree.contains(atom.id) {
            inTree.insert(atom.id)
            depth[atom.id] = 1
            childCount[root.id, default: 0] += 1
            edges.append(ForceTreeBackbone.Edge(parent: root.id, child: atom.id))
        }

        return ForceTreeBackbone(rootID: root.id, edges: edges, depth: depth, childCount: childCount)
    }

    /// Centrality used to pick the root and break weight ties, matching the
    /// projection engine's ranking: match score first, then atom weight.
    private static func score(_ atom: SceneAtom) -> Double {
        atom.metadata["matchScore"]?.doubleValue ?? atom.weight ?? 0
    }

    /// Strict total order over candidate edges so Prim's result is independent of
    /// `Set` iteration order: heavier edge wins; then the more central child; then
    /// the lower child id; finally the lower parent id.
    private static func isStronger(
        _ lhs: (parent: String, child: String, weight: Double),
        than rhs: (parent: String, child: String, weight: Double),
        scoreByID: [String: Double]
    ) -> Bool {
        if lhs.weight != rhs.weight { return lhs.weight > rhs.weight }
        let lScore = scoreByID[lhs.child] ?? 0
        let rScore = scoreByID[rhs.child] ?? 0
        if lScore != rScore { return lScore > rScore }
        if lhs.child != rhs.child { return lhs.child < rhs.child }
        return lhs.parent < rhs.parent
    }
}
