import SwiftUI
import TheoremIOSCore

@main
struct TheoremIOSApp: App {
    init() {
        // Register the bundled OFL faces (Archivo Black + IBM Plex Sans) before
        // any view reads a font token.
        TheoremFonts.registerBundledFonts()
    }

    var body: some Scene {
        WindowGroup {
            TheoremRootView()
        }
    }
}
