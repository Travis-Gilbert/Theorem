import SwiftUI

/// The Maps orientation surface (harness UI spec, Part 3). Renders a `MapArtifact`
/// as orientation, not a file tree: grouped by role (boundary, read first, avoid,
/// rules, verify, tools) so the answer to "where am I and what matters here" is
/// legible at a glance. The map is curated, not exhaustive: that is the visual
/// form of "orientation is not retrieval."
///
/// Today it renders the recorded codebase map (`SampleMap.theoremCodebase`); when
/// the map compiler ports to Rust and a transport ships, a live `MapArtifact`
/// renders through this same view.
struct MapsView: View {
    var theme: TheoremTheme

    private let map: MapArtifact = SampleMap.theoremCodebase

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                header
                ForEach(map.sections, id: \.role) { section in
                    sectionView(section.role, section.entries)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(theme.field.ignoresSafeArea())
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("MAP")
                .font(TheoremFonts.label(size: 10)).tracking(0.9)
                .foregroundStyle(theme.textMuted)
            Text(map.title)
                .font(TheoremFonts.display(size: 28, relativeTo: .title))
                .foregroundStyle(theme.ink)
            Text("Where you are and what matters here. An orientation, not a file tree.")
                .font(TheoremFonts.body(size: 14)).foregroundStyle(theme.textSecondary)
                .lineSpacing(3)
        }
    }

    private func sectionView(_ role: MapEntryRole, _ entries: [MapEntry]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(role.label)
                .font(TheoremFonts.label(size: 10)).tracking(0.9)
                .foregroundStyle(theme.textMuted)
            ForEach(entries) { entry in
                entryCard(entry)
            }
        }
    }

    private func entryCard(_ entry: MapEntry) -> some View {
        let muted = entry.role == .avoid
        return HStack(alignment: .top, spacing: 10) {
            Image(systemName: symbol(entry.role))
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(muted ? theme.textMuted : theme.ruleStrong)
                .frame(width: 16)
            VStack(alignment: .leading, spacing: 3) {
                Text(entry.title)
                    .font(TheoremFonts.body(size: 14).weight(.medium))
                    .foregroundStyle(muted ? theme.textMuted : theme.ink)
                Text(entry.summary)
                    .font(TheoremFonts.body(size: 12)).foregroundStyle(theme.textSecondary)
                    .lineSpacing(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(13)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.chrome.opacity(0.5), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.hairline, lineWidth: 1)
        )
    }

    private func symbol(_ role: MapEntryRole) -> String {
        switch role {
        case .structure: "square.dashed"
        case .readFirst: "arrow.right.circle.fill"
        case .avoid: "slash.circle"
        case .rules: "list.bullet.rectangle"
        case .verify: "checkmark.seal"
        case .tools: "wrench.and.screwdriver"
        case .note: "circle"
        }
    }
}

#Preview {
    MapsView(theme: .defaultPalette)
}
