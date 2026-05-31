import Foundation

/// Talks to the hosted Theorem API (the deployed Index-API) and turns a query
/// into a real `ScenePackageV2` the graph renders. This is the search-box → live
/// scene seam (spec build-step 3): heavy retrieval runs server-side; the phone
/// receives the neighbourhood and draws it.
///
/// Ported from Codex's `apps/ios` search client, adapted to this app's models.
/// v1 wires the GRAPH path (native-search → scene). The 31B answer
/// (`/console/chat`) is the Ask flow and lands when there is a panel to show it.
public struct TheoremSearchClient: Sendable {
    public typealias DataLoader = @Sendable (URLRequest) async throws -> (Data, URLResponse)

    public static let productionBaseURL = URL(string: "https://index-api-production-a5f7.up.railway.app")!

    public let baseURL: URL
    private let dataLoader: DataLoader

    public init(
        baseURL: URL = TheoremSearchClient.productionBaseURL,
        dataLoader: @escaping DataLoader = { try await URLSession.shared.data(for: $0) }
    ) {
        self.baseURL = baseURL
        self.dataLoader = dataLoader
    }

    /// Run a substrate search and return a scene the renderer can draw. Throws
    /// `TheoremSearchError` on a bad query, HTTP failure, or empty result (so the
    /// UI shows an honest error / empty state, never a fabricated graph).
    public func search(query: String, topK: Int = 12) async throws -> ScenePackageV2 {
        let clean = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { throw TheoremSearchError.emptyQuery }

        let native = try await post(
            "/api/v2/theseus/native-search/",
            body: NativeSearchRequest(query: clean, topK: topK),
            response: TheoremNativeSearchResponse.self
        )
        if let message = native.error, !message.isEmpty {
            throw TheoremSearchError.server(message)
        }
        let scene = Self.scene(from: native, query: clean)
        guard !scene.atoms.isEmpty else { throw TheoremSearchError.noResults(clean) }
        return scene
    }

    // MARK: native response -> ScenePackageV2

    static func scene(from native: TheoremNativeSearchResponse, query: String) -> ScenePackageV2 {
        var seen = Set<String>()
        var atoms: [SceneAtom] = []

        // Ranked results are the direct matches (ring 0).
        for (index, result) in (native.rankedResults ?? []).enumerated() {
            let nodeID = nodeID(result.id, result.url, fallback: "ranked-\(index + 1)")
            guard seen.insert(nodeID).inserted else { continue }
            // The real URL lives in metadata when the top-level field is empty.
            let url = nonEmpty(result.url) ?? result.metadata?["url"]?.stringValue ?? ""
            let title = displayTitle(nonEmpty(result.title) ?? result.metadata?["title"]?.stringValue, url: url)
            let snippet = nonEmpty(result.snippet) ?? result.metadata?["snippet"]?.stringValue ?? ""
            atoms.append(atom(
                id: nodeID, url: url, title: title,
                snippet: snippet, ring: 0, ringLabel: "match",
                score: result.score ?? 0))
        }

        // Graph neighbours are one hop out (ring 1).
        for (index, node) in (native.graphNodes ?? []).enumerated() {
            let nodeID = nodeID(node.id, node.properties?["url"]?.stringValue, fallback: "graph-\(index + 1)")
            guard seen.insert(nodeID).inserted else { continue }
            let url = node.properties?["url"]?.stringValue ?? ""
            let title = displayTitle(nonEmpty(node.title) ?? nonEmpty(node.label), url: url)
            atoms.append(atom(
                id: nodeID, url: url, title: title,
                snippet: node.properties?["snippet"]?.stringValue ?? "", ring: 1, ringLabel: "adjacent",
                score: node.properties?["score"]?.doubleValue ?? 0))
        }

        let ids = Set(atoms.map(\.id))
        let relations = (native.graphEdges ?? []).compactMap { edge -> SceneRelation? in
            let source = edge.source ?? edge.fromID ?? ""
            let target = edge.target ?? edge.toID ?? ""
            guard ids.contains(source), ids.contains(target), source != target else { return nil }
            return SceneRelation(
                id: "\(source)->\(target)", sourceId: source, targetId: target,
                kind: edge.kind ?? edge.edgeType ?? "links_to")
        }

        let actions = atoms.compactMap { atom -> ActionDescriptor? in
            guard let url = atom.metadata["url"]?.stringValue, !url.isEmpty else { return nil }
            return ActionDescriptor(
                id: "open-\(atom.id)", label: "Open", actionType: "open-url",
                interaction: "tap", target: atom.id, payload: ["url": .string(url)])
        }

        return ScenePackageV2(
            id: "theorem-search-\(UUID().uuidString)",
            manifestRef: native.searchSessionID ?? "theorem-search",
            atoms: atoms,
            relations: relations,
            projection: ProjectionBinding(id: ProjectionID.forceGraph.rawValue),
            chrome: ChromeBinding(id: "search_scene"),
            actions: actions,
            provenance: [
                "source": .string("theorem-native-search"),
                "query": .string(query),
                "matched_count": .double(Double(native.rankedResults?.count ?? 0)),
                "kept_count": .double(Double(atoms.count)),
            ])
    }

