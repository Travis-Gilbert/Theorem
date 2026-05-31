import SwiftUI

/// The node dossier that lives INSIDE the expanded Dynamic Island (addendum D8).
/// Tapping a node morphs the island into this: the node's identity plus a choice
/// menu of how to interact with it. The dossier IS the island expanding around
/// the thing you tapped, not a slab below the graph.
///
/// The default choice is a substrate-generated summary of the node in the
/// context of the rest of the graph (real `/ask/` compose, streamed). The others
/// are SceneOS (the on-device scene view), a deeper search seeded from the node,
/// and opening the source link. Nothing here is faked: the summary is a real
/// request with honest loading / error states, and Link is disabled when the
/// node carries no source URL.
struct NodeDossierView: View {
    let atom: SceneAtom
    var theme: TheoremTheme
    var searchClient: TheoremSearchClient
    var onSceneOS: () -> Void
    var onDeeperSearch: (String) -> Void
    var onClose: () -> Void

    enum Choice: String, CaseIterable, Identifiable {
        case summary, scene, search, link
        var id: String { rawValue }
        var title: String {
            switch self {
            case .summary: "Summary"
            case .scene: "SceneOS"
            case .search: "Search"
            case .link: "Link"
            }
        }
        var symbol: String {
            switch self {
            case .summary: "text.alignleft"
            case .scene: "square.stack.3d.up.fill"
            case .search: "magnifyingglass"
            case .link: "arrow.up.right.square"
            }
        }
    }

    enum SummaryState: Equatable {
        case loading
        case loaded(String)
        case failed(String)
    }

    @State private var choice: Choice = .summary
    @State private var summary: SummaryState = .loading
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.openURL) private var openURL

    private var title: String { atom.label ?? atom.id }

    private var sourceURL: URL? {
        let raw = atom.metadata["url"]?.stringValue
            ?? atom.sourceRefs.compactMap { $0.metadata["url"]?.stringValue }.first
        guard let raw, let url = URL(string: raw), (url.scheme ?? "").hasPrefix("http") else { return nil }
        return url
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            choiceBar
            Divider().overlay(theme.hairline)
            content
                .frame(maxWidth: .infinity, alignment: .leading)
                .animation(TheoremMotion.chrome(reduceMotion: reduceMotion), value: choice)
        }
        .task(id: atom.id) {
            choice = .summary
            await loadSummary(force: true)
        }
    }

    private var header: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 3) {
                Text("NODE")
                    .font(TheoremFonts.label(size: 10)).tracking(0.9)
                    .foregroundStyle(theme.textMuted)
                Text(title)
                    .font(TheoremFonts.display(size: 21, relativeTo: .title2))
                    .foregroundStyle(theme.ink)
                    .lineLimit(2)
            }
            Spacer(minLength: 8)
            Button(action: onClose) {
                Image(systemName: "xmark").font(.system(size: 12, weight: .bold))
                    .foregroundStyle(theme.textMuted)
            }
            .buttonStyle(.plain)
        }
    }

    private var choiceBar: some View {
        HStack(spacing: 6) {
            ForEach(Choice.allCases) { item in
                let disabled = item == .link && sourceURL == nil
                let active = item == choice
                Button { choice = item } label: {
                    HStack(spacing: 5) {
                        Image(systemName: item.symbol).font(.system(size: 11, weight: .semibold))
                        Text(item.title).font(TheoremFonts.label(size: 11))
                    }
                    .frame(maxWidth: .infinity).frame(height: 32)
                }
                .buttonStyle(.plain)
                .foregroundStyle(active ? theme.field : (disabled ? theme.textMuted : theme.ink))
                .background(active ? theme.ink : theme.chrome, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .disabled(disabled)
                .opacity(disabled ? 0.45 : 1)
            }
        }
    }

    @ViewBuilder private var content: some View {
        switch choice {
        case .summary:
            summaryContent
        case .scene:
            actionRow(
                icon: "square.stack.3d.up.fill",
                text: "Render this node as a scene on the substrate canvas.",
                cta: "Open scene",
                action: onSceneOS)
        case .search:
            actionRow(
                icon: "magnifyingglass",
                text: "Run a deeper substrate search seeded from this node.",
                cta: "Search deeper",
                action: { onDeeperSearch(title) })
        case .link:
            if let url = sourceURL {
                actionRow(
                    icon: "arrow.up.right.square",
                    text: url.host ?? url.absoluteString,
                    cta: "Open link",
                    action: { openURL(url) })
            } else {
                honest("No source link on this node.")
            }
        }
    }

    @ViewBuilder private var summaryContent: some View {
        switch summary {
        case .loading:
            HStack(spacing: 9) {
                ProgressView().controlSize(.small).tint(theme.signal)
                Text("The substrate is composing a summary in the context of your graph\u{2026}")
                    .font(TheoremFonts.body(size: 13)).foregroundStyle(theme.textMuted)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case .loaded(let text):
            ScrollView {
                Text(text)
                    .font(TheoremFonts.body(size: 14, relativeTo: .body))
                    .foregroundStyle(theme.ink)
                    .lineSpacing(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 220)
        case .failed(let message):
            VStack(alignment: .leading, spacing: 8) {
                honest(message)
                Button("Retry") { Task { await loadSummary(force: true) } }
                    .font(TheoremFonts.label(size: 12)).foregroundStyle(theme.signal)
            }
        }
    }

    private func actionRow(icon: String, text: String, cta: String, action: @escaping () -> Void) -> some View {
        VStack(alignment: .leading, spacing: 11) {
            Text(text).font(TheoremFonts.body(size: 13)).foregroundStyle(theme.textSecondary)
            Button(action: action) {
                Label(cta, systemImage: icon).font(TheoremFonts.label(size: 13))
                    .foregroundStyle(theme.field)
                    .padding(.horizontal, 14).padding(.vertical, 9)
                    .background(theme.ink, in: Capsule())
            }
            .buttonStyle(.plain)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func honest(_ message: String) -> some View {
        Text(message)
            .font(TheoremFonts.body(size: 13)).foregroundStyle(theme.textMuted)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func loadSummary(force: Bool) async {
        if case .loaded = summary, !force { return }
        summary = .loading
        do {
            let result = try await searchClient.ask(query: title)
            summary = .loaded(result.answer)
        } catch {
            let message = (error as? TheoremSearchError)?.message ?? "Summary unavailable right now."
            summary = .failed(message)
        }
    }
}
