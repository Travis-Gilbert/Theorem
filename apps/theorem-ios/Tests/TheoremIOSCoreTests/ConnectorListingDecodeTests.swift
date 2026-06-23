import XCTest

@testable import TheoremIOSCore

/// Verifies the Swift client decodes theorem-harness-server's connector listing
/// contract (`GET /connectors` -> `connectors_json`: `{ tenant, connectors: [...],
/// affordances: [...], count }`) and groups it into the `Connector` rows the
/// Connectors surface renders. Pure decode + map; byte-parity against a live
/// server is a follow-up once the transport ships in production.
final class ConnectorListingDecodeTests: XCTestCase {
    private let payload = """
    {
      "tenant": "default",
      "count": 3,
      "connectors": ["everything", "filesystem"],
      "affordances": [
        { "affordance_id": "everything.add", "server_id": "everything",
          "tool_name": "add", "label": "Add", "description": "Add two numbers",
          "writeback_policy": "read-only", "fitness": 0.5 },
        { "affordance_id": "everything.echo", "server_id": "everything",
          "tool_name": "echo", "label": "", "description": "Echo input",
          "writeback_policy": "", "fitness": 0.5 },
        { "affordance_id": "filesystem.delete", "server_id": "filesystem",
          "tool_name": "delete", "label": "Delete", "description": "Remove a file",
          "writeback_policy": "destructive", "fitness": 0.5 }
      ]
    }
    """

    private func decodeAndMap(_ json: String) throws -> ConnectorListing {
        let data = Data(json.utf8)
        let response = try JSONDecoder().decode(ConnectorListResponse.self, from: data)
        return ConnectorListingMapper.map(response)
    }

    func test_decodes_and_groups_by_server() throws {
        let listing = try decodeAndMap(payload)
        XCTAssertEqual(listing.tenant, "default")
        XCTAssertEqual(listing.connectors.count, 2)
        XCTAssertEqual(listing.toolCount, 3)
        // Ordered by the server's distinct `connectors` list.
        XCTAssertEqual(listing.connectors.map(\.serverID), ["everything", "filesystem"])
        let everything = listing.connectors.first { $0.serverID == "everything" }
        XCTAssertEqual(everything?.toolCount, 2)
        // Tools sorted by tool_name within a server: add before echo.
        XCTAssertEqual(everything?.tools.map(\.toolName), ["add", "echo"])
    }

    func test_unannotated_tool_falls_back_to_unknown_policy_and_toolname_label() throws {
        let listing = try decodeAndMap(payload)
        let echo = listing.connectors.flatMap(\.tools).first { $0.toolName == "echo" }
        // Empty label -> tool_name; empty writeback_policy -> "unknown" (honest
        // about an undeclared profile, not assumed read-only).
        XCTAssertEqual(echo?.label, "echo")
        XCTAssertEqual(echo?.writebackPolicy, "unknown")
        // An annotated tool keeps its declared policy.
        let delete = listing.connectors.flatMap(\.tools).first { $0.toolName == "delete" }
        XCTAssertEqual(delete?.writebackPolicy, "destructive")
    }

    func test_empty_listing_decodes_to_empty() throws {
        let empty = """
        { "tenant": "default", "count": 0, "connectors": [], "affordances": [] }
        """
        let listing = try decodeAndMap(empty)
        XCTAssertTrue(listing.isEmpty)
        XCTAssertEqual(listing.toolCount, 0)
    }

    func test_sample_store_is_honestly_empty() async throws {
        let listing = try await SampleConnectorStore().listing()
        XCTAssertTrue(
            listing.isEmpty,
            "the offline default must be an honest empty listing, not fabricated connectors"
        )
    }
}
