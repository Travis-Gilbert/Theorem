import SwiftUI

/// MT19937-seeded hex-blueprint substrate watermark (addendum D3): a faint field
/// of flat-top hexagons, ~8-15% of them inked in blueprint-blue, the rest left
/// as field. Deterministic and static per scene (the seed is a fingerprint of
/// the scene), so the same scene always prints the same texture, and the seed
/// changes between scenes. Sits behind the graph; content stays at full
/// contrast on top. This is a tiling, not a binning, so it does NOT use a
/// hexbin: every grid cell is visited in a fixed order and the MT draw decides
/// whether it inks.
struct HexWatermarkView: View {
    var seed: UInt32
    var theme: TheoremTheme
    /// Fraction of hexes inked. 0.08-0.15 reads as a watermark, not a checkerboard.
    var frequency: Double = 0.10
    /// Flat-top hex radius (center to vertex), in points. Smaller = finer grain.
    var radius: Double = 13
    /// Ink opacity. Kept low so the texture stays a faint substrate, not a pattern.
    var inkOpacity: Double = 0.09

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
        let colStep = r * 1.5
        let rowStep = r * sqrt(3.0)
        let stagger = rowStep / 2
        let shading = GraphicsContext.Shading.color(theme.blueprintInk.opacity(inkOpacity))

        // Walk columns then rows in a fixed order so the MT draw is deterministic;
        // every cell consumes exactly one MT value whether or not it inks.
        let cols = Int((size.width / colStep).rounded(.up)) + 2
        let rows = Int((size.height / rowStep).rounded(.up)) + 2
        for col in 0..<cols {
            let cx = Double(col) * colStep
            let yOffset = (col % 2 == 0) ? 0 : stagger
            for row in 0..<rows {
                let inked = rng.nextDouble() < frequency
                guard inked else { continue }
                let cy = Double(row) * rowStep + yOffset
                context.fill(hexPath(cx: cx, cy: cy, r: r), with: shading)
            }
        }
    }

    /// Flat-top hexagon: corners at 60-degree steps from angle 0 (vertices left
    /// and right, flat edges top and bottom).
    private func hexPath(cx: Double, cy: Double, r: Double) -> Path {
        var path = Path()
        for i in 0..<6 {
            let angle = Double(i) * .pi / 3
            let point = CGPoint(x: cx + r * cos(angle), y: cy + r * sin(angle))
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
