import Foundation

/// A `ConnectorStore` backed by theorem-harness-server's connector listing
/// (`GET /connectors?tenant={slug}`). Lists the registered MCP connectors and
/// their tool affordances; this endpoint is read-only and contacts no MCP server.
/// Swapping this in for `SampleConnectorStore` makes the Connectors surface render
/// the live registry with no view change. Reuses `HarnessRunStoreError` so
/// transport/status/decoding failures surface the same way as the runs and
/// presence paths.
public struct RemoteConnectorStore: ConnectorStore {
    public let baseURL: URL
    public let tenantSlug: String
    private let session: URLSession

    public init(baseURL: URL, tenantSlug: String = "default", session: URLSession = .shared) {
        self.baseURL = baseURL
        self.tenantSlug = tenantSlug
        self.session = session
    }

    public func listing() async throws -> ConnectorListing {
        guard
            var components = URLComponents(
                url: baseURL.appendingPathComponent("connectors"),
                resolvingAgainstBaseURL: false
            )
        else {
            throw HarnessRunStoreError.transport("Bad connectors URL")
        }
        components.queryItems = [URLQueryItem(name: "tenant", value: tenantSlug)]
        guard let url = components.url else {
            throw HarnessRunStoreError.transport("Bad connectors URL")
        }

        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse else {
            throw HarnessRunStoreError.transport("No HTTP response")
        }
        guard (200..<300).contains(http.statusCode) else {
            throw HarnessRunStoreError.status(http.statusCode)
        }
        let decoded: ConnectorListResponse
        do {
            decoded = try JSONDecoder().decode(ConnectorListResponse.self, from: data)
        } catch {
            throw HarnessRunStoreError.decoding(String(describing: error))
        }
        return ConnectorListingMapper.map(decoded)
    }
}
