import SwiftUI

/// Patent-callout plate (addendum D7): a node rendered as a patent drafting
/// sheet. White sheet (field), black ink (edge) linework, a central figure (the
/// focus node), its real graph neighbors as numbered callouts with thin lead
/// lines, a numbered parts legend, a serif title cartouche, and a sheet footer.
/// This reimplements the d3-annotation grammar natively (subject + connector +
/// note + numbered badge); d3-annotation is the reference, not a dependency.
///
/// Drill-down is option A: tapping a numbered callout opens a new plate focused
/// on that neighbor, stacking onto a breadcrumb you can pop back through.
///
/// All content is real: the figure, callouts, and legend are derived from the
/// scene's atoms + relations. A node with no connections shows an honest empty
/// figure, never a fabricated one.
struct PatentPlateView: View {
    let package: ScenePackageV2
    var theme: TheoremTheme
    var onClose: () -> Void = {}

    @State private var focusID: String
    @State private var stack: [String] = []
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(package: ScenePackageV2, focusID: String, theme: TheoremTheme, onClose: @escaping () -> Void = {}) {
        self.package = package
        self.theme = theme
        self.onClose = onClose
        self._focusID = State(initialValue: focusID)
    }

    private var focus: SceneAtom? { package.atoms.first { $0.id == focusID } }

    private struct Callout {
        let atom: SceneAtom
        let relation: String
        let number: Int
    }

    private var callouts: [Callout] {
        var out: [Callout] = []
        var seen = Set<String>()
        for relation in package.relations {
            let other = relation.sourceId == focusID ? relation.targetId
                : (relation.targetId == focusID ? relation.sourceId : nil)
            guard let id = other, seen.insert(id).inserted,
                  let atom = package.atoms.first(where: { $0.id == id }) else { continue }
            out.append(Callout(atom: atom, relation: relation.kind, number: out.count + 1))
            if out.count >= 6 { break }
        }
        return out
    }

    var body: some View {
        GeometryReader { proxy in
            let figureHeight = proxy.size.height * 0.58
            ZStack(alignment: .topLeading) {
                Canvas { context, size in
                    drawSheet(&context, size: size, figureHeight: figureHeight)
                    drawFigure(&context, size: size, figureHeight: figureHeight)
                    drawLegend(&context, size: size, figureHeight: figureHeight)
                    drawTitleBlock(&context, size: size)
                }
                .contentShape(Rectangle())
                .onTapGesture { location in
                    let layout = figureLayout(width: proxy.size.width, figureHeight: figureHeight)
                    if let hit = layout.first(where: {
                        hypot($0.point.x - location.x, $0.point.y - location.y) <= 26
                    }) {
                        drill(to: hit.callout.atom.id)
                    }
                }
                chrome
            }
            .frame(width: proxy.size.width, height: proxy.size.height)
            .background(theme.field)
        }
        .ignoresSafeArea()
    }

    // MARK: - Chrome (close / back)

    private var chrome: some View {
        VStack {
            HStack {
                if !stack.isEmpty {
                    Button(action: back) {
                        Label("Back", systemImage: "chevron.left")
                            .font(TheoremFonts.label(size: 12))
                            .foregroundStyle(theme.ink)
                    }
                    .buttonStyle(.plain)
                }
                Spacer()
                Button(action: onClose) {
                    Image(systemName: "xmark")
                        .font(.system(size: 14, weight: .bold))
                        .foregroundStyle(theme.ink)
                        .padding(8)
                        .background(theme.chrome, in: Circle())
                        .overlay(Circle().stroke(theme.ink.opacity(0.4), lineWidth: 1))
                }
                .buttonStyle(.plain)
            }
            Spacer()
        }
        .padding(.horizontal, 22)
        .padding(.top, 58)
    }

    // MARK: - Layout

    private func figureLayout(width: CGFloat, figureHeight: CGFloat) -> [(callout: Callout, point: CGPoint)] {
        let center = CGPoint(x: width / 2, y: figureHeight * 0.52)
        let radius = min(width, figureHeight) * 0.32
        let items = callouts
        return items.map { callout in
            let t = Double(callout.number - 1) / Double(max(items.count, 1))
            let angle = t * 2 * .pi - .pi / 2
            let point = CGPoint(x: center.x + CoreGraphics.cos(angle) * radius,
                                y: center.y + CoreGraphics.sin(angle) * radius)
            return (callout, point)
        }
    }

    // MARK: - Draw

    private func drawSheet(_ context: inout GraphicsContext, size: CGSize, figureHeight: CGFloat) {
        // Double drafting frame.
        let outer = CGRect(x: 14, y: 44, width: size.width - 28, height: size.height - 88)
        context.stroke(Path(outer), with: .color(theme.ink), lineWidth: 1.4)
        context.stroke(Path(outer.insetBy(dx: 5, dy: 5)), with: .color(theme.ink.opacity(0.5)), lineWidth: 0.8)
        // Figure / legend divider.
        var divider = Path()
        divider.move(to: CGPoint(x: outer.minX + 5, y: 44 + figureHeight))
        divider.addLine(to: CGPoint(x: outer.maxX - 5, y: 44 + figureHeight))
        context.stroke(divider, with: .color(theme.ink.opacity(0.5)), lineWidth: 0.8)
    }

