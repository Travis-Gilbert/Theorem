import Foundation

/// A platform-agnostic RGBA color (components in 0...1).
///
/// Kept free of SwiftUI so the theme + role logic is unit-testable on the host
/// without a UI framework. `Theme+SwiftUI.swift` bridges it to `Color`.
public struct RGBAColor: Hashable, Sendable {
    public var red: Double
    public var green: Double
    public var blue: Double
    public var alpha: Double

    public init(red: Double, green: Double, blue: Double, alpha: Double = 1) {
        self.red = red
        self.green = green
        self.blue = blue
        self.alpha = alpha
    }

    /// Parse `#RGB`, `#RRGGBB`, or `#RRGGBBAA` (the `#` is optional). Returns nil
    /// on malformed input so a bad palette entry surfaces instead of silently
    /// rendering black.
    public init?(hex: String) {
        var s = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        if s.hasPrefix("#") { s.removeFirst() }
        let scalars = s.unicodeScalars
        guard scalars.allSatisfy({ $0.properties.isASCIIHexDigit }) else { return nil }

        func channel(_ substr: Substring) -> Double? {
            guard let v = Int(substr, radix: 16) else { return nil }
            return Double(v) / 255.0
        }

        switch s.count {
        case 3: // #RGB -> expand each nibble
            let chars = Array(s)
            guard let r = channel("\(chars[0])\(chars[0])"),
                  let g = channel("\(chars[1])\(chars[1])"),
                  let b = channel("\(chars[2])\(chars[2])") else { return nil }
            self.init(red: r, green: g, blue: b)
        case 6:
            let c = Array(s)
            guard let r = channel("\(c[0])\(c[1])"),
                  let g = channel("\(c[2])\(c[3])"),
                  let b = channel("\(c[4])\(c[5])") else { return nil }
            self.init(red: r, green: g, blue: b)
        case 8:
            let c = Array(s)
            guard let r = channel("\(c[0])\(c[1])"),
                  let g = channel("\(c[2])\(c[3])"),
                  let b = channel("\(c[4])\(c[5])"),
                  let a = channel("\(c[6])\(c[7])") else { return nil }
            self.init(red: r, green: g, blue: b, alpha: a)
        default:
            return nil
        }
    }
}

public extension RGBAColor {
    /// Visible fallback used when a role is missing from a palette (magenta, so
    /// a gap is obvious in development rather than blending in).
    static let fallback = RGBAColor(red: 1, green: 0, blue: 1)
}
