import Foundation

public struct TheoremSearchOptions: Sendable, Hashable {
    public var topK: Int
    public var maxPages: Int
    public var maxDepth: Int
    public var mode: String
    public var maxTokens: Int
    public var temperature: Double

    public init(
        topK: Int = 12,
        maxPages: Int = 8,
        maxDepth: Int = 1,
        mode: String = "deep",
        maxTokens: Int = 1200,
        temperature: Double = 0.2
    ) {
        self.topK = topK
        self.maxPages = maxPages
        self.maxDepth = maxDepth
        self.mode = mode
        self.maxTokens = maxTokens
        self.temperature = temperature
    }
}

public struct TheoremSearchTurn: Sendable, Hashable {
    public let query: String
    public let search: SubstrateSearch
    public let scene: ScenePackageV2
    public let answerText: String
    public let chosenExecutor: String
    public let searchSessionID: String

    public init(
        query: String,
        search: SubstrateSearch,
        scene: ScenePackageV2,
        answerText: String,
        chosenExecutor: String,
        searchSessionID: String
    ) {
        self.query = query
        self.search = search
        self.scene = scene
        self.answerText = answerText
        self.chosenExecutor = chosenExecutor
        self.searchSessionID = searchSessionID
    }
}

public enum TheoremSearchClientError: Error, Sendable, Equatable {
    case emptyQuery
    case invalidEndpoint(String)
    case httpStatus(Int, String)
    case missingAnswer
}

public struct TheoremNativeSearchResponse: Codable, Hashable, Sendable {
    public let query: String?
    public let rankedResults: [TheoremRankedResult]?
    public let graphNodes: [TheoremGraphNode]?
    public let graphEdges: [TheoremGraphEdge]?
    public let searchSessionID: String?
    public let metadata: [String: JSONValue]?
    public let error: String?

    enum CodingKeys: String, CodingKey {
        case query
        case rankedResults = "ranked_results"
        case graphNodes = "graph_nodes"
        case graphEdges = "graph_edges"
        case searchSessionID = "search_session_id"
        case metadata
        case error
    }
}

public struct TheoremRankedResult: Codable, Hashable, Sendable {
    public let id: String?
    public let title: String?
    public let url: String?
    public let snippet: String?
    public let score: Double?
    public let sourceType: String?
    public let metadata: [String: JSONValue]?

    enum CodingKeys: String, CodingKey {
        case id, title, url, snippet, score, metadata
        case sourceType = "source_type"
    }
}

public struct TheoremGraphNode: Codable, Hashable, Sendable {
    public let id: String?
    public let label: String?
    public let title: String?
    public let properties: [String: JSONValue]?
}

public struct TheoremGraphEdge: Codable, Hashable, Sendable {
    public let id: String?
    public let source: String?
    public let target: String?
    public let fromID: String?
    public let toID: String?
    public let kind: String?
    public let edgeType: String?
    public let weight: Double?
    public let properties: [String: JSONValue]?

    enum CodingKeys: String, CodingKey {
        case id, source, target, kind, weight, properties
        case fromID = "from_id"
        case toID = "to_id"
        case edgeType = "edge_type"
    }
}

public struct TheoremEvidenceSearchResponse: Codable, Hashable, Sendable {
    public let operation: String?
    public let searchSessionID: String?
    public let totalCount: Int?
    public let results: [TheoremEvidenceResult]?

    enum CodingKeys: String, CodingKey {
        case operation, results
        case searchSessionID = "search_session_id"
        case totalCount = "total_count"
    }
}

public struct TheoremEvidenceResult: Codable, Hashable, Sendable {
    public let objectID: String?
    public let webdocID: String?
    public let title: String?
    public let url: String?
    public let canonicalURL: String?
    public let snippet: String?
    public let markdown: String?
    public let rankScore: Double?
    public let sourceQuality: Double?

    enum CodingKeys: String, CodingKey {
        case title, url, snippet, markdown
        case objectID = "object_id"
        case webdocID = "webdoc_id"
        case canonicalURL = "canonical_url"
        case rankScore = "rank_score"
        case sourceQuality = "source_quality"
    }
}

private struct NativeSearchRequest: Encodable {
    let query: String
    let topK: Int
    let includeGraph: Bool
    let includeReasoning: Bool
    let allowBrowserActions: Bool
    let useBrowserActions: Bool
    let admitResults: Bool
    let mode: String
    let maxPages: Int
    let maxDepth: Int

    enum CodingKeys: String, CodingKey {
        case query, mode
        case topK = "top_k"
        case includeGraph = "include_graph"
        case includeReasoning = "include_reasoning"
        case allowBrowserActions = "allow_browser_actions"
        case useBrowserActions = "use_browser_actions"
        case admitResults = "admit_results"
        case maxPages = "max_pages"
        case maxDepth = "max_depth"
    }
}

private struct ConsoleChatRequest: Encodable {
    let prompt: String
    let queryID: String
    let maxTokens: Int
    let temperature: Double
    let inputRefs: [String]

    enum CodingKeys: String, CodingKey {
        case prompt, temperature
        case queryID = "query_id"
        case maxTokens = "max_tokens"
        case inputRefs = "input_refs"
    }
}

private struct EvidenceSearchRequest: Encodable {
    let query: String
    let topK: Int
    let includeGraphHits: Bool
    let includeWebHits: Bool
    let captureNewWebdocs: Bool

    enum CodingKeys: String, CodingKey {
        case query
        case topK = "top_k"
        case includeGraphHits = "include_graph_hits"
        case includeWebHits = "include_web_hits"
        case captureNewWebdocs = "capture_new_webdocs"
    }
}

