import SwiftUI

public struct TheoremRootView: View {
    @State private var surface: AppSurface = TheoremRootView.launchSurface
    @State private var projection: ProjectionID = TheoremRootView.launchProjection
    @State private var islandMode: DynamicIslandMode = TheoremRootView.launchIslandMode
    @State private var query: String = ""
    @State private var selectedNodeID: String?

    @State private var room: CommonplaceRoom = SampleRoom.room
    @State private var isSearching = false
    @State private var searchError: String?
    @State private var patentFocus: String?

    private let theme = TheoremTheme.defaultPalette
    private let searchClient = TheoremSearchClient()

    /// The run data source. `-remote <url>` points the Runs surface at a live
    /// harness HTTP server (theorem-harness-server); default is the recorded sample.
    private var runStore: HarnessRunStore {
        if let raw = UserDefaults.standard.string(forKey: "remote"),
           let url = URL(string: raw) {
            return RemoteHarnessRunStore(baseURL: url)
        }
        return SampleRunStore()
    }

    /// The participant data source. The same `-remote <url>` that drives the Runs
    /// surface points Participants at the runtime's presence feed; `-tenant` and
    /// `-room` override the coordination scope (both default to "default").
    /// Default is the recorded roster with idle status.
    private var participantStore: ParticipantStore {
        if let raw = UserDefaults.standard.string(forKey: "remote"),
           let url = URL(string: raw) {
            let tenant = UserDefaults.standard.string(forKey: "tenant") ?? "default"
            let room = UserDefaults.standard.string(forKey: "room") ?? "default"
            return RemoteParticipantStore(baseURL: url, roomID: room, tenantSlug: tenant)
        }
        return SampleParticipantStore()
    }

    /// The connector data source. The same `-remote <url>` that drives Runs and
    /// Participants points the Connectors surface at theorem-harness-server's
    /// connector registry; `-tenant` overrides the scope. Default is an honest
    /// empty listing (no connectors until one is registered).
    private var connectorStore: ConnectorStore {
        if let raw = UserDefaults.standard.string(forKey: "remote"),
           let url = URL(string: raw) {
            let tenant = UserDefaults.standard.string(forKey: "tenant") ?? "default"
            return RemoteConnectorStore(baseURL: url, tenantSlug: tenant)
        }
        return SampleConnectorStore()
    }

    private var package: ScenePackageV2 {
        room.scene
    }

    public init() {}

    private var projectionAvailability: [ProjectionAvailability] {
        TheoremProjectionEngine.availableProjections(for: package)
    }

    /// Run a live substrate search and load the resulting scene into the graph.
    /// On failure, surface the error (honest) rather than a fabricated scene.
    @MainActor
    private func runSearch() async {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !isSearching else { return }
        isSearching = true
        searchError = nil
        do {
            let scene = try await searchClient.search(query: trimmed)
            room = room.replacingScene(scene, ask: trimmed)
            projection = .forceGraph
            selectedNodeID = nil
            islandMode = .idle
        } catch {
            searchError = (error as? TheoremSearchError)?.message ?? "Search failed."
        }
        isSearching = false
    }

    /// Optional launch-argument overrides for the initial projection / surface
    /// (deep-link + screenshot support), e.g.
    /// `simctl launch ... -projection radial_rings -surface Projects`.
    /// Default to Force / Home.
    private static var launchProjection: ProjectionID {
        if let raw = UserDefaults.standard.string(forKey: "projection"),
           let projection = ProjectionID(rawValue: raw) {
            return projection
        }
        return .forceGraph
    }

    private static var launchSurface: AppSurface {
        if let raw = UserDefaults.standard.string(forKey: "surface"),
           let surface = AppSurface(rawValue: raw) {
            return surface
        }
        return .home
    }

    /// `-islandMode search` opens the island already expanded (deep-link +
    /// screenshot capture of the on-search algorithm switcher). Defaults to idle.
    private static var launchIslandMode: DynamicIslandMode {
        if let raw = UserDefaults.standard.string(forKey: "islandMode"),
           let mode = DynamicIslandMode(rawValue: raw) {
            return mode
        }
        return .idle
    }

