import SwiftUI
import TheoremIOSCore

/// iOS app entry. Thin shell: registers the bundled OFL faces, then hands off to
/// `TheoremRootView` (the paged 5-surface IA + Dynamic Island) from
/// `TheoremIOSCore`. The app target carries only this @main; all logic + UI live
/// in the package so they stay `swift build`-testable on the host.
@main
struct TheoremApp: App {
    init() {
        TheoremFonts.registerBundledFonts()
    }

    var body: some Scene {
        WindowGroup {
            TheoremRootView()
        }
    }
}
