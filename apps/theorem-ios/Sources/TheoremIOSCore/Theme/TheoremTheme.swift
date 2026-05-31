import SwiftUI
import CoreText

/// The instrument register (SPEC-THEOREM-IOS-V1-ADDENDUM-DESIGN.md): a Braun-era
/// workbench. Warm off-white matte surfaces, warm-near-black ink, oxblood as the
/// single functional accent (selection/active, 2-3% of any view), blueprint-blue
/// as a second structural ink (the hex watermark). NOT dark, NOT beige-cozy.
///
/// The stored fields keep their original names (existing views read them); the
/// VALUES are remapped to instrument tokens, and `field/chrome/ink/signal/rule/…`
/// aliases give new code the design-language names.
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
    /// Deep cyanotype ink — the MT19937 hex watermark + optional second structural
    /// ink. An ink, never a surface (do not tint the chrome blue).
    public var blueprintInk: Color

    // Instrument-language aliases (map onto the stored fields).
    public var field: Color { background }      // #F6F5F2 working surface
    public var chrome: Color { surface }        // #EAE8E2 chrome zones / island
    public var pebble: Color { nodeDimmed }     // secondary fills / disabled
    public var ink: Color { textPrimary }       // primary type + graph linework
    public var signal: Color { ringMatch }      // OXBLOOD — selection/active only
    public var rule: Color { edge }             // zone-boundary + graph edge lines
    public var textMuted: Color { textSecondary }
    public var hairline: Color { textPrimary.opacity(0.12) }
    public var ruleStrong: Color { textPrimary.opacity(0.72) }

    /// The instrument palette. Cool-neutral, NOT warm: the field + chrome are
    /// paper-cool (blue channel >= red) so the ground reads as clean drafting
    /// paper, never cream/beige. The graph is monochrome — node fills are the
    /// field color (drawn as ink outlines in `TheoremSceneView`); the only hues
    /// are oxblood `signal` on selection and `blueprintInk` in the hex watermark.
    /// (Deviates from the addendum's warm hexes per Travis's "still reads beige"
    /// feedback; the warm field #F6F5F2 + taupe chrome #EAE8E2 read beige.)
    public static let defaultPalette = TheoremTheme(
        nodeCore: Color(red: 1.0, green: 1.0, blue: 1.0),  // field (node fill)
        nodeWeb: Color(red: 1.0, green: 1.0, blue: 1.0),   // field
        nodeTool: Color(red: 1.0, green: 1.0, blue: 1.0),  // field
        nodeDimmed: Color(red: 0.761, green: 0.780, blue: 0.808),// pebble #C2C7CE (cool)
        edge: Color(red: 0.133, green: 0.145, blue: 0.165).opacity(0.42), // rule
        ringMatch: Color(red: 0.482, green: 0.180, blue: 0.149), // OXBLOOD #7B2E26
        ringAdjacent: Color(red: 1.0, green: 1.0, blue: 1.0), // field
        ringNearby: Color(red: 1.0, green: 1.0, blue: 1.0),   // field
        background: Color(red: 1.0, green: 1.0, blue: 1.0), // FIELD #FFFFFF (pure white)
        surface: Color(red: 0.898, green: 0.910, blue: 0.929),   // CHROME #E5E8ED (cool grey)
        textPrimary: Color(red: 0.133, green: 0.145, blue: 0.165),// INK #22252A (cool near-black)
        textSecondary: Color(red: 0.133, green: 0.145, blue: 0.165).opacity(0.62), // muted
        blueprintInk: Color(red: 0.122, green: 0.251, blue: 0.388) // #1F4063
    )
}

/// Type tokens — the instrument font stack (all OFL, bundled):
///   - display: Karrik (hero + section headers; replaces Berthold/Archivo Black —
///     display presence without the heaviness)
///   - body: IBM Plex Sans (variable)
///   - mono / data: JetBrains Mono (numbers, IDs, edge-label values; tabular)
///   - flavor: Terminal Grotesque (code/flavor labels, scramble-text — NOT data)
/// Views read these tokens; no hardcoded font names in views.
public enum TheoremFonts {
    public static let displayFamily = "Karrik"
    public static let bodyFamily = "IBM Plex Sans"
    public static let dataFamily = "JetBrains Mono"
    public static let flavorFamily = "Terminal Grotesque"

    public static func display(size: CGFloat, relativeTo textStyle: Font.TextStyle = .largeTitle) -> Font {
        .custom(displayFamily, size: size, relativeTo: textStyle)
    }

    public static func body(size: CGFloat, relativeTo textStyle: Font.TextStyle = .body) -> Font {
        .custom(bodyFamily, size: size, relativeTo: textStyle)
    }

    /// Instrument label (eyebrow / caption): Plex Sans semibold, set uppercase +
    /// tracked at the call site.
    public static func label(size: CGFloat, relativeTo textStyle: Font.TextStyle = .caption) -> Font {
        .custom(bodyFamily, size: size, relativeTo: textStyle).weight(.semibold)
    }

    /// Data readouts (numbers, IDs, edge values) — JetBrains Mono, tabular.
    public static func mono(size: CGFloat, relativeTo textStyle: Font.TextStyle = .caption) -> Font {
        .custom(dataFamily, size: size, relativeTo: textStyle)
    }

    /// Code-interface / flavor chrome — Terminal Grotesque. NOT for numeric data.
    public static func flavor(size: CGFloat, relativeTo textStyle: Font.TextStyle = .caption) -> Font {
        .custom(flavorFamily, size: size, relativeTo: textStyle)
    }

    /// Register every bundled .ttf with CoreText (SwiftPM resources are not
    /// auto-registered). Idempotent. Missing face → `.custom` falls back to the
    /// system face (honest, never a crash).
    public static func registerBundledFonts() {
        guard !didRegister else { return }
        didRegister = true
        let urls = (Bundle.module.urls(forResourcesWithExtension: "ttf", subdirectory: nil) ?? [])
            + (Bundle.module.urls(forResourcesWithExtension: "ttf", subdirectory: "Fonts") ?? [])
        var seen = Set<String>()
        for url in urls where seen.insert(url.lastPathComponent).inserted {
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }

    private nonisolated(unsafe) static var didRegister = false
}
