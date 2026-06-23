import SwiftUI
import Grape
import TheoremKit

/// The hero projection (spec algo 1): a node-link layout via Grape's force
/// simulation. Node radius scales with `match_score` (PPR mass); edges come from
/// the scene relations. Native drag / pan / zoom via Grape's gesture overlay.
///
/// Grape runs the simulation itself, so this view does not consume the sliver's
/// positions — it is the one projection that lays out on the GPU/CPU sim rather
/// than via `reproject` (which only seeds it).
struct ForceGraphView: View {
    let scene: ScenePackageV2
    let theme: Theme

    @State private var graphState = ForceDirectedGraphState(initialIsRunning: true)

    var body: some View {
        let maxMag = max(scene.atoms.map(\.magnitude).max() ?? 1, 1)

        ForceDirectedGraph(states: graphState) {
            Series(scene.atoms) { atom in
                NodeMark(id: atom.id)
                    .symbol(.circle)
                    .symbolSize(radius: radius(for: atom, maxMag: maxMag))
                    .foregroundStyle(theme.swiftUIColorForAtom(kind: atom.kind, ring: atom.ring))
                    .stroke(theme.swiftUIColor(.background), StrokeStyle(lineWidth: 1.0))
            }
            Series(scene.relations) { relation in
                LinkMark(from: relation.sourceId, to: relation.targetId)
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
        .background(theme.swiftUIColor(.background))
    }

    private func radius(for atom: SceneAtom, maxMag: Double) -> Double {
        let norm = atom.magnitude / maxMag
        return 5 + norm * 13
    }
}
