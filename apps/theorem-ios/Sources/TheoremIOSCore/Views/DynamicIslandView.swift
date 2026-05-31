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
    @Binding var surface: AppSurface
    @Binding var projection: ProjectionID
    var centerTitle: String
    var projectionAvailability: [ProjectionAvailability]
    var theme: TheoremTheme
    /// Called when the user submits a query in search mode. Wired to the live
    /// substrate search in `TheoremRootView`.
    var onSubmitQuery: () -> Void = {}

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
        .shadow(color: .black.opacity(0.5), radius: 18, y: 8)
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
                    .autocorrectionDisabled()
                    .onSubmit {
                        if mode == .search { onSubmitQuery() }
                    }
            }

            controlDeck
        }
        .padding(18)
        .frame(width: 356)
        .frame(minHeight: 206)
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

    private var controlDeck: some View {
        VStack(spacing: 10) {
            HStack(spacing: 8) {
                ForEach(AppSurface.allCases) { item in
                    Button {
                        surface = item
                    } label: {
                        Image(systemName: item.symbolName)
                            .font(.system(size: 15, weight: .semibold))
                            .frame(maxWidth: .infinity)
                            .frame(height: 34)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(surface == item ? theme.surface : theme.textPrimary)
                    .background(surface == item ? theme.textPrimary : theme.background.opacity(0.42))
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .help(item.rawValue)
                }
            }

            HStack(spacing: 8) {
                ForEach(ProjectionID.allCases) { item in
                    let availability = projectionAvailability.first { $0.id == item }
                    let available = availability?.available == true
                    Button {
                        projection = item
                    } label: {
                        Label(item.title, systemImage: symbol(for: item))
                            .labelStyle(.titleAndIcon)
                            .font(TheoremFonts.mono(size: 10))
                            .lineLimit(1)
                            .frame(maxWidth: .infinity)
                            .frame(height: 34)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(projectionForeground(for: item, available: available))
                    .background(projectionBackground(for: item, available: available))
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .disabled(!available)
                    .help(availability?.reason ?? "")
                }
            }
        }
    }

    private func projectionForeground(for item: ProjectionID, available: Bool) -> Color {
        if !available {
            return theme.nodeDimmed
        }
        return projection == item ? theme.surface : theme.textPrimary
    }

    private func projectionBackground(for item: ProjectionID, available: Bool) -> Color {
        if !available {
            return Color.clear
        }
        return projection == item ? theme.textPrimary : theme.background.opacity(0.42)
    }

    private func symbol(for projection: ProjectionID) -> String {
        switch projection {
        case .forceGraph:
            "circle.hexagongrid"
        case .radialRings:
            "circle.dashed"
        case .treeLayout:
            "point.3.connected.trianglepath.dotted"
        case .fractalExpansion:
            "wave.3.forward"
        }
    }
}
