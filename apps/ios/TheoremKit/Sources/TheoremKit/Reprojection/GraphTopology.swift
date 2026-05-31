import Foundation

/// Pure graph helpers over a scene's atoms + relations. Deterministic and
/// allocation-light: the sliver runs this on every reprojection over the bounded
/// current scene (tens to low hundreds of atoms), never the corpus.
///
/// Edges are taken from `relations` whose BOTH endpoints are atoms in the scene
/// (the substrate guarantees this for SubstrateSearch-derived scenes; we filter
/// defensively). Adjacency is undirected for shape tests; `children` preserves
/// the source -> target direction for hierarchy roots.
struct GraphTopology {
    /// Atom ids in a deterministic order (sorted) so every layout is reproducible.
    let atomIDs: [String]
    private let atomSet: Set<String>
    /// Undirected adjacency: id -> sorted neighbour ids (deduped).
    let adjacency: [String: [String]]
    /// Unordered endpoint pairs after dedup (no parallel edges, no self-loops).
    let undirectedEdges: [(String, String)]
    /// Whether any relation was a self-loop (source == target) — a hard reject
    /// for tree shape.
    let hasSelfLoop: Bool

    init(atoms: [SceneAtom], relations: [SceneRelation]) {
        let ids = atoms.map(\.id).sorted()
        self.atomIDs = ids
        let set = Set(ids)
        self.atomSet = set

        var adj: [String: Set<String>] = [:]
        var pairSet = Set<String>()
        var pairs: [(String, String)] = []
        var selfLoop = false

        for rel in relations {
            let s = rel.sourceId
            let t = rel.targetId
            guard set.contains(s), set.contains(t) else { continue }
            if s == t { selfLoop = true; continue }
            adj[s, default: []].insert(t)
            adj[t, default: []].insert(s)
            // Dedup unordered pair.
            let key = s < t ? "\(s)\u{1}\(t)" : "\(t)\u{1}\(s)"
            if pairSet.insert(key).inserted {
                pairs.append(s < t ? (s, t) : (t, s))
            }
        }

        self.adjacency = adj.mapValues { $0.sorted() }
        self.undirectedEdges = pairs
        self.hasSelfLoop = selfLoop
    }

    var atomCount: Int { atomIDs.count }
    var edgeCount: Int { undirectedEdges.count }

    func degree(of id: String) -> Int {
        adjacency[id]?.count ?? 0
    }

    /// True iff the (deduped, self-loop-free) link graph is a forest — acyclic.
    /// A forest is exactly the condition under which `tree_layout` is honest:
    /// every component is a tree, so a BFS from any root produces a real
    /// hierarchy with no cross-edges. Uses union-find: the first edge that joins
    /// two already-connected nodes is a cycle.
    func isForest() -> Bool {
        if hasSelfLoop { return false }
        var parent: [String: String] = Dictionary(uniqueKeysWithValues: atomIDs.map { ($0, $0) })
        var rank: [String: Int] = [:]

        func find(_ x: String) -> String {
            var root = x
            while parent[root] != root { root = parent[root]! }
            // Path compression.
            var cur = x
            while parent[cur] != root {
                let next = parent[cur]!
                parent[cur] = root
                cur = next
            }
            return root
        }

        for (a, b) in undirectedEdges {
            let ra = find(a)
            let rb = find(b)
            if ra == rb { return false } // joining an already-connected pair => cycle
            let raRank = rank[ra, default: 0]
            let rbRank = rank[rb, default: 0]
            if raRank < rbRank {
                parent[ra] = rb
            } else if raRank > rbRank {
                parent[rb] = ra
            } else {
                parent[rb] = ra
                rank[ra] = raRank + 1
            }
        }
        return true
    }

    /// BFS parent map rooted at `root` over the undirected graph. Nodes not
    /// reachable from `root` (other forest components) are absent from the map.
    /// Neighbours are visited in sorted order for determinism.
    func bfsParents(root: String) -> [String: String] {
        var parent: [String: String] = [:]
        var visited: Set<String> = [root]
        var queue = [root]
        var head = 0
        while head < queue.count {
            let node = queue[head]
            head += 1
            for neighbour in adjacency[node] ?? [] where !visited.contains(neighbour) {
                visited.insert(neighbour)
                parent[neighbour] = node
                queue.append(neighbour)
            }
        }
        return parent
    }
}
