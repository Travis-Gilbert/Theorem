import SwiftUI

/// Motion discipline (addendum D5). The instrument register moves with restraint:
///  - Chrome (panels, the island, controls) moves crisply: fast, eased, no
///    spring overshoot. cubic-bezier(0.22, 1, 0.36, 1), 150-250ms.
///  - Spring is reserved for DATA motion (graph layout convergence, the fractal
///    wavefront) where physical settling reads as the data finding its shape.
///  - prefers-reduced-motion collapses chrome motion to ~0ms.
public enum TheoremMotion {
    /// Crisp chrome easing. Pass the view's `accessibilityReduceMotion` so the
    /// motion collapses to near-instant when the user asks for less motion.
    public static func chrome(_ duration: Double = 0.22, reduceMotion: Bool = false) -> Animation {
        reduceMotion
            ? .linear(duration: 0.001)
            : .timingCurve(0.22, 1.0, 0.36, 1.0, duration: duration)
    }

    /// Data settling: a gentle spring for graph/layout motion only. Reduced
    /// motion still collapses it (the data simply snaps to its final shape).
    public static func data(reduceMotion: Bool = false) -> Animation {
        reduceMotion
            ? .linear(duration: 0.001)
            : .spring(response: 0.42, dampingFraction: 0.86)
    }
}
