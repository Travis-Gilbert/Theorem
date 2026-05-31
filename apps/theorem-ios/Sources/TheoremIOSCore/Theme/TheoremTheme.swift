import SwiftUI
import CoreText

public struct TheoremTheme: Equatable, Sendable {
    public var nodeCore: Color
    public var nodeWeb: Color
    public var nodeTool: Color
    public var nodeDimmed: Color
    public var edge: Color
    public var ringMatch: Color
    public var ringAdjacent: Color
    public var ringNearby: Color
    public var background: Color
    public var surface: Color
    public var textPrimary: Color
    public var textSecondary: Color

    /// Dark, cool-neutral ground (no beige). Jewel-tone node colors are tuned to
    /// pop against the near-black surface instead of going muted on cream;
    /// text/surface steps follow a perceptually-uniform dark scale. This is the
    /// graph-exploration default — the canvas should recede, the data should glow.
    public static let defaultPalette = TheoremTheme(
        nodeCore: Color(red: 0.96, green: 0.71, blue: 0.27),    // amber / engine core
        nodeWeb: Color(red: 0.24, green: 0.72, blue: 0.79),     // teal / substrate-web
        nodeTool: Color(red: 0.62, green: 0.52, blue: 0.93),    // violet / tooling
        nodeDimmed: Color(red: 0.44, green: 0.47, blue: 0.51),  // muted gray
        edge: Color(red: 0.42, green: 0.46, blue: 0.52).opacity(0.55),
        ringMatch: Color(red: 0.96, green: 0.42, blue: 0.25),   // hot coral / direct match
        ringAdjacent: Color(red: 0.90, green: 0.58, blue: 0.36),// warm mid
        ringNearby: Color(red: 0.56, green: 0.67, blue: 0.64),  // cool sage fade
        background: Color(red: 0.063, green: 0.071, blue: 0.086),// near-black, slightly cool
        surface: Color(red: 0.114, green: 0.129, blue: 0.153),  // lifted panel / island
        textPrimary: Color(red: 0.93, green: 0.94, blue: 0.95), // near-white
        textSecondary: Color(red: 0.60, green: 0.62, blue: 0.65)// muted
    )
}

/// Type tokens. Two distinct faces (Travis: not a one-font app), both OFL so
/// they ship in the binary:
///   - display: Archivo Black — a heavy grotesque standing in for Berthold
///     Akzidenz-Grotesk (which has no embed license). For headers, the Dynamic
///     Island readout, surface titles.
///   - body: IBM Plex Sans (variable; `.fontWeight()` drives the weight axis).
///   - mono: the system monospaced face for technical labels.
/// Views read these tokens; no hardcoded font names in views.
public enum TheoremFonts {
    public static let displayFamily = "Archivo Black"
    public static let bodyFamily = "IBM Plex Sans"

    public static func display(size: CGFloat, relativeTo textStyle: Font.TextStyle = .largeTitle) -> Font {
        .custom(displayFamily, size: size, relativeTo: textStyle)
    }

    public static func body(size: CGFloat, relativeTo textStyle: Font.TextStyle = .body) -> Font {
        .custom(bodyFamily, size: size, relativeTo: textStyle)
    }

    public static func mono(size: CGFloat, relativeTo textStyle: Font.TextStyle = .caption) -> Font {
        .system(size: size, weight: .medium, design: .monospaced)
    }

    /// Register the bundled OFL faces with CoreText. SwiftPM-bundled fonts are
    /// not auto-registered (unlike an app target's `UIAppFonts` Info.plist), so
    /// the app calls this once at launch. Idempotent. If a face is missing,
    /// `.custom` falls back to the system face (honest, never a crash).
    public static func registerBundledFonts() {
        guard !didRegister else { return }
        didRegister = true
        for name in ["ArchivoBlack-Regular", "IBMPlexSans"] {
            let url = Bundle.module.url(forResource: name, withExtension: "ttf")
                ?? Bundle.module.url(forResource: name, withExtension: "ttf", subdirectory: "Fonts")
            guard let url else { continue }
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }

    private nonisolated(unsafe) static var didRegister = false
}
