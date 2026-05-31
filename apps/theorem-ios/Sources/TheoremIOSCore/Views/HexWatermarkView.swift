import SwiftUI

/// MT19937-seeded hex-blueprint texture. The instrument's own surface grain,
/// living ONLY inside the Dynamic Island chrome (the pill + the expanded island),
/// never the page field. A d3-hexbin pointy-top lattice: most cells blank, a
/// sparse algorithmic fraction inked in fairly-dark blueprint blue, like an old
/// blueprint. Deterministic (MT-seeded) and static. Blue is an accent here, not
/// a surface, so the frequency stays low.
struct HexWatermarkView: View {
    var seed: UInt32
    var theme: TheoremTheme
    /// Fraction of cells inked. Sparse on purpose: blue is a bonus, not a surface,
    /// so most cells stay blank.
    var frequency: Double = 0.05
    /// Hex radius (center to vertex), in points.
    var radius: Double = 12
    /// Ink opacity. Fairly dark per cell (an old-blueprint mark), but sparse
    /// overall so blue stays an accent, not a surface.
    var inkOpacity: Double = 0.60

    var body: some View {
        Canvas { context, size in
            draw(into: &context, size: size)
        }
        .drawingGroup()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    private func draw(into context: inout GraphicsContext, size: CGSize) {
        var rng = MT19937(seed: seed)
        let r = radius
        // d3-hexbin pointy-top lattice: dx = r*sqrt(3) across, dy = r*1.5 down,
        // odd rows offset by dx/2.
        let dx = r * sqrt(3.0)
        let dy = r * 1.5
        let shading = GraphicsContext.Shading.color(theme.blueprintInk.opacity(inkOpacity))

        let rows = Int((size.height / dy).rounded(.up)) + 2
        let cols = Int((size.width / dx).rounded(.up)) + 2
        // Walk rows then cols in a fixed order so the MT draw is deterministic;
        // every cell consumes exactly one MT value whether or not it inks.
        for row in 0..<rows {
            let cy = Double(row) * dy
            let xOffset = (row % 2 == 0) ? 0 : dx / 2
            for col in 0..<cols {
                let inked = rng.nextDouble() < frequency
                guard inked else { continue }
                let cx = Double(col) * dx + xOffset
                context.fill(hexPath(cx: cx, cy: cy, r: r), with: shading)
            }
        }
    }

    /// Pointy-top hexagon (d3-hexbin): first vertex straight up, flat left/right
    /// edges. point = (sin(a)*r, -cos(a)*r) for a in 60-degree steps.
    private func hexPath(cx: Double, cy: Double, r: Double) -> Path {
        var path = Path()
        for i in 0..<6 {
            let angle = Double(i) * .pi / 3
            let point = CGPoint(x: cx + sin(angle) * r, y: cy - cos(angle) * r)
            if i == 0 {
                path.move(to: point)
            } else {
                path.addLine(to: point)
            }
        }
        path.closeSubpath()
        return path
    }
}
