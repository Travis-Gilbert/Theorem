import SwiftUI
import Pow

public enum DynamicIslandMode: String {
    case idle
    case search
    case ask
    case detail
}

struct DynamicIslandView: View {
    @Binding var mode: DynamicIslandMode
    @Binding var query: String
    @Binding var projection: ProjectionID
    var centerTitle: String
    var projectionAvailability: [ProjectionAvailability]
    var theme: TheoremTheme
    /// The node the dossier opens on in .detail mode (the tapped node, else the
    /// center node). When set and mode == .detail, the island IS the dossier.
    var focusedAtom: SceneAtom?
    /// Drives the dossier's real substrate summary (the /ask/ compose stream).
    var searchClient: TheoremSearchClient
    /// Called when the user submits a query in search mode. Wired to the live
    /// substrate search in `TheoremRootView`.
    var onSubmitQuery: () -> Void = {}
    /// Open the on-device scene view (SceneOS) for the focused node.
    var onSceneOS: () -> Void = {}
    /// Run a deeper substrate search seeded from a node label.
    var onDeeperSearch: (String) -> Void = { _ in }

    @Namespace private var namespace
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            if mode == .idle {
                collapsed
                    .transition(.opacity.combined(with: .scale(scale: 0.96)))
            } else {
                expanded
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        // Chrome motion: crisp, no spring overshoot (addendum D5). Collapses to
        // near-instant under reduced-motion.
        .animation(TheoremMotion.chrome(0.26, reduceMotion: reduceMotion), value: mode)
    }

    private var collapsed: some View {
        HStack(spacing: 0) {
            Button {
                mode = .detail
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "sparkles")
                    Text(centerTitle)
                        .lineLimit(1)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.leading, 16)
            }
            .buttonStyle(.plain)

            Divider()
                .frame(height: 22)
                .background(theme.textSecondary.opacity(0.28))

            Button {
                mode = .search
            } label: {
                Image(systemName: "magnifyingglass")
                    .frame(width: 48)
            }
            .buttonStyle(.plain)
        }
        .font(TheoremFonts.body(size: 14, relativeTo: .callout))
        .foregroundStyle(theme.textPrimary)
        .frame(width: 326, height: 48)
        .background(theme.surface, in: Capsule())
        .overlay(Capsule().stroke(theme.textPrimary.opacity(0.10), lineWidth: 1))
        .shadow(color: theme.ink.opacity(0.12), radius: 10, y: 3)
        .matchedGeometryEffect(id: "island", in: namespace)
        .sensoryFeedback(.selection, trigger: centerTitle)
        // One restrained Pow moment: a faint oxblood-signal glow when the center
        // node changes (a new scene resolved). Off under reduced-motion.
        .changeEffect(
            .glow(color: theme.signal.opacity(0.45), radius: 16),
            value: centerTitle,
            isEnabled: !reduceMotion
        )
    }

    private var expanded: some View {
        VStack(alignment: .leading, spacing: 12) {
            if mode == .detail {
                if let focusedAtom {
                    // The node dossier IS the island here: it brings its own
                    // header, close, choice menu, and the real substrate summary.
                    NodeDossierView(
                        atom: focusedAtom,
                        theme: theme,
                        searchClient: searchClient,
                        onSceneOS: onSceneOS,
                        onDeeperSearch: onDeeperSearch,
                        onClose: { mode = .idle }
                    )
                } else {
                    Text("No node selected.")
                        .font(TheoremFonts.body(size: 13))
                        .foregroundStyle(theme.textMuted)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            } else {
                HStack {
                    Text(modeTitle)
                        .font(TheoremFonts.mono(size: 11))
                        .textCase(.uppercase)
                        .foregroundStyle(theme.textSecondary)
                    Spacer()
                    Button { mode = .idle } label: {
                        Image(systemName: "xmark").font(.system(size: 12, weight: .bold))
                    }
                    .buttonStyle(.plain)
                }

                TextField(mode == .search ? "Search substrate" : "Ask over scene", text: $query)
                    .textFieldStyle(.plain)
                    .font(TheoremFonts.body(size: 18))
                    .foregroundStyle(theme.textPrimary)
                    .submitLabel(.search)
                    .autocorrectionDisabled()
                    .onSubmit { if mode == .search { onSubmitQuery() } }

                // Algorithm (projection) switcher revealed only on search — the
                // expanded search surface is where you re-project. Honest-shape
                // gated (unavailable projections grey out).
                if mode == .search {
                    ProjectionPicker(
                        selection: $projection,
                        availability: projectionAvailability,
                        theme: theme
                    )
                }
            }
        }
        .padding(18)
        .frame(width: 356)
        .frame(minHeight: mode == .detail ? 300 : (mode == .ask ? 96 : 150))
        .background(theme.surface, in: RoundedRectangle(cornerRadius: 28, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .stroke(theme.textPrimary.opacity(0.08), lineWidth: 1)
        )
        .shadow(color: theme.ink.opacity(0.10), radius: 22, x: 0, y: 12)
        .matchedGeometryEffect(id: "island", in: namespace)
    }

    private var modeTitle: String {
        switch mode {
        case .idle:
            "Theorem"
        case .search:
            "Search"
        case .ask:
            "Ask"
        case .detail:
            "Center node"
        }
    }

}
