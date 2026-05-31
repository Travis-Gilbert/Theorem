import SwiftUI
import TheoremKit

/// The projection picker — the spec differentiator made visible. Lights exactly
/// the projections `available_projections` returned; greys the rest with a
/// one-line reason on tap/long-press ("links form a cycle — no tree"). The
/// greying teaches the user that the projections are honest to their data.
struct ProjectionSwitcher: View {
    let projections: [ProjectionAvailability]
    let selectedID: String
    let theme: Theme
    let onSelect: (String) -> Void

    @State private var reasonShown: ProjectionAvailability?

    var body: some View {
        HStack(spacing: 6) {
            ForEach(projections) { projection in
                pill(projection)
            }
        }
        .padding(6)
        .background(theme.swiftUIColor(.surface).opacity(0.92), in: Capsule())
        .overlay(Capsule().stroke(theme.swiftUIColor(.textSecondary).opacity(0.15), lineWidth: 1))
        .popover(item: $reasonShown) { item in
            Text(item.reason ?? "Unavailable for this scene")
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(theme.swiftUIColor(.textPrimary))
                .padding(12)
                .presentationCompactAdaptation(.popover)
        }
    }

    private func pill(_ projection: ProjectionAvailability) -> some View {
        let selected = projection.projectionID == selectedID
        return Text(projection.label)
            .font(.system(size: 12, weight: selected ? .semibold : .regular, design: .monospaced))
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .foregroundStyle(
                selected ? theme.swiftUIColor(.textPrimary)
                         : theme.swiftUIColor(projection.available ? .textPrimary : .textSecondary)
            )
            .background {
                if selected {
                    Capsule().fill(theme.swiftUIColor(.ringMatch))
                }
            }
            .opacity(projection.available ? 1 : 0.5)
            .contentShape(Capsule())
            .onTapGesture {
                if projection.available {
                    onSelect(projection.projectionID)
                } else {
                    reasonShown = projection
                }
            }
            .onLongPressGesture(minimumDuration: 0.3) {
                if !projection.available { reasonShown = projection }
            }
            .accessibilityLabel("\(projection.label) layout")
            .accessibilityHint(projection.available ? "Switch to this layout" : (projection.reason ?? "Unavailable"))
    }
}
