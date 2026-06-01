import SwiftUI
import Grape

/// The search-results graph, rendered in the spirit of d3's force-directed tree
/// (Travis's @d3/force-directed-tree reference). Internal nodes (degree >= 2)
/// are hollow rings, leaves (degree 1) are filled dots — the d3
/// `d.children ? hollow : filled` rule as a degree test — with thin grey edges
/// and native drag. Two deliberate departures from d3: search caps at ~10 nodes
/// (more overwhelms a person), and the nodes are larger and more tappable than
/// d3's tiny r=3.5-over-150. The force config is loosened to suit ~10 big nodes.
struct ForceGraphView: View {
    var package: ScenePackageV2
    var theme: TheoremTheme

    @State private var graphState = ForceDirectedGraphState(initialIsRunning: true)

    /// Node radius. Deliberately larger than d3's r=3.5: search returns at most
    /// ~10 nodes, so they can be big and tappable rather than a dense field.
    private let nodeRadius: CGFloat = 9

    var body: some View {
        let degree = degreeMap()
        // The d3 reference is unlabeled; keep labels only for sparse graphs where
        // they help, and let dense trees speak as pure structure.
        let showLabels = package.atoms.count <= 14

        ForceDirectedGraph(states: graphState) {
            Series(package.atoms) { atom in
                let leaf = (degree[atom.id] ?? 0) <= 1
                // d3 force-directed-tree node: internal = hollow ring (field fill,
                // ink stroke 1.5); leaf = filled ink dot with a thin field halo
                // (the halo separates leaves where clusters overlap).
                NodeMark(id: atom.id)
                    .symbol(.circle)
                    .symbolSize(radius: nodeRadius)
                    .foregroundStyle(leaf ? theme.ink : theme.field)
                    .stroke(
                        leaf ? theme.field : theme.ink,
                        StrokeStyle(lineWidth: leaf ? 1.5 : 2)
                    )
                    .annotation(
                        showLabels ? labelText(atom) : nil,
                        alignment: .bottom,
                        offset: SIMD2(0, Double(nodeRadius) + 5)
                    )
            }
            Series(package.relations) { relation in
                // Thin grey edges (d3 #999 at 0.6 opacity reads as a light-mid grey).
                LinkMark(from: relation.sourceId, to: relation.targetId)
                    .stroke(theme.ink.opacity(0.30), StrokeStyle(lineWidth: 1))
            }
        } force: {
            // ~10 large nodes want a comfortable radial, not d3's tight 150-node
            // clustering. Stronger charge + longer spokes spread the hub-and-leaf
            // result across the canvas; the hollow/filled + drag stay d3-faithful.
            .manyBody(strength: -150)
            .center()
            .link(originalLength: 44.0, stiffness: .weightedByDegree { _, _ in 1.0 })
        }
        .graphOverlay { proxy in
            Rectangle()
                .fill(.clear)
                .contentShape(Rectangle())
                .withGraphDragGesture(proxy, of: String.self)
                .withGraphMagnifyGesture(proxy)
        }
    }

    /// Undirected degree per node. Leaf = degree <= 1 (filled); internal = degree
    /// >= 2 (hollow), mirroring d3's parent/child distinction on a tree.
    private func degreeMap() -> [String: Int] {
        var degree: [String: Int] = [:]
        for relation in package.relations {
            degree[relation.sourceId, default: 0] += 1
            degree[relation.targetId, default: 0] += 1
        }
        return degree
    }

    /// Node label (atom.label, else id), in the instrument label face. Returns a
    /// `Text` so Grape's `.annotation(_ text: Text?, ...)` overload renders it —
    /// the `String?` overload silently discards its argument.
    private func labelText(_ atom: SceneAtom) -> Text? {
        let raw = atom.label ?? atom.id
        guard !raw.isEmpty else { return nil }
        let shown = raw.count > 18 ? String(raw.prefix(17)) + "\u{2026}" : raw
        return Text(shown)
            .font(TheoremFonts.label(size: 9))
            .foregroundStyle(theme.ink)
    }
}
