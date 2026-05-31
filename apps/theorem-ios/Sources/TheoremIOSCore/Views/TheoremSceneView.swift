import SwiftUI

struct TheoremSceneView: View {
    var package: ScenePackageV2
    var projection: ProjectionID
    @Binding var selectedNodeID: String?
    var theme: TheoremTheme

    private var result: ReprojectResult? {
        try? TheoremProjectionEngine.reproject(package, as: projection)
    }

    var body: some View {
        Group {
            if projection == .forceGraph {
                // Live force simulation (Grape) for the hero projection; the
                // other three draw the sliver's exact positions on the Canvas.
                ForceGraphView(package: package, theme: theme)
            } else {
                canvasBody
            }
        }
        .overlay(alignment: .topLeading) {
            sceneHeader
                .padding(18)
        }
    }

    private var canvasBody: some View {
        GeometryReader { proxy in
            let positions = positionMap(in: proxy.size)
            Canvas { context, size in
                drawRelations(context: &context, size: size, positions: positions)
                drawAtoms(context: &context, size: size, positions: positions)
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onEnded { value in
                        selectedNodeID = hitTest(value.location, positions: positions)
                    }
            )
        }
    }

    private var sceneHeader: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(projection.title)
                .font(TheoremFonts.mono(size: 11))
                .textCase(.uppercase)
                .foregroundStyle(theme.textSecondary)
            Text(selectedTitle)
                .font(TheoremFonts.display(size: 30, relativeTo: .title))
                .foregroundStyle(theme.textPrimary)
                .lineLimit(2)
        }
        .frame(maxWidth: 250, alignment: .leading)
    }

    private var selectedTitle: String {
        guard let selectedNodeID,
              let atom = package.atoms.first(where: { $0.id == selectedNodeID }) else {
            return "Substrate Scene"
        }
        return atom.label ?? atom.id
    }

    private func positionMap(in size: CGSize) -> [String: CGPoint] {
        guard let result else {
            return [:]
        }
        let raw = result.positions
        guard let minX = raw.map(\.x).min(),
              let maxX = raw.map(\.x).max(),
              let minY = raw.map(\.y).min(),
              let maxY = raw.map(\.y).max() else {
            return [:]
        }

        let pad = 76.0
        let width = max(maxX - minX, 1)
        let height = max(maxY - minY, 1)
        let scale = min(
            max((size.width - pad * 2) / width, 0.5),
            max((size.height - pad * 2) / height, 0.5)
        )

        return Dictionary(uniqueKeysWithValues: raw.map { position in
            let x = (position.x - minX - width / 2) * scale + size.width / 2
            let y = (position.y - minY - height / 2) * scale + size.height / 2
            return (position.id, CGPoint(x: x, y: y))
        })
    }

    private func drawRelations(context: inout GraphicsContext, size: CGSize, positions: [String: CGPoint]) {
        for relation in package.relations {
            guard let source = positions[relation.sourceId],
                  let target = positions[relation.targetId] else {
                continue
            }
            var path = Path()
            path.move(to: source)
            path.addLine(to: target)
            context.stroke(path, with: .color(theme.edge), lineWidth: 1.2)
        }
    }

    private func drawAtoms(context: inout GraphicsContext, size: CGSize, positions: [String: CGPoint]) {
        for atom in package.atoms {
            guard let point = positions[atom.id] else {
                continue
            }
            let radius = radius(for: atom)
            let rect = CGRect(x: point.x - radius, y: point.y - radius, width: radius * 2, height: radius * 2)
            let color = color(for: atom)
            context.fill(Path(ellipseIn: rect), with: .color(color))
            context.stroke(
                Path(ellipseIn: rect.insetBy(dx: -3, dy: -3)),
                with: .color(atom.id == selectedNodeID ? theme.ringMatch : theme.surface.opacity(0.72)),
                lineWidth: atom.id == selectedNodeID ? 3 : 1
            )
        }
    }

    private func hitTest(_ location: CGPoint, positions: [String: CGPoint]) -> String? {
        package.atoms
            .compactMap { atom -> (String, Double)? in
                guard let point = positions[atom.id] else {
                    return nil
                }
                let distance = hypot(point.x - location.x, point.y - location.y)
                return distance <= radius(for: atom) + 8 ? (atom.id, Double(distance)) : nil
            }
            .min { $0.1 < $1.1 }?
            .0
    }

    private func radius(for atom: SceneAtom) -> Double {
        let score = atom.metadata["matchScore"]?.doubleValue ?? atom.weight ?? 0.1
        return 8 + min(max(score, 0.05), 1.0) * 18
    }

    private func color(for atom: SceneAtom) -> Color {
        switch atom.kind {
        case "core":
            theme.nodeCore
        case "web":
            theme.nodeWeb
        case "tool":
            theme.nodeTool
        default:
            switch atom.metadata["ring"]?.intValue {
            case 0:
                theme.ringMatch
            case 1:
                theme.ringAdjacent
            default:
                theme.ringNearby
            }
        }
    }
}
