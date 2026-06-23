import SwiftUI
import TheoremIOSCore

@main
struct TheoremIOSApp: App {
    init() {
        // Register the bundled OFL faces (Karrik, IBM Plex Sans, JetBrains Mono,
        // Terminal Grotesque, jgs9) before any view reads a font token.
        TheoremFonts.registerBundledFonts()
    }

    var body: some Scene {
        WindowGroup {
            TheoremRootView()
        }
    }
}
