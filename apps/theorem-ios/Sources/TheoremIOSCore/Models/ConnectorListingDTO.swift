import Foundation

/// Decoder for the connector listing contract
/// (`GET /connectors?tenant={slug}` -> theorem-harness-server's `connectors_json`,
/// shaped as `{ "tenant", "connectors": [server_id], "affordances": [...], "count" }`).
///
/// Each affordance is one registered MCP tool, modeled as an `Affordance` node in
/// the RustyRed substrate (docs/plans/mcp-learning-layer/). The `connectors` array
/// is the distinct owning-server set; the view groups affordances by their server
/// into the `Connector` rows it renders. Read-only: this endpoint contacts no MCP
/// server, it just lists what the substrate already learned.

public struct ConnectorListResponse: Decodable, Sendable {
    public let tenant: String
    public let connectors: [String]
    public let affordances: [AffordanceDTO]
    public let count: Int
}

/// One registered tool. `affordance_id`/`server_id`/`tool_name` are always set on
/// an affordance node; the rest are optional so a sparsely-populated node decodes
/// rather than failing the whole listing.
public struct AffordanceDTO: Decodable, Sendable {
    public let affordanceID: String
    public let serverID: String
    public let toolName: String
    public let label: String?
    public let description: String?
    public let writebackPolicy: String?
    public let fitness: Double?

    enum CodingKeys: String, CodingKey {
        case affordanceID = "affordance_id"
        case serverID = "server_id"
        case toolName = "tool_name"
        case label
        case description
        case writebackPolicy = "writeback_policy"
        case fitness
    }
}

/// A registered connector (one MCP server) and the tool affordances it offers.
public struct Connector: Identifiable, Sendable {
    public let serverID: String
    public let tools: [ConnectorTool]

    public init(serverID: String, tools: [ConnectorTool]) {
        self.serverID = serverID
        self.tools = tools
    }

    public var id: String { serverID }
    public var toolCount: Int { tools.count }
}

/// One tool affordance, with its declared side-effect profile and learned fitness.
public struct ConnectorTool: Identifiable, Sendable {
    public let affordanceID: String
    public let serverID: String
    public let toolName: String
    public let label: String
    public let detail: String
    /// "read-only" / "write" / "destructive" / "unknown", from the MCP
    /// readOnlyHint/destructiveHint extracted at registration. "unknown" is honest
    /// about an undeclared profile, not an assumption of safety.
    public let writebackPolicy: String
    public let fitness: Double?

    public init(
        affordanceID: String,
        serverID: String,
        toolName: String,
        label: String,
        detail: String,
        writebackPolicy: String,
        fitness: Double?
    ) {
        self.affordanceID = affordanceID
        self.serverID = serverID
        self.toolName = toolName
        self.label = label
        self.detail = detail
        self.writebackPolicy = writebackPolicy
        self.fitness = fitness
    }

    public var id: String { affordanceID }
}

/// The grouped listing the Connectors surface renders.
public struct ConnectorListing: Sendable {
    public let tenant: String
    public let connectors: [Connector]

    public init(tenant: String, connectors: [Connector]) {
        self.tenant = tenant
        self.connectors = connectors
    }

    public var isEmpty: Bool { connectors.isEmpty }
    public var toolCount: Int { connectors.reduce(0) { $0 + $1.tools.count } }
}

/// Group the flat affordance list into connectors, ordered by the server list the
/// server returned (which is distinct + sorted). Pure: unit-testable without a
/// live server. A server present only in `affordances` (not in `connectors`) is
/// appended defensively, so the listing never drops a real tool.
public enum ConnectorListingMapper {
    public static func map(_ response: ConnectorListResponse) -> ConnectorListing {
        var byServer: [String: [ConnectorTool]] = [:]
        for dto in response.affordances {
            let label = (dto.label?.isEmpty == false) ? dto.label! : dto.toolName
            let policy = (dto.writebackPolicy?.isEmpty == false) ? dto.writebackPolicy! : "unknown"
            let tool = ConnectorTool(
                affordanceID: dto.affordanceID,
                serverID: dto.serverID,
                toolName: dto.toolName,
                label: label,
                detail: dto.description ?? "",
                writebackPolicy: policy,
                fitness: dto.fitness
            )
            byServer[dto.serverID, default: []].append(tool)
        }

        var orderedServers = response.connectors
        for dto in response.affordances where !orderedServers.contains(dto.serverID) {
            orderedServers.append(dto.serverID)
        }

        let connectors = orderedServers.compactMap { server -> Connector? in
            guard let tools = byServer[server], !tools.isEmpty else { return nil }
            return Connector(serverID: server, tools: tools.sorted { $0.toolName < $1.toolName })
        }
        return ConnectorListing(tenant: response.tenant, connectors: connectors)
    }
}
