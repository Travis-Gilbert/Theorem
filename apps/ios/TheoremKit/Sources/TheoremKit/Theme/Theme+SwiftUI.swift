#if canImport(SwiftUI)
import SwiftUI

public extension RGBAColor {
    /// Bridge to a SwiftUI `Color` in the sRGB space.
    var swiftUIColor: Color {
        Color(.sRGB, red: red, green: green, blue: blue, opacity: alpha)
    }
}

public extension Theme {
    /// Resolve a role straight to a SwiftUI `Color`.
    func swiftUIColor(_ role: ColorRole) -> Color {
        color(role).swiftUIColor
    }

    /// SwiftUI color for an atom (ring color when present, else kind color).
    func swiftUIColorForAtom(kind: String, ring: Int?) -> Color {
        colorForAtom(kind: kind, ring: ring).swiftUIColor
    }
}

public extension TheoremTypography {
    enum DisplayWeight: Sendable { case regular, medium }

    /// Display face token (Berthold Akzidenz-Grotesk). Views call this, never a
    /// raw font name. Falls back to the system display face automatically if the
    /// face failed to register.
    static func display(_ size: CGFloat, _ weight: DisplayWeight = .regular) -> Font {
        let name = weight == .medium ? displayMediumName : displayRegularName
        return .custom(name, size: size)
    }

    /// Body face token (IBM Plex Sans SemiCondensed). Honestly falls back to the
    /// system body face until the Plex .ttf is bundled (`bodyIsBundled`).
    static func body(_ size: CGFloat) -> Font {
        bodyIsBundled ? .custom(bodyName, size: size) : .system(size: size)
    }
}

/// Injects the active `Theme` into the SwiftUI environment so any view can read
/// `@Environment(\.theme)` instead of threading the palette through inits.
public struct ThemeEnvironmentKey: EnvironmentKey {
    public static let defaultValue: Theme = .theorem
}

public extension EnvironmentValues {
    var theme: Theme {
        get { self[ThemeEnvironmentKey.self] }
        set { self[ThemeEnvironmentKey.self] = newValue }
    }
}
#endif
