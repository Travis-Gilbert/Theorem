import SwiftUI

/// The Connectors surface: the registered MCP servers and the tool affordances
/// the substrate has learned to reach for. Reads through a `ConnectorStore`, so
/// the same surface shows an honest empty state offline and the live registry
/// when pointed at a runtime (`-remote`), with no view change.
///
/// Honest, not decorative: with no connectors registered the surface says so and
/// explains how to register one, rather than inventing tools. A transport failure
/// surfaces the error rather than falling back to a faked-populated list.
struct ConnectorsView: View {
    var theme: TheoremTheme
    /// The data source. Defaults to an honest empty listing; swap for a
    /// runtime-backed store (`RemoteConnectorStore`) to render the live registry.
    var store: ConnectorStore = SampleConnectorStore()

    @State private var listing: ConnectorListing?
    @State private var state: LoadState = .loading

    enum LoadState: Equatable {
        case loading
        case loaded
        case failed(String)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header
                content
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(theme.field.ignoresSafeArea())
        .task { await load() }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("CONNECTORS")
                .font(TheoremFonts.label(size: 10)).tracking(0.9)
                .foregroundStyle(theme.textMuted)
            Text("Connected tools")
                .font(TheoremFonts.display(size: 28, relativeTo: .title))
                .foregroundStyle(theme.ink)
            Text("MCP servers registered into the substrate. Each tool becomes a learnable affordance the router can reach for; an outcome that works raises that tool's fitness, so selection improves with use.")
                .font(TheoremFonts.body(size: 14)).foregroundStyle(theme.textSecondary)
                .lineSpacing(3)
        }
    }

    @ViewBuilder private var content: some View {
        switch state {
        case .loading:
            HStack(spacing: 9) {
                ProgressView().controlSize(.small).tint(theme.signal)
                Text("Reading the connector registry\u{2026}")
                    .font(TheoremFonts.body(size: 13)).foregroundStyle(theme.textMuted)
            }
            .padding(.top, 4)
        case .failed(let message):
            honest(message)
        case .loaded:
            if let listing, !listing.isEmpty {
                statusBanner(listing)
                VStack(spacing: 12) {
                    ForEach(listing.connectors) { connector in
                        connectorCard(connector)
                    }
                }
            } else {
                emptyState
            }
        }
    }

    private func honest(_ message: String) -> some View {
        Text(message)
            .font(TheoremFonts.body(size: 13)).foregroundStyle(theme.textMuted)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 4)
    }

    /// Honest empty state: no connectors are registered, with the real way to add
    /// one. Not a fake populated list (No-Fake-UI).
    private var emptyState: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 9) {
                Image(systemName: "powerplug")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(theme.textMuted)
                Text("No connectors registered yet.")
                    .font(TheoremFonts.body(size: 13).weight(.medium))
                    .foregroundStyle(theme.ink)
            }
            Text("Register an MCP server with the harness (POST /connectors/register) and its tools appear here as learnable affordances.")
                .font(TheoremFonts.body(size: 12)).foregroundStyle(theme.textMuted)
                .lineSpacing(2)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(13)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.chrome.opacity(0.5), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.hairline, lineWidth: 1)
        )
    }

    private func statusBanner(_ listing: ConnectorListing) -> some View {
        let connectorCount = listing.connectors.count
        let toolCount = listing.toolCount
        return HStack(spacing: 9) {
            Image(systemName: "dot.radiowaves.left.and.right")
                .font(.system(size: 13, weight: .semibold)).foregroundStyle(theme.signal)
            Text("\(connectorCount) connector\(connectorCount == 1 ? "" : "s"), \(toolCount) tool\(toolCount == 1 ? "" : "s") registered.")
                .font(TheoremFonts.body(size: 12)).foregroundStyle(theme.textMuted)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.chrome.opacity(0.5), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    private func connectorCard(_ connector: Connector) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 9) {
                Image(systemName: "powerplug")
                    .font(.system(size: 12, weight: .semibold)).foregroundStyle(theme.signal)
                Text(connector.serverID)
                    .font(TheoremFonts.body(size: 15).weight(.medium)).foregroundStyle(theme.ink)
                Spacer(minLength: 8)
                tag("\(connector.toolCount) tool\(connector.toolCount == 1 ? "" : "s")")
            }
            VStack(spacing: 0) {
                ForEach(connector.tools) { tool in
                    toolRow(tool)
                }
            }
        }
        .padding(13)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.chrome.opacity(0.5), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.hairline, lineWidth: 1)
        )
    }

    private func toolRow(_ tool: ConnectorTool) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 8) {
                Text(tool.label)
                    .font(TheoremFonts.body(size: 13).weight(.medium)).foregroundStyle(theme.ink)
                Spacer(minLength: 8)
                writebackTag(tool.writebackPolicy)
            }
            if !tool.detail.isEmpty {
                Text(tool.detail)
                    .font(TheoremFonts.body(size: 11)).foregroundStyle(theme.textMuted)
                    .lineSpacing(2).lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func tag(_ text: String) -> some View {
        Text(text)
            .font(TheoremFonts.label(size: 9)).foregroundStyle(theme.textMuted)
            .padding(.horizontal, 7).padding(.vertical, 3)
            .background(theme.chrome, in: Capsule())
    }

    /// The tool's declared side-effect profile (from MCP readOnlyHint /
    /// destructiveHint, extracted at registration). "unknown" when the server
    /// declared nothing: honest about not knowing, not assumed safe.
    private func writebackTag(_ policy: String) -> some View {
        Text(policy)
            .font(TheoremFonts.label(size: 9))
            .foregroundStyle(writebackColor(policy))
            .padding(.horizontal, 7).padding(.vertical, 3)
            .background(theme.chrome, in: Capsule())
    }

    private func writebackColor(_ policy: String) -> Color {
        switch policy {
        case "read-only": theme.signal
        case "destructive": theme.ringMatch
        case "write": theme.ruleStrong
        default: theme.textMuted
        }
    }

    @MainActor
    private func load() async {
        state = .loading
        do {
            listing = try await store.listing()
            state = .loaded
        } catch {
            let message = (error as? HarnessRunStoreError)?.message ?? "Couldn't read connectors."
            state = .failed(message)
        }
    }
}

#Preview {
    ConnectorsView(theme: .defaultPalette)
}
