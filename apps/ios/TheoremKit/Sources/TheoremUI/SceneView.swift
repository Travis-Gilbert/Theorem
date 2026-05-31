import SwiftUI
import TheoremKit

/// The scene surface: the graph hero. Dispatches to the renderer for the
/// selected projection and overlays the projection switcher. This is the Home
/// surface's centerpiece (the IA shell composes it under the Dynamic Island).
///
/// The caller owns the `SceneViewModel` (so the Dynamic Island can read the same
/// center node and the app can drive scene loads); `SceneView` only renders it.
public struct SceneView: View {
    @Bindable public var model: SceneViewModel
    public var theme: Theme

    public init(model: SceneViewModel, theme: Theme = .theorem) {
        self.model = model
        self.theme = theme
    }

    public var body: some View {
        ZStack {
            theme.swiftUIColor(.background).ignoresSafeArea()

            if model.scene != nil {
                renderer
                VStack {
                    Spacer()
                    ProjectionSwitcher(
                        projections: model.availableProjections,
                        selectedID: model.selectedProjectionID,
                        theme: theme,
                        onSelect: { model.select($0) }
                    )
                    .padding(.bottom, 28)
                }
            } else {
                emptyState
            }
        }
        .sensoryFeedback(.selection, trigger: model.centerNodeID)
    }

    @ViewBuilder
    private var renderer: some View {
        if let scene = model.scene {
            switch model.selectedProjectionID {
            case ProjectionID.forceGraph:
                ForceGraphView(scene: scene, theme: theme)

            case ProjectionID.fractalExpansion:
                FractalExpansionView(
                    scene: scene,
                    positions: model.layout?.positions ?? [:],
                    theme: theme,
                    trace: model.pushTrace,
                    selectedID: $model.selectedAtomID
                )

            default:
                if let layout = model.layout {
                    GraphCanvas(
                        scene: scene,
                        positions: layout.positions,
                        theme: theme,
                        selectedID: $model.selectedAtomID
                    )
                } else {
                    rejectionState
                }
            }
        }
    }

    /// Honest backstop: if a projection was selected that the data can't support
    /// (the switcher should have greyed it), say so plainly — never a blank or
    /// fabricated view.
    private var rejectionState: some View {
        VStack(spacing: 10) {
            Text(ProjectionID.label(model.selectedProjectionID))
                .font(.system(size: 15, weight: .semibold, design: .monospaced))
                .foregroundStyle(theme.swiftUIColor(.textPrimary))
            Text(model.rejectionReason ?? "This layout is not available for this scene.")
                .font(.system(size: 13, design: .monospaced))
                .foregroundStyle(theme.swiftUIColor(.textSecondary))
                .multilineTextAlignment(.center)
        }
        .padding(32)
    }

    private var emptyState: some View {
        VStack(spacing: 8) {
            Text("No scene")
                .font(.system(size: 15, weight: .semibold, design: .monospaced))
                .foregroundStyle(theme.swiftUIColor(.textPrimary))
            Text("Search to build one.")
                .font(.system(size: 13, design: .monospaced))
                .foregroundStyle(theme.swiftUIColor(.textSecondary))
        }
    }
}
