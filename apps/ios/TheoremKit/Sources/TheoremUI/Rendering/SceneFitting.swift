import CoreGraphics
import TheoremKit

/// Fit a set of virtual-space layout points to a screen rect: uniform scale so
/// the whole scene is visible with `padding`, centered. The Canvas renderers use
/// this to map the sliver's positions to pixels (the substrate camera). Pure, so
/// it is trivially correct and could be unit-tested headlessly.
enum SceneFitting {
    static func fit(
        _ positions: [String: LayoutPoint],
        in rect: CGRect,
        padding: CGFloat = 44
    ) -> [String: CGPoint] {
        guard !positions.isEmpty else { return [:] }

        var minX = Double.infinity, minY = Double.infinity
        var maxX = -Double.infinity, maxY = -Double.infinity
        for point in positions.values {
            minX = min(minX, point.x); maxX = max(maxX, point.x)
            minY = min(minY, point.y); maxY = max(maxY, point.y)
        }

        let worldW = max(1e-6, maxX - minX)
        let worldH = max(1e-6, maxY - minY)
        let availW = max(1e-6, Double(rect.width) - Double(padding) * 2)
        let availH = max(1e-6, Double(rect.height) - Double(padding) * 2)
        let scale = min(availW / worldW, availH / worldH)

        let worldCx = (minX + maxX) / 2
        let worldCy = (minY + maxY) / 2
        let screenCx = Double(rect.midX)
        let screenCy = Double(rect.midY)

        var out: [String: CGPoint] = [:]
        out.reserveCapacity(positions.count)
        for (id, point) in positions {
            out[id] = CGPoint(
                x: screenCx + (point.x - worldCx) * scale,
                y: screenCy + (point.y - worldCy) * scale
            )
        }
        return out
    }
}
