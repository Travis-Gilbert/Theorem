import SwiftUI
import Pow

public enum DynamicIslandMode: String {
    case idle
    case search
    case ask
    case detail
    case room
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
    /// The room conversation, shown when the island morphs into .room.
    var room: CommonplaceRoom?
    /// Called when the user submits a query in search mode. Wired to the live
    /// substrate search in `TheoremRootView`.
    var onSubmitQuery: () -> Void = {}
    /// Open the on-device scene view (SceneOS) for the focused node.
    var onSceneOS: () -> Void = {}
    /// Run a deeper substrate search seeded from a node label.
    var onDeeperSearch: (String) -> Void = { _ in }

    @Namespace private var namespace
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// Near-white paper the island prints on. The page field is pure white; the
    /// island is a hair cooler so it reads as a distinct surface.
    private var islandPaper: Color { Color(red: 0.965, green: 0.976, blue: 0.988) }

    /// The island's background: clean near-white paper, clipped to the island's
    /// shape. (The hex-blueprint texture was removed per Travis.)
    private func islandBackground<S: Shape>(_ shape: S) -> some View {
        islandPaper.clipShape(shape)
    }

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
            // The pill's main area opens the room (the conversation). A node tap
            // opens that node's dossier; the search half opens search. All three
            // are the same box changing shape and context.
            Button {
                mode = .room
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "bubble.left.and.bubble.right.fill")
                        .font(.system(size: 12))
                    Text(room?.ask ?? centerTitle)
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
        .background(islandBackground(Capsule()))
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
            } else if mode == .room {
                if let room {
                    // The room conversation lives in the island now, not a slab.
                    IslandRoomView(room: room, theme: theme, onClose: { mode = .idle })
                } else {
                    Text("No room.")
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

                if mode == .search, let creditPreview {
                    CreditPreviewStrip(estimate: creditPreview, theme: theme)
                }

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
        .frame(minHeight: (mode == .detail || mode == .room) ? 300 : (mode == .ask ? 96 : 150))
        .background(islandBackground(RoundedRectangle(cornerRadius: 28, style: .continuous)))
        .overlay(
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .stroke(theme.textPrimary.opacity(0.08), lineWidth: 1)
        )
        .shadow(color: theme.ink.opacity(0.10), radius: 22, x: 0, y: 12)
        .matchedGeometryEffect(id: "island", in: namespace)
    }

    private var creditPreview: CommonplaceCreditEstimate? {
        guard let room,
              let registry = room.registry else { return nil }
        let clean = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let ask = clean.isEmpty ? room.ask : clean
        let routePlan = CommonplaceRouter().plan(query: ask, registry: registry)
        let toolBudget = CommonplaceToolUseBudget.preview(for: ask, features: routePlan.features)
        return CommonplaceCreditEstimator().estimate(
            routePlan: routePlan,
            registry: registry,
            toolBudget: toolBudget
        )
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
        case .room:
            "Room"
        }
    }

}

private struct CreditPreviewStrip: View {
    var estimate: CommonplaceCreditEstimate
    var theme: TheoremTheme

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: estimate.requiresConfirmation ? "exclamationmark.triangle.fill" : "creditcard")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(estimate.requiresConfirmation ? theme.signal : theme.blueprintInk)
                .frame(width: 18)

            Text("CREDITS")
                .font(TheoremFonts.label(size: 9))
                .tracking(0.7)
                .foregroundStyle(theme.textMuted)

            Text(estimate.creditRangeLabel)
                .font(TheoremFonts.mono(size: 11))
                .foregroundStyle(theme.ink)
                .monospacedDigit()

            Spacer(minLength: 8)

            Text("\(estimate.worstCaseParticipantIDs.count) voices")
                .font(TheoremFonts.mono(size: 10))
                .foregroundStyle(theme.textMuted)
        }
        .padding(.horizontal, 10)
        .frame(height: 30)
        .background(theme.chrome.opacity(0.72), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(theme.hairline, lineWidth: 1)
        )
    }
}
