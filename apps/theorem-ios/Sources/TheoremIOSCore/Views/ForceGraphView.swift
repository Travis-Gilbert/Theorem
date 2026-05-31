import SwiftUI
import Grape

/// The hero projection (spec algo 1): a live force simulation via Grape, rather
/// than the static Canvas seed the other projections draw. Node radius scales
/// with `matchScore` (PPR mass); native drag / pan / zoom via Grape's gesture
/// overlay. Colors and radii mirror `TheoremSceneView` so switching to/from this
/// projection is visually continuous — only the layout becomes live.
///
/// Ported in during the Swift-lane convergence (Travis: "include grape"). The
/// other three projections stay on the Canvas renderer, which honours the
/// sliver's exact positions.
struct ForceGraphView: View {
    var package: ScenePackageV2
    var theme: TheoremTheme

    @State private var graphState = ForceDirectedGraphState(initialIsRunning: true)

    var body: some View {
        let maxMag = max(package.atoms.map(magnitude).max() ?? 1, 1)

        ForceDirectedGraph(states: graphState) {
            Series(package.atoms) { atom in
                // Monochrome instrument node: field fill, ink outline (mirrors
                // TheoremSceneView's Canvas renderer so switching is continuous).
                NodeMark(id: atom.id)
                    .symbol(.circle)
                    .symbolSize(radius: 6 + (magnitude(atom) / maxMag) * 16)
                    .foregroundStyle(theme.field)
                    .stroke(theme.ink, StrokeStyle(lineWidth: 1.2))
            }
            Series(package.relations) { relation in
                LinkMark(from: relation.sourceId, to: relation.targetId)
                    .stroke(theme.rule, StrokeStyle(lineWidth: 1.2))
            }
        } force: {
            // Charge / link tuning carried from the proven SERP force graph.
            .manyBody(strength: -230)
            .center()
            .link(originalLength: 62.0, stiffness: .weightedByDegree { _, _ in 1.0 })
        }
        .graphOverlay { proxy in
            Rectangle()
                .fill(.clear)
                .contentShape(Rectangle())
                .withGraphDragGesture(proxy, of: String.self)
                .withGraphMagnifyGesture(proxy)
        }
        .background(theme.background)
    }

    /// PPR mass for radius (matches TheoremSceneView's `radius(for:)`).
    private func magnitude(_ atom: SceneAtom) -> Double {
        atom.metadata["matchScore"]?.doubleValue ?? atom.weight ?? 0.1
    }
}
