import SwiftUI

public struct TheoremRootView: View {
    @State private var surface: AppSurface = .home
    @State private var projection: ProjectionID = .forceGraph
    @State private var islandMode: DynamicIslandMode = .idle
    @State private var query: String = ""
    @State private var selectedNodeID: String?

    private let package = SampleScene.package
    private let theme = TheoremTheme.defaultPalette

    public init() {}

    public var body: some View {
        ZStack(alignment: .top) {
            theme.background.ignoresSafeArea()

            TabView(selection: $surface) {
                HomeScenePage(
                    package: package,
                    projection: $projection,
                    selectedNodeID: $selectedNodeID,
                    theme: theme
                )
                .tag(AppSurface.home)

                SurfacePlaceholder(surface: .projects, theme: theme)
                    .tag(AppSurface.projects)

                SurfacePlaceholder(surface: .models, theme: theme)
                    .tag(AppSurface.models)

                SurfacePlaceholder(surface: .build, theme: theme)
                    .tag(AppSurface.build)

                SurfacePlaceholder(surface: .artifacts, theme: theme)
                    .tag(AppSurface.artifacts)
            }
            .theoremPagedTabStyle()

            VStack(spacing: 0) {
                DynamicIslandView(
                    mode: $islandMode,
                    query: $query,
                    centerTitle: centerTitle,
                    theme: theme
                )
                .padding(.top, 10)

                Spacer()

                SurfaceRail(selection: $surface, theme: theme)
                    .padding(.horizontal, 18)
                    .padding(.bottom, 12)
            }
        }
    }

    private var centerTitle: String {
        if let selectedNodeID,
           let selected = package.atoms.first(where: { $0.id == selectedNodeID }) {
            return selected.label ?? selected.id
        }
        if let center = TheoremProjectionEngine.centerNodeID(in: package, mode: .pprMass),
           let atom = package.atoms.first(where: { $0.id == center }) {
            return atom.label ?? atom.id
        }
        return "Theorem"
    }
}

private extension View {
    @ViewBuilder
    func theoremPagedTabStyle() -> some View {
        #if os(iOS)
        self.tabViewStyle(.page(indexDisplayMode: .never))
        #else
        self
        #endif
    }
}

public enum AppSurface: String, CaseIterable, Identifiable {
    case home = "Home"
    case projects = "Projects"
    case models = "Models"
    case build = "Build"
    case artifacts = "Artifacts"

    public var id: String { rawValue }

    var symbolName: String {
        switch self {
        case .home:
            "network"
        case .projects:
            "folder"
        case .models:
            "cpu"
        case .build:
            "hammer"
        case .artifacts:
            "archivebox"
        }
    }
}

struct HomeScenePage: View {
    var package: ScenePackageV2
    @Binding var projection: ProjectionID
    @Binding var selectedNodeID: String?
    var theme: TheoremTheme

    private var availability: [ProjectionAvailability] {
        TheoremProjectionEngine.availableProjections(for: package)
    }

    var body: some View {
        VStack(spacing: 18) {
            Spacer(minLength: 82)

            TheoremSceneView(
                package: package,
                projection: projection,
                selectedNodeID: $selectedNodeID,
                theme: theme
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            ProjectionPicker(
                selection: $projection,
                availability: availability,
                theme: theme
            )
            .padding(.horizontal, 18)
            .padding(.bottom, 72)
        }
        .padding(.horizontal, 14)
    }
}

struct SurfacePlaceholder: View {
    var surface: AppSurface
    var theme: TheoremTheme

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Spacer(minLength: 100)
            Image(systemName: surface.symbolName)
                .font(.system(size: 28, weight: .medium))
                .foregroundStyle(theme.nodeTool)
            Text(surface.rawValue)
                .font(TheoremFonts.display(size: 42, relativeTo: .largeTitle))
                .foregroundStyle(theme.textPrimary)
            Text(surfaceCopy)
                .font(TheoremFonts.body(size: 17))
                .foregroundStyle(theme.textSecondary)
                .lineSpacing(4)
            Spacer()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 28)
    }

    private var surfaceCopy: String {
        switch surface {
        case .home:
            "Graph-native search and reason."
        case .projects:
            "Scoped containers and file-glyph workspaces."
        case .models:
            "Model selection for hosted GL-fusion or user keys."
        case .build:
            "Agent, skill, and plugin scaffolds."
        case .artifacts:
            "Saved scenes, captures, and dossiers."
        }
    }
}
