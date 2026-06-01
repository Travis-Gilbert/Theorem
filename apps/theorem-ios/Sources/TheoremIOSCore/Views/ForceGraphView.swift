import SwiftUI
import Grape

/// The search-results graph as a *knowledge-frontier map*, rendered in the
/// spirit of d3's force-directed tree (Travis's @d3/force-directed-tree
/// reference) but encoding two real backend signals instead of pure tree shape:
///
/// - **Fill = provenance.** A hollow ring (field fill, ink stroke) is the user's
///   relevant *corpus*: a page the substrate has actually fetched and knows. A
///   filled ink dot is the *frontier*: a discovered-but-unfetched link target,
///   something new past the edge of what's known. (When the backend predates the
///   provenance field, we fall back to d3's `d.children ? hollow : filled` rule
///   as a degree test — internal hollow, leaf filled.)
/// - **Size = centrality.** Radius scales with match score, so the central,
///   strongly-matched corpus reads large and the thin frontier branches read
///   small. Because fetched/matched pages score high and unfetched frontier
///   targets score ~0, size and fill reinforce: white tends large, black small.
///
/// Two deliberate departures from d3: search caps at ~10 nodes (more overwhelms a
/// person), and the nodes are larger and more tappable than d3's r=3.5-over-150.
struct ForceGraphView: View {
    var package: ScenePackageV2
    var theme: TheoremTheme

    @State private var graphState = ForceDirectedGraphState(initialIsRunning: true)

    /// Radius envelope. The floor stays well above d3's r=3.5 so even a frontier
    /// leaf is tappable; the ceiling lets a central corpus hub read as the anchor.
    private let minRadius: CGFloat = 7
    private let maxRadius: CGFloat = 15
    /// The typed query is the seed of the map: a fixed mid-anchor, not score-sized.
    private let queryRadius: CGFloat = 12

    var body: some View {
        let degree = degreeMap()
        let maxScore = maxMatchScore()
        // The d3 reference is unlabeled; keep labels only for sparse graphs where
        // they help, and let dense trees speak as pure structure.
        let showLabels = package.atoms.count <= 14

        ForceDirectedGraph(states: graphState) {
            Series(package.atoms) { atom in
                let r = radius(for: atom, maxScore: maxScore)
                let filled = isFilled(atom, degree: degree)
                // Frontier map: corpus = hollow ring (field fill, ink stroke),
                // frontier = filled ink dot with a thin field halo (the halo
                // separates dots where frontier clusters overlap).
                NodeMark(id: atom.id)
                    .symbol(.circle)
                    .symbolSize(radius: r)
                    .foregroundStyle(filled ? theme.ink : theme.field)
                    .stroke(
                        filled ? theme.field : theme.ink,
                        StrokeStyle(lineWidth: filled ? 1.5 : 2)
                    )
                    .annotation(
                        showLabels ? labelText(atom) : nil,
                        alignment: .bottom,
                        offset: SIMD2(0, Double(r) + 5)
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

    /// Whether a node renders filled (black/new) vs hollow (white/known).
    /// Provenance is the primary signal; the query seed reads as known; the
    /// degree rule is the fallback when the backend supplies no provenance.
    private func isFilled(_ atom: SceneAtom, degree: [String: Int]) -> Bool {
        switch provenance(of: atom) {
        case "frontier": return true   // new, past the frontier => filled/black
        case "corpus": return false    // the user's known corpus => hollow/white
        default:
            // The typed query is the most-known node: render it hollow.
            if atom.kind == "query" { return false }
            // d3 fallback: leaf (degree <= 1) filled, internal hollow.
            return (degree[atom.id] ?? 0) <= 1
        }
    }

    /// Node radius. The query is a fixed anchor; every other node scales with
    /// match score so central corpus reads large and frontier branches read small.
    private func radius(for atom: SceneAtom, maxScore: Double) -> CGFloat {
        if atom.kind == "query" { return queryRadius }
        let score = atom.metadata["matchScore"]?.doubleValue ?? atom.weight ?? 0
        let norm = maxScore > 0 ? min(max(score / maxScore, 0), 1) : 0.5
        return minRadius + CGFloat(norm) * (maxRadius - minRadius)
    }

    /// Largest match score among result nodes (excluding the query seed), the
    /// denominator that normalizes radius. Zero when nothing scored.
    private func maxMatchScore() -> Double {
        package.atoms
            .filter { $0.kind != "query" }
            .compactMap { $0.metadata["matchScore"]?.doubleValue ?? $0.weight }
            .max() ?? 0
    }

    private func provenance(of atom: SceneAtom) -> String? {
        atom.metadata["provenance"]?.stringValue
    }

    /// Undirected degree per node. Used only as the provenance fallback: leaf =
    /// degree <= 1 (filled), internal = degree >= 2 (hollow), mirroring d3's
    /// parent/child distinction on a tree.
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
