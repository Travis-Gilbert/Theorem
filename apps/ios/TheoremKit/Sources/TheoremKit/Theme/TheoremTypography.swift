import Foundation
import CoreText
import CoreGraphics

/// Type tokens + runtime registration of the bundled faces.
///
/// Two distinct faces (Travis: not a one-font app), both OFL so they ship in the
/// binary: `display*` is Archivo Black (the free stand-in for Berthold
/// Akzidenz-Grotesk, which has no embed license and is NOT shipped); `body*` is
/// IBM Plex Sans (variable). Views read these tokens; no hardcoded font names in
/// views.
///
/// Registration is at runtime via CoreText because SwiftPM-bundled fonts are not
/// auto-registered the way an app target's `UIAppFonts` Info.plist entries are.
public enum TheoremTypography {
    /// Family name SwiftUI `Font.custom(_:size:)` resolves once the face is
    /// registered. Archivo Black is a single heavy weight, so regular and medium
    /// both resolve to it.
    public static let displayRegularName = "Archivo Black"
    public static let displayMediumName = "Archivo Black"

    /// IBM Plex Sans (variable) family name; the body face, bundled OFL.
    public static let bodyName = "IBM Plex Sans"

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