    private static func atom(
        id: String, url: String, title: String, snippet: String,
        ring: Int, ringLabel: String, score: Double
    ) -> SceneAtom {
        SceneAtom(
            id: id, kind: "source",
            label: title.isEmpty ? (url.isEmpty ? id : url) : title,
            weight: max(score, 0.05), lifecycle: "present",
            metadata: [
                "url": .string(url),
                "snippet": .string(snippet),
                "ring": .double(Double(ring)),
                "ring_label": .string(ringLabel),
                "match_score": .double(score),
            ],
            sourceRefs: [SourceRef(kind: "WebDoc", id: id, label: title, metadata: ["url": .string(url)])])
    }

    private static func nodeID(_ id: String?, _ url: String?, fallback: String) -> String {
        nonEmpty(id) ?? nonEmpty(url) ?? fallback
    }

    /// Trim and treat empty strings as absent (the backend returns "" for an
    /// unfetched field, not null, so `??` alone would keep the empty string).
    private static func nonEmpty(_ value: String?) -> String? {
        guard let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              !trimmed.isEmpty else { return nil }
        return trimmed
    }

    /// A readable node label: a real title when present, else derived from the
    /// URL (last path segment, `_`/`-` -> space), else the host, else the URL.
    private static func displayTitle(_ title: String?, url: String) -> String {
        if let title, !title.lowercased().hasPrefix("http") { return title }
        if let parsed = URL(string: url) {
            if let segment = parsed.pathComponents.last(where: { $0 != "/" && !$0.isEmpty }) {
                let cleaned = segment
                    .replacingOccurrences(of: "_", with: " ")
                    .replacingOccurrences(of: "-", with: " ")
                    .removingPercentEncoding ?? segment
                if !cleaned.isEmpty { return cleaned }
            }
            if let host = parsed.host { return host }
        }
        return nonEmpty(title) ?? (url.isEmpty ? "Result" : url)
    }

    // MARK: HTTP

    private func post<Body: Encodable, Response: Decodable>(
        _ path: String, body: Body, response: Response.Type
    ) async throws -> Response {
        guard let url = URL(string: path, relativeTo: baseURL)?.absoluteURL else {
            throw TheoremSearchError.invalidEndpoint(path)
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = try JSONEncoder().encode(body)

        let (data, urlResponse) = try await dataLoader(request)
        if let http = urlResponse as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            throw TheoremSearchError.httpStatus(http.statusCode)
        }
        return try JSONDecoder().decode(Response.self, from: data)
    }
}

public enum TheoremSearchError: Error, Sendable, Equatable {
    case emptyQuery
    case invalidEndpoint(String)
    case httpStatus(Int)
    case server(String)
    case noResults(String)

    public var message: String {
        switch self {
        case .emptyQuery: return "Type something to search."
        case .invalidEndpoint: return "Bad search endpoint."
        case .httpStatus(let code): return "Search failed (HTTP \(code))."
        case .server(let detail): return detail
        case .noResults(let query): return "No results for \u{201C}\(query)\u{201D}."
        }
    }
}

// MARK: - Wire types (native search response)

struct NativeSearchRequest: Encodable {
    let query: String
    let topK: Int
    let includeGraph = true
    let includeReasoning = true
    let mode = "deep"
    let maxPages = 8
    let maxDepth = 1

    enum CodingKeys: String, CodingKey {
        case query, mode
        case topK = "top_k"
        case includeGraph = "include_graph"
        case includeReasoning = "include_reasoning"
        case maxPages = "max_pages"
        case maxDepth = "max_depth"
    }
}

public struct TheoremNativeSearchResponse: Codable, Hashable, Sendable {
    let query: String?
    let rankedResults: [TheoremRankedResult]?
    let graphNodes: [TheoremGraphNode]?
    let graphEdges: [TheoremGraphEdge]?
    let searchSessionID: String?
    let error: String?

    enum CodingKeys: String, CodingKey {
        case query, error
        case rankedResults = "ranked_results"
        case graphNodes = "graph_nodes"
        case graphEdges = "graph_edges"
        case searchSessionID = "search_session_id"
    }
}

struct TheoremRankedResult: Codable, Hashable, Sendable {
    let id: String?
    let title: String?
    let url: String?
    let snippet: String?
    let score: Double?
    let metadata: [String: JSONValue]?
}

struct TheoremGraphNode: Codable, Hashable, Sendable {
    let id: String?
    let label: String?
    let title: String?
    let properties: [String: JSONValue]?
}

struct TheoremGraphEdge: Codable, Hashable, Sendable {
    let source: String?
    let target: String?
    let fromID: String?
    let toID: String?
    let kind: String?
    let edgeType: String?

    enum CodingKeys: String, CodingKey {
        case source, target, kind
        case fromID = "from_id"
        case toID = "to_id"
        case edgeType = "edge_type"
    }
}
