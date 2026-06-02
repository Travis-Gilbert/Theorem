import SwiftUI

/// Scramble-text reveal (addendum D6). Each character cycles through a scramble
/// set until its reveal time, then locks to the real glyph; the reveal front
/// advances left to right. Rendered MONOSPACE so the width never reflows during
/// the decode. A haptic fires on settle. Reduced-motion shows the final text
/// immediately. For results, snippets, and AI summaries — never chrome labels
/// or numeric readouts.
struct ScrambleText: View {
    let text: String
    var size: CGFloat = 12
    var color: Color
    /// Seconds between each character's reveal (the front speed).
    var perCharacter: Double = 0.04
    /// How long a character scrambles before it locks.
    var scrambleLead: Double = 0.42

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var start = Date.now
    @State private var isComplete = false

    private static let glyphs = Array("░▒▓█ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#%&@")

    private var total: Double { Double(text.count) * perCharacter + scrambleLead + 0.05 }

    var body: some View {
        Group {
            if reduceMotion || isComplete {
                Text(text)
            } else {
                TimelineView(.animation) { timeline in
                    Text(rendered(at: timeline.date.timeIntervalSince(start)))
                }
            }
        }
        .font(TheoremFonts.mono(size: size))
        .foregroundStyle(color)
        .task(id: text) {
            start = .now
            isComplete = false
            try? await Task.sleep(for: .seconds(total))
            isComplete = true
        }
        .sensoryFeedback(.selection, trigger: isComplete) { _, settled in settled }
    }

    private func rendered(at elapsed: Double) -> String {
        var out = ""
        out.reserveCapacity(text.count)
        for (i, ch) in text.enumerated() {
            if ch == " " || ch == "\n" {
                out.append(ch)
                continue
            }
            let revealAt = Double(i) * perCharacter + scrambleLead
            if elapsed >= revealAt {
                out.append(ch)                              // revealed + locked
            } else if elapsed >= revealAt - scrambleLead {
                let tick = Int(elapsed / 0.05)              // glyph changes ~20x/s
                let seed = (i &* 73856093) ^ (tick &* 19349663)
                out.append(ScrambleText.glyphs[abs(seed) % ScrambleText.glyphs.count])
            } else {
                out.append(" ")                             // not yet reached
            }
        }
        return out
    }
}