    public var body: some View {
        ZStack(alignment: .bottom) {
            theme.background.ignoresSafeArea()

            TabView(selection: $surface) {
                // The graph is the canvas; ALL chrome (room, dossier, search)
                // lives in the Dynamic Island, which changes shape and context.
                // The room is no longer a slab below the graph.
                TheoremSceneView(
                    package: room.scene,
                    projection: projection,
                    selectedNodeID: $selectedNodeID,
                    theme: theme
                )
                .tag(AppSurface.home)

                RunsListView(theme: theme, store: runStore)
                    .tag(AppSurface.runs)

                MapsView(theme: theme)
                    .tag(AppSurface.maps)

                SurfacePlaceholder(surface: .projects, theme: theme)
                    .tag(AppSurface.projects)

                ParticipantPresenceView(theme: theme, store: participantStore)
                    .tag(AppSurface.models)

                UserCredentialsView(theme: theme)
                    .tag(AppSurface.credentials)

                ConnectorsView(theme: theme, store: connectorStore)
                    .tag(AppSurface.connectors)

                SurfacePlaceholder(surface: .build, theme: theme)
                    .tag(AppSurface.build)

                SurfacePlaceholder(surface: .artifacts, theme: theme)
                    .tag(AppSurface.artifacts)
            }
            .theoremPagedTabStyle()
            .safeAreaInset(edge: .top) {
                // Surfaces are a distinct concern from projections (addendum D4):
                // they get their own affordance, not the search-control island.
                SurfaceRail(selection: $surface, theme: theme)
                    .padding(.horizontal, 16)
                    .padding(.top, 6)
            }

            DynamicIslandView(
                mode: $islandMode,
                query: $query,
                projection: $projection,
                centerTitle: centerTitle,
                projectionAvailability: projectionAvailability,
                theme: theme,
                focusedAtom: focusedAtom,
                searchClient: searchClient,
                room: room,
                onSubmitQuery: { Task { await runSearch() } },
                onSceneOS: { patentFocus = focusedAtom?.id ?? patentFocusID },
                onDeeperSearch: { label in
                    query = label
                    Task { await runSearch() }
                }
            )
            .padding(.horizontal, 16)
            .padding(.bottom, 12)

            if isSearching {
                Color.black.opacity(0.18).ignoresSafeArea()
                ProgressView("Searching the substrate…")
                    .tint(theme.textPrimary)
                    .font(TheoremFonts.mono(size: 12))
                    .foregroundStyle(theme.textPrimary)
                    .padding(22)
                    .background(theme.surface, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                    .shadow(color: .black.opacity(0.18), radius: 24, y: 12)
            }

            if let searchError {
                VStack(spacing: 0) {
                    Spacer().frame(height: 72)
                    Text(searchError)
                        .font(TheoremFonts.mono(size: 12))
                        .foregroundStyle(theme.surface)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 11)
                        .background(theme.ringMatch, in: Capsule())
                        .onTapGesture { self.searchError = nil }
                        .padding(.horizontal, 24)
                    Spacer()
                }
            }

            // Patent plate laid over the field (addendum D7), driven by real
            // graph data for the focused node. Topmost; its own close/back chrome.
            if let patentFocus {
                PatentPlateView(
                    package: package,
                    focusID: patentFocus,
                    theme: theme,
                    onClose: { withAnimation { self.patentFocus = nil } }
                )
                .transition(.opacity)
                .zIndex(10)
            }
        }
        .animation(.easeInOut(duration: 0.2), value: patentFocus)
        // Tapping a node morphs the island into that node's dossier (addendum D8):
        // the dossier IS the island expanding, never a slab below the graph.
        .onChange(of: selectedNodeID) { _, newValue in
            if newValue != nil { islandMode = .detail }
        }
        .task {
            await autoSearchIfRequested()
            // `-patent 1` opens the plate on the center node (deep-link + capture).
            if UserDefaults.standard.bool(forKey: "patent") {
                patentFocus = patentFocusID
            }
        }
    }

    /// Auto-run a search at launch when `-autoSearch <query>` is passed (deep-link
    /// + screenshot capture).
    @MainActor
    private func autoSearchIfRequested() async {
        guard let query = UserDefaults.standard.string(forKey: "autoSearch"),
              !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        self.query = query
        await runSearch()
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

    /// The node a patent plate opens on: the selected node, else the center node.
    private var patentFocusID: String? {
        if let selectedNodeID { return selectedNodeID }
        return TheoremProjectionEngine.centerNodeID(in: package, mode: .pprMass)
    }

    /// The node the island dossier opens on: the selected node, else the center.
    private var focusedAtom: SceneAtom? {
        if let id = patentFocusID {
            return package.atoms.first { $0.id == id }
        }
        return package.atoms.first
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
    case runs = "Runs"
    case maps = "Maps"
    case projects = "Projects"
    case models = "Participants"
    case credentials = "Credentials"
    case connectors = "Connectors"
    case build = "Build"
    case artifacts = "Artifacts"

    public var id: String { rawValue }

    var symbolName: String {
        switch self {
        case .home:
            "network"
        case .runs:
            "clock.arrow.circlepath"
        case .maps:
            "map"
        case .projects:
            "folder"
        case .models:
            "person.2"
        case .credentials:
            "key"
        case .connectors:
            "powerplug"
        case .build:
            "hammer"
        case .artifacts:
            "archivebox"
        }
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
            "Ask over the substrate room."
        case .runs:
            "Recorded runs, each a governed state machine."
        case .maps:
            "Orientation maps: where you are and what matters here."
        case .projects:
            "Scoped containers and file-glyph workspaces."
        case .models:
            "Team presence and brought-agent endpoints."
        case .credentials:
            "Provider endpoints and user-held keys."
        case .connectors:
            "Registered MCP servers and their learnable tools."
        case .build:
            "Agent, skill, and plugin scaffolds."
        case .artifacts:
            "Saved scenes, captures, and dossiers."
        }
    }
}
