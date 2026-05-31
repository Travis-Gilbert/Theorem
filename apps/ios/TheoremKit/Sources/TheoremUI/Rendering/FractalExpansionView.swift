import SwiftUI
import TheoremKit

/// The signature animation (spec algo 4): `push_ppr` visualized as it runs.
/// Nodes light up in the order the ACL-Push walk dequeues them, mass spreading
/// outward from the seeds. This is a faithful REPLAY of a real trace the server
/// streamed (the heavy walk runs server-side) — not a `setTimeout` fake. When no
/// trace has streamed yet, the base layout renders statically with an honest
/// note rather than fabricated motion.
struct FractalExpansionView: View {
    let scene: ScenePackageV2
    let positions: [String: LayoutPoint]
    let theme: Theme
    let trace: PushTrace?
    @Binding var selectedID: String?

    /// Walk steps revealed per second of playback (a viewing pace over recorded
    /// events, like scrubbing a video — the events themselves are real).
    private let eventsPerSecond: Double = 14
    /// Fade window (in event-units) over which a freshly-reached node brightens.
    private let fadeWindow: Double = 2.5

    @State private var startDate = Date()

    var body: some View {
        Group {
            if let trace, !trace.events.isEmpty {
                let firstOrder = firstOrderByNode(trace)
                let lastOrder = Double(trace.events.map(\.order).max() ?? 0)
                TimelineView(.animation) { timeline in
                    let elapsed = timeline.date.timeIntervalSince(startDate)
                    let front = min(elapsed * eventsPerSecond, lastOrder + fadeWindow)
                    GraphCanvas(
                        scene: scene,
                        positions: positions,
                        theme: theme,
                        selectedID: $selectedID,
                        nodeReveal: { id in reveal(id, front: front, firstOrder: firstOrder) }
                    )
                }
                .onChange(of: trace) { _, _ in startDate = Date() }
            } else {
                ZStack {
                    GraphCanvas(scene: scene, positions: positions, theme: theme, selectedID: $selectedID)
                    VStack {
                        Spacer()
                        Text("Expansion streams when you run the search.")
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(theme.swiftUIColor(.textSecondary))
                            .padding(8)
                    }
                }
            }
        }
    }

    /// Brightness 0...1 for a node given the wavefront position. Reached nodes
    /// fade in as the front passes their first push; nodes the walk never reached
    /// stay dim-but-visible (honest: they were not part of the expansion).
    private func reveal(_ id: String, front: Double, firstOrder: [String: Int]) -> Double {
        guard let order = firstOrder[id] else { return 0.18 }
        let t = (front - Double(order)) / fadeWindow
        return min(max(t, 0), 1)
    }

    private func firstOrderByNode(_ trace: PushTrace) -> [String: Int] {
        var out: [String: Int] = [:]
        for event in trace.events {
            if let existing = out[event.nodeID] {
                out[event.nodeID] = min(existing, event.order)
            } else {
                out[event.nodeID] = event.order
            }
        }
        return out
    }
}
