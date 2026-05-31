import SwiftUI
import TheoremKit

/// Vanilla 2D-`Canvas` renderer for a laid-out scene. Shared by the radial, tree,
/// and fractal projections: each hands it a different set of positions (from the
/// sliver) and the canvas draws the substrate vocabulary — relations as lines,
/// atoms as role-colored circles with labels. Tap hit-tests real positions and
/// selects the nearest atom (real selection, surfaced to the view model; no
/// theater).
///
/// `nodeReveal` modulates per-atom brightness/size for the fractal wavefront
/// (1 = lit, 0 = not yet reached); when nil every atom is fully drawn.
struct GraphCanvas: View {
    let scene: ScenePackageV2
    let positions: [String: LayoutPoint]
    let theme: Theme
    @Binding var selectedID: String?
    var nodeReveal: ((String) -> Double)?

    private static let minRadius: Double = 6
    private static let maxRadius: Double = 18
    private static let labelThreshold = 40

    var body: some View {
        GeometryReader { geo in
            let screen = SceneFitting.fit(positions, in: CGRect(origin: .zero, size: geo.size))
            let maxMag = max(scene.atoms.map(\.magnitude).max() ?? 1, 1)
            let showAllLabels = scene.atoms.count <= Self.labelThreshold

            Canvas { context, _ in
                drawEdges(context, screen: screen)
                drawNodes(context, screen: screen, maxMag: maxMag, showAllLabels: showAllLabels)
            }
            .contentShape(Rectangle())
            .onTapGesture { location in
                selectedID = nearestNode(to: location, screen: screen)
            }
        }
        .background(theme.swiftUIColor(.background))
    }

    // MARK: Drawing

    private func drawEdges(_ context: GraphicsContext, screen: [String: CGPoint]) {
        let edgeColor = theme.swiftUIColor(.edge)
        for relation in scene.relations {
            guard let from = screen[relation.sourceId], let to = screen[relation.targetId] else { continue }
            let lit = min(reveal(relation.sourceId), reveal(relation.targetId))
            guard lit > 0 else { continue }
            var path = Path()
            path.move(to: from)
            path.addLine(to: to)
            let highlighted = selectedID == relation.sourceId || selectedID == relation.targetId
            context.stroke(
                path,
                with: .color(highlighted ? theme.swiftUIColor(.ringMatch) : edgeColor.opacity(0.5 * lit)),
                lineWidth: highlighted ? 2 : 1.25
            )
        }
    }

    private func drawNodes(
        _ context: GraphicsContext,
        screen: [String: CGPoint],
        maxMag: Double,
        showAllLabels: Bool
    ) {
        for atom in scene.atoms {
            guard let center = screen[atom.id] else { continue }
            let lit = reveal(atom.id)
            guard lit > 0 else { continue }

            let norm = atom.magnitude / maxMag
            let baseR = Self.minRadius + norm * (Self.maxRadius - Self.minRadius)
            let r = baseR * (0.5 + 0.5 * lit) // fractal: grows as the wavefront lands
            let selected = atom.id == selectedID
            let color = theme.swiftUIColorForAtom(kind: atom.kind, ring: atom.ring).opacity(0.35 + 0.65 * lit)

            let rect = CGRect(x: center.x - r, y: center.y - r, width: r * 2, height: r * 2)
            context.fill(Path(ellipseIn: rect), with: .color(color))
            context.stroke(
                Path(ellipseIn: rect),
                with: .color(selected ? theme.swiftUIColor(.ringMatch) : theme.swiftUIColor(.textPrimary).opacity(0.7)),
                lineWidth: selected ? 2.5 : 1
            )

            if (showAllLabels || selected), let label = atom.label ?? Optional(atom.id) {
                let text = Text(truncated(label, max: selected ? 40 : 18))
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(theme.swiftUIColor(selected ? .textPrimary : .textSecondary))
                context.draw(text, at: CGPoint(x: center.x, y: center.y + r + 9), anchor: .center)
            }
        }
    }

    // MARK: Helpers

    private func reveal(_ id: String) -> Double {
        nodeReveal?(id) ?? 1
    }

    private func nearestNode(to location: CGPoint, screen: [String: CGPoint]) -> String? {
        var best: String?
        var bestDist = Double.infinity
        for atom in scene.atoms {
            guard let p = screen[atom.id] else { continue }
            let dx = Double(location.x - p.x)
            let dy = Double(location.y - p.y)
            let dist = dx * dx + dy * dy
            if dist < bestDist, dist <= 26 * 26 {
                best = atom.id
                bestDist = dist
            }
        }
        return best
    }

    private func truncated(_ value: String, max: Int) -> String {
        value.count > max ? String(value.prefix(max - 1)) + "…" : value
    }
}