private struct ConsoleChatResponse: Decodable {
    let text: String?
    let chosenExecutor: String?

    enum CodingKeys: String, CodingKey {
        case text
        case chosenExecutor = "chosen_executor"
    }
}

public struct TheoremSearchClient: Sendable {
    public typealias DataLoader = @Sendable (URLRequest) async throws -> (Data, URLResponse)

    public static let productionBaseURL = URL(string: "https://index-api-production-a5f7.up.railway.app")!
    public static let defaultDataLoader: DataLoader = { request in
        try await URLSession.shared.data(for: request)
    }

    public let baseURL: URL
    private let dataLoader: DataLoader

    public init(
        baseURL: URL = TheoremSearchClient.productionBaseURL,
        dataLoader: @escaping DataLoader = TheoremSearchClient.defaultDataLoader
    ) {
        self.baseURL = baseURL
        self.dataLoader = dataLoader
    }

    public func search(query: String, options: TheoremSearchOptions = TheoremSearchOptions()) async throws -> TheoremSearchTurn {
        let cleanQuery = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanQuery.isEmpty else { throw TheoremSearchClientError.emptyQuery }

        let native = try await post(
            "/api/v2/theseus/native-search/",
            body: NativeSearchRequest(
                query: cleanQuery,
                topK: options.topK,
                includeGraph: true,
                includeReasoning: true,
                allowBrowserActions: true,
                useBrowserActions: true,
                admitResults: true,
                mode: options.mode,
                maxPages: options.maxPages,
                maxDepth: options.maxDepth
            ),
            response: TheoremNativeSearchResponse.self
        )

        let nativeSearch = SubstrateSearch(nativeSearch: native, fallbackQuery: cleanQuery)
        let substrateSearch: SubstrateSearch
        let searchSessionID: String
        if native.error == nil && !nativeSearch.hits.isEmpty {
            substrateSearch = nativeSearch
            searchSessionID = native.searchSessionID ?? ""
        } else {
            let evidence = try await post(
                "/api/v2/theseus/search/search/",
                body: EvidenceSearchRequest(
                    query: cleanQuery,
                    topK: options.topK,
                    includeGraphHits: true,
                    includeWebHits: true,
                    captureNewWebdocs: true
                ),
                response: TheoremEvidenceSearchResponse.self
            )
            substrateSearch = SubstrateSearch(evidenceSearch: evidence, fallbackQuery: cleanQuery)
            searchSessionID = evidence.searchSessionID ?? native.searchSessionID ?? ""
        }
        let scene = substrateSearch.scenePackage(
            id: "theorem-search-\(UUID().uuidString)",
            manifestRef: searchSessionID.isEmpty ? "theorem-search" : searchSessionID
        )
        let console = try await post(
            "/api/v2/theseus/console/chat",
            body: ConsoleChatRequest(
                prompt: Self.build31BPrompt(query: cleanQuery, search: substrateSearch),
                queryID: searchSessionID,
                maxTokens: options.maxTokens,
                temperature: options.temperature,
                inputRefs: substrateSearch.hits.map(\.nodeID)
            ),
            response: ConsoleChatResponse.self
        )

        guard let text = console.text, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw TheoremSearchClientError.missingAnswer
        }

        return TheoremSearchTurn(
            query: cleanQuery,
            search: substrateSearch,
            scene: scene,
            answerText: text,
            chosenExecutor: console.chosenExecutor ?? "gl-fusion-31b",
            searchSessionID: searchSessionID
        )
    }

    public static func build31BPrompt(query: String, search: SubstrateSearch) -> String {
        let evidence = search.hits.prefix(8).enumerated().map { index, hit in
            let title = hit.title.isEmpty ? hit.url : hit.title
            let source = hit.url.isEmpty ? "Source: \(hit.nodeID)" : "URL: \(hit.url)"
            let score = String(format: "%.3f", hit.matchScore)
            let snippet = hit.snippet.isEmpty ? "Snippet: not returned" : "Snippet: \(hit.snippet)"
            return "[\(index + 1)] \(title)\n\(source)\nScore: \(score)\n\(snippet)"
        }.joined(separator: "\n\n")

        return [
            "You are Theorem's GL-Fusion 31B answer model.",
            "Use the Theorem search evidence below first. The evidence can include RustyWeb, native browser search, graph search, admitted WebDocs, and substrate results.",
            "If the evidence is thin, say what is missing instead of inventing citations.",
            "User query: \(query)",
            evidence.isEmpty ? "Theorem search evidence: no hits returned." : "Theorem search evidence:\n\(evidence)",
        ].joined(separator: "\n\n")
    }

    private func endpoint(_ path: String) throws -> URL {
        guard let url = URL(string: path, relativeTo: baseURL)?.absoluteURL else {
            throw TheoremSearchClientError.invalidEndpoint(path)
        }
        return url
    }

    private func post<Body: Encodable, Response: Decodable>(
        _ path: String,
        body: Body,
        response: Response.Type
    ) async throws -> Response {
        var request = URLRequest(url: try endpoint(path))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = try JSONEncoder().encode(body)

        let (data, urlResponse) = try await dataLoader(request)
        if let http = urlResponse as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            let detail = String(data: data, encoding: .utf8) ?? ""
            throw TheoremSearchClientError.httpStatus(http.statusCode, detail)
        }
        return try JSONDecoder().decode(Response.self, from: data)
    }
}
