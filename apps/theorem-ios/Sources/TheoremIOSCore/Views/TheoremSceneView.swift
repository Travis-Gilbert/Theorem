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
        ZStack {
            // Pure-white field. The hex-blueprint texture lives in the Dynamic
            // Island chrome now (pill + expanded), not behind the graph.
            theme.field
            Group {
                if projection == .forceGraph {
                    // Live force simulation (Grape) for the hero projection; the
                    // other three draw the sliver's exact positions on the Canvas.
                    ForceGraphView(package: package, theme: theme)
                } else {
                    canvasBody
                }
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
        VStack(alignment: .leading, spacing: 5) {
            // The query / center-node name is the headline, kept subtle. The
            // "FORCE · N NODES" caption was cut per Travis.
            if let headline = sceneHeadline {
                Text(headline)
                    .font(TheoremFonts.display(size: 22, relativeTo: .title2))
                    .foregroundStyle(theme.ink)
                    .lineLimit(2)
            }
        }
        .frame(maxWidth: 280, alignment: .leading)
    }

    /// The prominent type: the selected node, else the search query, else the
    /// center node (highest match_score) — never a generic screen title.
    private var sceneHeadline: String? {
        if let selectedNodeID,
           let atom = package.atoms.first(where: { $0.id == selectedNodeID }) {
            return atom.label ?? atom.id
        }
        if let query = package.provenance["query"]?.stringValue,
           !query.trimmingCharacters(in: .whitespaces).isEmpty {
            return query
        }
        if let center = TheoremProjectionEngine.centerNodeID(in: package, mode: .pprMass),
           let atom = package.atoms.first(where: { $0.id == center }) {
            return atom.label ?? atom.id
        }
        return nil
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

            // Edge-kind label (relation.kind) in the data face. Gated by the
            // >12-node rule: dense graphs only label the selected node's edges.
            guard showEdgeLabel(for: relation), !relation.kind.isEmpty else { continue }
            let mid = CGPoint(x: (source.x + target.x) / 2, y: (source.y + target.y) / 2)
            let resolved = context.resolve(
                Text(relation.kind)
                    .font(TheoremFonts.mono(size: 8))
                    .foregroundStyle(theme.textMuted)
            )
            let sz = resolved.measure(in: size)
            // Field chip behind the label so the rule line doesn't strike through.
            let chip = CGRect(
                x: mid.x - sz.width / 2 - 3, y: mid.y - sz.height / 2 - 1,
                width: sz.width + 6, height: sz.height + 2
            )
            context.fill(Path(roundedRect: chip, cornerRadius: 2), with: .color(theme.field))
            context.draw(resolved, at: mid, anchor: .center)
        }
    }

    private func drawAtoms(context: inout GraphicsContext, size: CGSize, positions: [String: CGPoint]) {
        for atom in package.atoms {
            guard let point = positions[atom.id] else {
                continue
            }
            let radius = radius(for: atom)
            let rect = CGRect(x: point.x - radius, y: point.y - radius, width: radius * 2, height: radius * 2)
            // Monochrome instrument node: field fill, ink outline. Selection is the
            // only hue — the node flips to oxblood at a heavier stroke.
            let selected = atom.id == selectedNodeID
            context.fill(Path(ellipseIn: rect), with: .color(theme.field))
            context.stroke(
                Path(ellipseIn: rect),
                with: .color(selected ? theme.signal : theme.ink),
                lineWidth: selected ? 2 : 1.2
            )

            // Node label (atom.label) below the node, in the instrument label
            // face. Shown for sparse graphs, or the selected node when dense.
            if (package.atoms.count <= 14 || selected), let label = nodeLabel(atom) {
                context.draw(
                    Text(label)
                        .font(TheoremFonts.label(size: 9))
                        .foregroundStyle(selected ? theme.signal : theme.ink),
                    at: CGPoint(x: point.x, y: point.y + radius + 7),
                    anchor: .top
                )
            }
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

    /// >12-node gate (addendum): sparse graphs label every edge; dense graphs
    /// label only the selected node's edges (others would crowd the field).
    private func showEdgeLabel(for relation: SceneRelation) -> Bool {
        package.atoms.count <= 12
            || relation.sourceId == selectedNodeID
            || relation.targetId == selectedNodeID
    }

    /// Node label text (atom.label, else id), truncated for the canvas.
    private func nodeLabel(_ atom: SceneAtom) -> String? {
        let raw = atom.label ?? atom.id
        guard !raw.isEmpty else { return nil }
        return raw.count > 18 ? String(raw.prefix(17)) + "\u{2026}" : raw
    }
}
