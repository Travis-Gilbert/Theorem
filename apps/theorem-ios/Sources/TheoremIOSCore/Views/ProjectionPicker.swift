import SwiftUI

struct ProjectionPicker: View {
    @Binding var selection: ProjectionID
    var availability: [ProjectionAvailability]
    var theme: TheoremTheme

    var body: some View {
        HStack(spacing: 8) {
            ForEach(ProjectionID.allCases) { projection in
                let item = availability.first { $0.id == projection }
                Button {
                    selection = projection
                } label: {
                    Label(projection.title, systemImage: symbol(for: projection))
                        .labelStyle(.titleAndIcon)
                        .font(TheoremFonts.mono(size: 11))
                        .lineLimit(1)
                        .frame(maxWidth: .infinity)
                        .frame(height: 40)
                }
                .buttonStyle(.plain)
                .foregroundStyle(foreground(for: projection, available: item?.available == true))
                .background(background(for: projection, available: item?.available == true))
                .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                .disabled(item?.available != true)
                .help(item?.reason ?? "")
            }
        }
        .padding(4)
        .background(theme.surface.opacity(0.78), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }

    private func foreground(for projection: ProjectionID, available: Bool) -> Color {
        if !available {
            return theme.nodeDimmed
        }
        return selection == projection ? theme.surface : theme.textPrimary
    }

    private func background(for projection: ProjectionID, available: Bool) -> Color {
        if !available {
            return Color.clear
        }
        return selection == projection ? theme.textPrimary : theme.background.opacity(0.45)
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
