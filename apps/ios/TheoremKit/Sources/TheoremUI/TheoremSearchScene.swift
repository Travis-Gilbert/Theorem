import Observation
import SwiftUI
import TheoremKit

public enum TheoremSearchSceneState: Equatable, Sendable {
    case idle
    case searching(String)
    case answer(text: String, executor: String)
    case failed(String)

    var isSearching: Bool {
        if case .searching = self { return true }
        return false
    }
}

@MainActor
@Observable
public final class TheoremSearchViewModel {
    public let sceneModel: SceneViewModel
    public var query: String
    public private(set) var state: TheoremSearchSceneState
    public private(set) var lastTurn: TheoremSearchTurn?

    private let client: TheoremSearchClient

    public init(
        client: TheoremSearchClient = TheoremSearchClient(),
        sceneModel: SceneViewModel = SceneViewModel(),
        query: String = ""
    ) {
        self.client = client
        self.sceneModel = sceneModel
        self.query = query
        self.state = .idle
    }

    public func submit() {
        let submitted = query
        Task { await search(submitted) }
    }

    public func search(_ rawQuery: String) async {
        let cleanQuery = rawQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanQuery.isEmpty else { return }

        state = .searching(cleanQuery)
        do {
            let turn = try await client.search(query: cleanQuery)
            lastTurn = turn
            sceneModel.load(turn.scene)
            state = .answer(text: turn.answerText, executor: turn.chosenExecutor)
        } catch {
            state = .failed(String(describing: error))
        }
    }
}

public struct TheoremSearchScene: View {
    @Bindable public var model: TheoremSearchViewModel
    public var theme: Theme

    public init(model: TheoremSearchViewModel, theme: Theme = .theorem) {
        self.model = model
        self.theme = theme
    }

    public var body: some View {
        ZStack {
            SceneView(model: model.sceneModel, theme: theme)

            VStack(spacing: 10) {
                Spacer()
                statusReadout
                TheoremSearchBar(model: model, theme: theme)
            }
            .padding(.horizontal, 18)
            .padding(.bottom, 24)
        }
    }

    @ViewBuilder
    private var statusReadout: some View {
        switch model.state {
        case .idle:
            EmptyView()
        case .searching(let query):
            SearchStatusPill(text: "Searching Theorem: \(query)", theme: theme)
        case .answer(let text, let executor):
            SearchAnswerPill(text: text, executor: executor, theme: theme)
        case .failed(let message):
            SearchStatusPill(text: message, theme: theme)
        }
    }
}

public struct TheoremSearchBar: View {
    @Bindable public var model: TheoremSearchViewModel
    public var theme: Theme

    public init(model: TheoremSearchViewModel, theme: Theme = .theorem) {
        self.model = model
        self.theme = theme
    }

    public var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(theme.swiftUIColor(.textSecondary))

            TextField("Ask Theorem", text: $model.query)
                .textFieldStyle(.plain)
                .font(.system(size: 16, weight: .medium, design: .rounded))
                .foregroundStyle(theme.swiftUIColor(.textPrimary))
                .submitLabel(.search)
                .onSubmit { model.submit() }

            Button(action: model.submit) {
                Image(systemName: model.state.isSearching ? "hourglass" : "arrow.up.circle.fill")
                    .font(.system(size: 24, weight: .semibold))
                    .foregroundStyle(theme.swiftUIColor(.ringMatch))
                    .frame(width: 32, height: 32)
            }
            .buttonStyle(.plain)
            .disabled(model.state.isSearching || model.query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .padding(.leading, 16)
        .padding(.trailing, 8)
        .padding(.vertical, 10)
        .background(.ultraThinMaterial, in: Capsule())
        .overlay(
            Capsule()
                .stroke(theme.swiftUIColor(.edge).opacity(0.55), lineWidth: 1)
        )
    }
}

private struct SearchStatusPill: View {
    let text: String
    let theme: Theme

    var body: some View {
        Text(text)
            .font(.system(size: 12, weight: .medium, design: .monospaced))
            .foregroundStyle(theme.swiftUIColor(.textSecondary))
            .lineLimit(2)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(.ultraThinMaterial, in: Capsule())
    }
}

private struct SearchAnswerPill: View {
    let text: String
    let executor: String
    let theme: Theme

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(text)
                .font(.system(size: 13, weight: .regular, design: .rounded))
                .foregroundStyle(theme.swiftUIColor(.textPrimary))
                .lineLimit(4)
            Text(executor)
                .font(.system(size: 10, weight: .semibold, design: .monospaced))
                .foregroundStyle(theme.swiftUIColor(.textSecondary))
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}
