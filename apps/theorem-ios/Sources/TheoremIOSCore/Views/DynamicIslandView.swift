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
    /// Called when the user submits a query in search mode. Wired to the live
    /// substrate search in `TheoremRootView`.
    var onSubmitQuery: () -> Void = {}

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
            HStack {
                Text(modeTitle)
                    .font(TheoremFonts.mono(size: 11))
                    .textCase(.uppercase)
                    .foregroundStyle(theme.textSecondary)
                Spacer()
                Button {
                    mode = .idle
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 12, weight: .bold))
                }
                .buttonStyle(.plain)
            }

            if mode == .detail {
                Text(centerTitle)
                    .font(TheoremFonts.display(size: 28, relativeTo: .title))
                    .foregroundStyle(theme.textPrimary)
                    .lineLimit(2)
            } else {
                TextField(mode == .search ? "Search substrate" : "Ask over scene", text: $query)
                    .textFieldStyle(.plain)
                    .font(TheoremFonts.body(size: 18))
                    .foregroundStyle(theme.textPrimary)
                    .submitLabel(.search)
                    .autocorrectionDisabled()
                    .onSubmit {
                        if mode == .search { onSubmitQuery() }
                    }
            }

            // Algorithm (projection) switcher revealed only on search — the
            // expanded search surface is where you re-project. Honest-shape
            // gated (unavailable projections grey out). Hidden in ask/detail.
            if mode == .search {
                ProjectionPicker(
                    selection: $projection,
                    availability: projectionAvailability,
                    theme: theme
                )
            }
        }
        .padding(18)
        .frame(width: 356)
        .frame(minHeight: mode == .search ? 150 : 92)
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
