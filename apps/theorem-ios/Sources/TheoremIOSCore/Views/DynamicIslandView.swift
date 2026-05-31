import SwiftUI

public enum DynamicIslandMode {
    case idle
    case search
    case ask
    case detail
}

struct DynamicIslandView: View {
    @Binding var mode: DynamicIslandMode
    @Binding var query: String
    var centerTitle: String
    var theme: TheoremTheme

    @Namespace private var namespace

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
        .animation(.spring(response: 0.34, dampingFraction: 0.82), value: mode)
    }

    private var collapsed: some View {
        HStack(spacing: 0) {
            Button {
                mode = .ask
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
        .foregroundStyle(theme.surface)
        .frame(width: 326, height: 48)
        .background(.black, in: Capsule())
        .matchedGeometryEffect(id: "island", in: namespace)
        .sensoryFeedback(.selection, trigger: centerTitle)
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
            }
        }
        .padding(18)
        .frame(width: 356)
        .frame(minHeight: 116)
        .background(theme.surface, in: RoundedRectangle(cornerRadius: 28, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .stroke(theme.textPrimary.opacity(0.08), lineWidth: 1)
        )
        .shadow(color: .black.opacity(0.16), radius: 30, x: 0, y: 18)
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
