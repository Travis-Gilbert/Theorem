import Foundation
import CoreText
import CoreGraphics

/// Type tokens + runtime registration of the bundled display faces.
///
/// Two tokens (spec "Theming"): `display*` is Berthold Akzidenz-Grotesk
/// (Travis supplied the licensed files; they ship in the binary as
/// Bundle.module resources), `body*` is IBM Plex Sans SemiCondensed (SIL OFL).
/// Views read these tokens; no hardcoded font names in views.
///
/// Registration is at runtime via CoreText because SwiftPM-bundled fonts are not
/// auto-registered the way an app target's `UIAppFonts` Info.plist entries are.
public enum TheoremTypography {
    /// PostScript names (read from the .ttf headers via fc-scan), the strings
    /// SwiftUI `Font.custom(_:size:)` resolves once the faces are registered.
    public static let displayRegularName = "AkzidenzGroteskBE-Regular"
    public static let displayMediumName = "AGSchulbuchBQ-Medium"

    /// IBM Plex Sans SemiCondensed PostScript name. The .ttf is not yet bundled
    /// (it is freely available under the SIL OFL); until it is, body text falls
    /// back to the system face. `bodyIsBundled` reports the honest state.
    public static let bodyName = "IBMPlexSansCondensed-Regular"

    /// Whether the body face actually shipped (vs. system fallback). False until
    /// the Plex .ttf is added to Resources/Fonts.
    public static var bodyIsBundled: Bool {
        fontURLs().contains { url in
            url.deletingPathExtension().lastPathComponent.localizedCaseInsensitiveContains("plex")
        }
    }

    /// Register every bundled .ttf with CoreText. Idempotent (already-registered
    /// is treated as success). Returns the PostScript names that are available
    /// after the call. Call once at app launch (and the verify harness calls it).
    @discardableResult
    public static func registerBundledFonts() -> [String] {
        var registered: [String] = []
        for url in fontURLs() {
            var errorRef: Unmanaged<CFError>?
            let ok = CTFontManagerRegisterFontsForURL(url as CFURL, .process, &errorRef)
            let alreadyDone = !ok && isAlreadyRegistered(errorRef?.takeRetainedValue())
            guard ok || alreadyDone else { continue }
            if let psName = postScriptName(of: url) {
                registered.append(psName)
            }
        }
        return registered
    }

    // MARK: - Internals

    /// Every bundled .ttf URL, deduped by filename. Robust to whether SwiftPM
    /// flattened `Resources/Fonts` to the bundle root (.process) or kept a
    /// `Fonts` subdirectory.
    private static func fontURLs() -> [URL] {
        let roots = (Bundle.module.urls(forResourcesWithExtension: "ttf", subdirectory: nil) ?? [])
        let inFonts = (Bundle.module.urls(forResourcesWithExtension: "ttf", subdirectory: "Fonts") ?? [])
        var seen = Set<String>()
        var out: [URL] = []
        for url in roots + inFonts where seen.insert(url.lastPathComponent).inserted {
            out.append(url)
        }
        return out
    }

    private static func postScriptName(of url: URL) -> String? {
        guard let provider = CGDataProvider(url: url as CFURL),
              let cgFont = CGFont(provider),
              let name = cgFont.postScriptName as String? else {
            return nil
        }
        return name
    }

    private static func isAlreadyRegistered(_ error: CFError?) -> Bool {
        guard let error else { return false }
        // CTFontManagerError.alreadyRegistered == 105.
        return CFErrorGetCode(error) == 105
    }
}