    private func drawFigure(_ context: inout GraphicsContext, size: CGSize, figureHeight: CGFloat) {
        let center = CGPoint(x: size.width / 2, y: figureHeight * 0.52)
        let layout = figureLayout(width: size.width, figureHeight: figureHeight)

        context.draw(
            Text("FIG. 1").font(.system(size: 15, weight: .regular, design: .serif)).foregroundStyle(theme.ink),
            at: CGPoint(x: 34, y: 70), anchor: .leading
        )

        // Lead lines (connectors) from the central subject to each callout.
        for item in layout {
            var lead = Path()
            lead.move(to: center)
            lead.addLine(to: item.point)
            context.stroke(lead, with: .color(theme.ink.opacity(0.55)), lineWidth: 0.9)
        }

        // Central subject (the focus node) as a labelled box.
        let label = focus?.label ?? focusID
        let subjectText = context.resolve(
            Text(label).font(TheoremFonts.label(size: 13)).foregroundStyle(theme.ink)
        )
        let ts = subjectText.measure(in: size)
        let box = CGRect(x: center.x - ts.width / 2 - 14, y: center.y - ts.height / 2 - 9,
                         width: ts.width + 28, height: ts.height + 18)
        context.fill(Path(roundedRect: box, cornerRadius: 4), with: .color(theme.field))
        context.stroke(Path(roundedRect: box, cornerRadius: 4), with: .color(theme.ink), lineWidth: 1.6)
        context.draw(subjectText, at: center, anchor: .center)

        // Callout markers: a small ring on the neighbour + a numbered badge.
        for item in layout {
            context.stroke(Path(ellipseIn: CGRect(x: item.point.x - 6, y: item.point.y - 6, width: 12, height: 12)),
                           with: .color(theme.ink), lineWidth: 1.2)
            let badge = CGRect(x: item.point.x + 7, y: item.point.y - 18, width: 18, height: 18)
            context.fill(Path(ellipseIn: badge), with: .color(theme.ink))
            context.draw(
                Text("\(item.callout.number)").font(TheoremFonts.mono(size: 10)).foregroundStyle(theme.field),
                at: CGPoint(x: badge.midX, y: badge.midY), anchor: .center
            )
        }

        if layout.isEmpty {
            // Honest empty figure: a node with no connections.
            context.draw(
                Text("No connections to diagram.").font(TheoremFonts.mono(size: 11)).foregroundStyle(theme.textMuted),
                at: CGPoint(x: size.width / 2, y: center.y + 54), anchor: .center
            )
        }
    }

    private func drawLegend(_ context: inout GraphicsContext, size: CGSize, figureHeight: CGFloat) {
        let top = 44 + figureHeight + 16
        context.draw(
            Text("REFERENCE NUMERALS").font(TheoremFonts.label(size: 10)).foregroundStyle(theme.textMuted),
            at: CGPoint(x: 34, y: top), anchor: .leading
        )
        for (i, callout) in callouts.enumerated() {
            let y = top + 22 + Double(i) * 20
            context.draw(
                Text("\(callout.number)").font(TheoremFonts.mono(size: 11)).foregroundStyle(theme.ink),
                at: CGPoint(x: 34, y: y), anchor: .leading
            )
            let name = callout.atom.label ?? callout.atom.id
            context.draw(
                Text("\(name)  ·  \(callout.relation)").font(TheoremFonts.body(size: 12)).foregroundStyle(theme.ink),
                at: CGPoint(x: 58, y: y), anchor: .leading
            )
        }
    }

    private func drawTitleBlock(_ context: inout GraphicsContext, size: CGSize) {
        let block = CGRect(x: size.width - 198, y: size.height - 116, width: 170, height: 58)
        context.fill(Path(block), with: .color(theme.field))
        context.stroke(Path(block), with: .color(theme.ink), lineWidth: 1.2)
        var mid = Path()
        mid.move(to: CGPoint(x: block.minX, y: block.minY + 26))
        mid.addLine(to: CGPoint(x: block.maxX, y: block.minY + 26))
        context.stroke(mid, with: .color(theme.ink.opacity(0.5)), lineWidth: 0.8)
        let title = focus?.label ?? focusID
        context.draw(
            Text(title).font(.system(size: 13, weight: .semibold, design: .serif)).foregroundStyle(theme.ink),
            at: CGPoint(x: block.minX + 9, y: block.minY + 13), anchor: .leading
        )
        context.draw(
            Text("THEOREM SUBSTRATE  ·  REF \(reference)").font(TheoremFonts.mono(size: 9)).foregroundStyle(theme.textMuted),
            at: CGPoint(x: block.minX + 9, y: block.minY + 40), anchor: .leading
        )
    }

    /// A derived substrate reference for the node (NOT a real patent number).
    private var reference: String {
        let hex = String(MT19937.seed(from: focusID) & 0xFFFF, radix: 16, uppercase: true)
        return "TH-" + String(repeating: "0", count: max(0, 4 - hex.count)) + hex
    }

    // MARK: - Drill

    private func drill(to id: String) {
        guard id != focusID else { return }
        stack.append(focusID)
        withAnimation(TheoremMotion.chrome(reduceMotion: reduceMotion)) { focusID = id }
    }

    private func back() {
        guard let previous = stack.popLast() else { return }
        withAnimation(TheoremMotion.chrome(reduceMotion: reduceMotion)) { focusID = previous }
    }
}
