import SwiftUI

struct SurfaceRail: View {
    @Binding var selection: AppSurface
    var theme: TheoremTheme

    var body: some View {
        HStack(spacing: 6) {
            ForEach(AppSurface.allCases) { surface in
                Button {
                    selection = surface
                } label: {
                    Image(systemName: surface.symbolName)
                        .font(.system(size: 16, weight: .semibold))
                        .frame(maxWidth: .infinity)
                        .frame(height: 44)
                }
                .buttonStyle(.plain)
                .foregroundStyle(selection == surface ? theme.surface : theme.textPrimary)
                .background(selection == surface ? theme.textPrimary : Color.clear)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .help(surface.rawValue)
            }
        }
        .padding(5)
        .background(theme.surface.opacity(0.86), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .shadow(color: .black.opacity(0.10), radius: 24, x: 0, y: 12)
    }
}
