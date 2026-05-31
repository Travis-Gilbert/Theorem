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

    public static let defaultPalette = TheoremTheme(
        nodeCore: Color(red: 0.86, green: 0.60, blue: 0.22),
        nodeWeb: Color(red: 0.18, green: 0.57, blue: 0.55),
        nodeTool: Color(red: 0.52, green: 0.38, blue: 0.70),
        nodeDimmed: Color(red: 0.55, green: 0.57, blue: 0.58),
        edge: Color(red: 0.34, green: 0.37, blue: 0.40).opacity(0.58),
        ringMatch: Color(red: 0.78, green: 0.25, blue: 0.17),
        ringAdjacent: Color(red: 0.18, green: 0.57, blue: 0.55),
        ringNearby: Color(red: 0.64, green: 0.48, blue: 0.76),
        background: Color(red: 0.94, green: 0.91, blue: 0.84),
        surface: Color(red: 0.98, green: 0.96, blue: 0.91),
        textPrimary: Color(red: 0.10, green: 0.11, blue: 0.12),
        textSecondary: Color(red: 0.38, green: 0.40, blue: 0.42)
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
