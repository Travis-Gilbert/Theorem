import Foundation

/// The source of registered connectors for the Connectors surface. The UI reads
/// through this protocol so the data source is swappable: an honest empty listing
/// offline, the harness server's live connector registry when pointed at a
/// runtime (`-remote`), with no view change. Mirrors `ParticipantStore` and
/// `HarnessRunStore`.
public protocol ConnectorStore: Sendable {
    func listing() async throws -> ConnectorListing
}

/// The default source: an honest empty listing. No connectors exist until an
/// operator registers an MCP server (POST /connectors/register on
/// theorem-harness-server), so the surface shows an empty state rather than
/// fabricating connectors. Swap in `RemoteConnectorStore` to render the live
/// registry; the view does not change.
public struct SampleConnectorStore: ConnectorStore {
    public init() {}

    public func listing() async throws -> ConnectorListing {
        ConnectorListing(tenant: "default", connectors: [])
    }
}
