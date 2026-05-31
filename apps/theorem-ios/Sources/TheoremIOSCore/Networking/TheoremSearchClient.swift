import Foundation

/// Talks to the hosted Theorem/RustyRed search API and turns a query into a real
/// `ScenePackageV2` the graph renders. This is the search-box to live-scene seam:
/// retrieval runs server-side; the phone receives the neighbourhood and draws it.
///
/// Ported from Codex's `apps/ios` search client, adapted to this app's models.
/// The default backend is the pure Rust `rustyred-thg-server` `/search.json`
/// route. Index-API normalization remains only as an explicit diagnostic
/// override via `-searchBackend index-api`.
public struct TheoremSearchClient: Sendable {
    public typealias DataLoader = @Sendable (URLRequest) async throws -> (Data, URLResponse)

    public static let productionIndexAPIBaseURL = URL(string: "https://index-api-production-a5f7.up.railway.app")!
    public static let productionRustyRedBaseURL = URL(string: "https://rustyredcore-theorem-production.up.railway.app")!
    public static let localRustyRedBaseURL = URL(string: "http://127.0.0.1:8380")!

    public let baseURL: URL
    public let backend: TheoremSearchBackend
    private let dataLoader: DataLoader

    public init(
        baseURL: URL? = nil,
        backend: TheoremSearchBackend? = nil,
        dataLoader: @escaping DataLoader = { try await URLSession.shared.data(for: $0) }
    ) {
        let resolvedBackend = backend ?? Self.configuredBackend
        self.backend = resolvedBackend
        self.baseURL = baseURL ?? Self.configuredBaseURL(for: resolvedBackend)
        self.dataLoader = dataLoader
    }

    /// Run a substrate search and return a scene the renderer can draw. Throws
    /// `TheoremSearchError` on a bad query, HTTP failure, or empty result (so the
    /// UI shows an honest error / empty state, never a fabricated graph).
    public func search(query: String, topK: Int = 12) async throws -> ScenePackageV2 {
        let clean = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { throw TheoremSearchError.emptyQuery }

        switch backend {
        case .indexAPI:
            return try await searchIndexAPI(query: clean, topK: topK)
        case .rustyRed:
            return try await searchRustyRed(query: clean)
        }
    }

    /// Ask the substrate for a graph-grounded summary of a focus (a node, in the
    /// context of the rest of the graph). Two-step async: enqueue the compose job,
    /// then consume the SSE stream to the terminal `complete` event. The compose
    /// engine is Index-API only, so this always targets the Index-API base
    /// regardless of the search backend. Honest failure paths only: the queue-down
    /// case returns 503 with a message, stream `error` events surface their
    /// payload. It never fabricates an answer.
    public func ask(query: String, maxObjects: Int = 8) async throws -> AskResult {
        let clean = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { throw TheoremSearchError.emptyQuery }
        let askBase = (backend == .indexAPI) ? baseURL : Self.productionIndexAPIBaseURL

        guard let enqueueURL = URL(string: "/api/v2/theseus/ask/async/", relativeTo: askBase)?.absoluteURL else {
            throw TheoremSearchError.invalidEndpoint("/api/v2/theseus/ask/async/")
        }
        var enqueueRequest = URLRequest(url: enqueueURL)
        enqueueRequest.httpMethod = "POST"
        enqueueRequest.setValue("application/json", forHTTPHeaderField: "Content-Type")
        enqueueRequest.setValue("application/json", forHTTPHeaderField: "Accept")
        enqueueRequest.httpBody = try JSONEncoder().encode(AskRequest(query: clean, maxObjects: maxObjects))

        let (enqueueData, enqueueResponse) = try await dataLoader(enqueueRequest)
        if let http = enqueueResponse as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            if let payload = try? JSONDecoder().decode(AskEnqueueResponse.self, from: enqueueData),
               let message = payload.message ?? payload.error {
                throw TheoremSearchError.server(message)
            }
            throw TheoremSearchError.httpStatus(http.statusCode)
        }
        let enqueue = try JSONDecoder().decode(AskEnqueueResponse.self, from: enqueueData)
        if let error = enqueue.error {
            throw TheoremSearchError.server(enqueue.message ?? error)
        }
        guard let streamPath = enqueue.streamURL,
              let streamURL = URL(string: streamPath, relativeTo: askBase)?.absoluteURL else {
            throw TheoremSearchError.server("Ask enqueue returned no stream URL.")
        }
        return try await consumeAskStream(streamURL)
    }

    /// Read the SSE stream line-by-line, dispatching on blank-line event
    /// boundaries, and return the answer at the terminal `complete` event.
    private func consumeAskStream(_ url: URL) async throws -> AskResult {
        var request = URLRequest(url: url)
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        request.timeoutInterval = 90
        let (bytes, response) = try await URLSession.shared.bytes(for: request)
        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            throw TheoremSearchError.httpStatus(http.statusCode)
        }
        var eventName = "message"
        var dataLines: [String] = []
        for try await line in bytes.lines {
            if line.hasPrefix(":") { continue }                 // SSE comment / keep-alive
            if line.isEmpty {
                guard !dataLines.isEmpty else { eventName = "message"; continue }
                let dataRaw = dataLines.joined(separator: "\n")
                dataLines.removeAll(keepingCapacity: true)
                if eventName == "complete" {
                    let payload = try JSONDecoder().decode(AskComplete.self, from: Data(dataRaw.utf8))
                    let answer = (payload.answer ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !answer.isEmpty else {
                        throw TheoremSearchError.server("The substrate returned an empty answer.")
                    }
                    return AskResult(answer: answer)
                }
                if eventName == "error" {
                    let message = (try? JSONDecoder().decode(AskErrorEvent.self, from: Data(dataRaw.utf8)))?.error
                    throw TheoremSearchError.server(message ?? "The substrate ask failed.")
                }
                eventName = "message"
                continue
            }
            if line.hasPrefix("event:") {
                eventName = line.dropFirst("event:".count).trimmingCharacters(in: .whitespaces)
            } else if line.hasPrefix("data:") {
                dataLines.append(String(line.dropFirst("data:".count)).trimmingCharacters(in: .whitespaces))
            }
        }
        throw TheoremSearchError.server("The ask stream ended before completing.")
    }

    private func searchIndexAPI(query clean: String, topK: Int) async throws -> ScenePackageV2 {
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

    private func searchRustyRed(query clean: String) async throws -> ScenePackageV2 {
        let tenant = Self.configuredTenant
        let route = try await get(
            "/search.json",
            queryItems: [
                URLQueryItem(name: "q", value: clean),
                tenant.map { URLQueryItem(name: "tenant", value: $0) },
            ].compactMap { $0 },
            response: RustyRedSearchResponse.self
        )
        if let ok = route.ok, !ok {
            throw TheoremSearchError.server(route.error ?? "RustyRed search failed.")
        }
        guard let search = route.search else {
            throw TheoremSearchError.server(route.error ?? "RustyRed search returned no search payload.")
        }
        let scene = Self.scene(from: search, tenant: route.tenant ?? tenant, query: clean)
        guard !scene.atoms.isEmpty else { throw TheoremSearchError.noResults(clean) }
        return scene
    }

    // MARK: native response -> ScenePackageV2

    static func scene(from native: TheoremNativeSearchResponse, query: String) -> ScenePackageV2 {
        var seen = Set<String>()
        var atoms: [SceneAtom] = []
        let queryID = native.graphNodes?
            .first(where: { ($0.label ?? "").caseInsensitiveCompare("Query") == .orderedSame })?
            .id ?? "query:\(stableQueryID(query))"

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
            if nodeID == queryID {
                atoms.append(queryAtom(id: nodeID, query: query))
            } else {
                atoms.append(atom(
                    id: nodeID, url: url, title: title,
                    snippet: node.properties?["snippet"]?.stringValue ?? "", ring: 1, ringLabel: "adjacent",
                    score: node.properties?["score"]?.doubleValue ?? 0))
            }
        }

        if !seen.contains(queryID) {
            seen.insert(queryID)
            atoms.insert(queryAtom(id: queryID, query: query), at: 0)
        }

        let ids = Set(atoms.map(\.id))
        var relations = (native.graphEdges ?? []).compactMap { edge -> SceneRelation? in
            let source = edge.source ?? edge.fromID ?? ""
            let target = edge.target ?? edge.toID ?? ""
            guard ids.contains(source), ids.contains(target), source != target else { return nil }
            return SceneRelation(
                id: "\(source)->\(target)", sourceId: source, targetId: target,
                kind: edge.kind ?? edge.edgeType ?? "links_to",
                weight: edge.weight,
                metadata: [
                    "reason": .string(edge.reason ?? ""),
                    "source": .string("native-search"),
                ])
        }
        if relations.isEmpty {
            relations = retrievalRelations(queryID: queryID, atoms: atoms)
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
                "relation_count": .double(Double(relations.count)),
                "backend": .string(TheoremSearchBackend.indexAPI.rawValue),
            ])
    }

    static func scene(from substrate: RustyRedSubstrateSearch, tenant: String?, query: String) -> ScenePackageV2 {
        let queryID = "query:\(stableQueryID(query))"
        var atoms = [queryAtom(id: queryID, query: query)]
        atoms.append(contentsOf: substrate.hits.map { hit in
            atom(
                id: hit.nodeID, url: hit.url, title: hit.title, snippet: hit.snippet,
                ring: hit.ring, ringLabel: hit.ringLabel, score: hit.matchScore)
        })

        let ids = Set(atoms.map(\.id))
        var relations = substrate.links.compactMap { link -> SceneRelation? in
            guard ids.contains(link.source), ids.contains(link.target), link.source != link.target else {
                return nil
            }
            return SceneRelation(
                id: "\(link.source)->\(link.target)",
                sourceId: link.source,
                targetId: link.target,
                kind: "LINKS_TO",
                metadata: ["source": .string("rustyred-substrate")])
        }
        relations.append(contentsOf: retrievalRelations(queryID: queryID, atoms: atoms))

        let actions = atoms.compactMap { atom -> ActionDescriptor? in
            guard let url = atom.metadata["url"]?.stringValue, !url.isEmpty else { return nil }
            return ActionDescriptor(
                id: "open-\(atom.id)", label: "Open", actionType: "open-url",
                interaction: "tap", target: atom.id, payload: ["url": .string(url)])
        }

        return ScenePackageV2(
            id: "rustyred-search-\(UUID().uuidString)",
            manifestRef: tenant ?? "rustyred-search",
            atoms: atoms,
            relations: relations,
            projection: ProjectionBinding(id: ProjectionID.forceGraph.rawValue),
            chrome: ChromeBinding(id: "rustyred_search_scene"),
            actions: actions,
            provenance: [
                "source": .string("rustyred-thg-server"),
                "query": .string(query),
                "matched_count": .double(Double(substrate.matchedCount)),
                "kept_count": .double(Double(substrate.keptCount)),
                "relation_count": .double(Double(relations.count)),
                "backend": .string(TheoremSearchBackend.rustyRed.rawValue),
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
                "matchScore": .double(score),
            ],
            sourceRefs: [SourceRef(kind: "WebDoc", id: id, label: title, metadata: ["url": .string(url)])])
    }

    private static func queryAtom(id: String, query: String) -> SceneAtom {
        SceneAtom(
            id: id,
            kind: "query",
            label: query,
            weight: 0.25,
            lifecycle: "present",
            metadata: [
                "ring": .double(0),
                "ring_label": .string("query"),
                "match_score": .double(0.25),
                "matchScore": .double(0.25),
                "query": .string(query),
            ])
    }

    private static func retrievalRelations(queryID: String, atoms: [SceneAtom]) -> [SceneRelation] {
        atoms.compactMap { atom in
            guard atom.id != queryID,
                  atom.metadata["ring"]?.intValue == 0 else { return nil }
            return SceneRelation(
                id: "\(queryID)->RETRIEVED->\(atom.id)",
                sourceId: queryID,
                targetId: atom.id,
                kind: "RETRIEVED",
                weight: atom.weight,
                metadata: ["source": .string("client-query-edge")])
        }
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

    private static func stableQueryID(_ query: String) -> String {
        String(query.lowercased().unicodeScalars.map(\.value).reduce(UInt32(2_166_136_261)) { hash, scalar in
            (hash ^ scalar) &* 16_777_619
        }, radix: 16)
    }

    private static var configuredBackend: TheoremSearchBackend {
        let raw = UserDefaults.standard.string(forKey: "searchBackend")
            ?? UserDefaults.standard.string(forKey: "theoremSearchBackend")
        return raw.flatMap(TheoremSearchBackend.init(rawValue:)) ?? .rustyRed
    }

    private static func configuredBaseURL(for backend: TheoremSearchBackend) -> URL {
        let raw = UserDefaults.standard.string(forKey: "searchBaseURL")
            ?? UserDefaults.standard.string(forKey: "theoremSearchBaseURL")
        if let raw, let url = URL(string: raw), !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return url
        }
        switch backend {
        case .indexAPI:
            return productionIndexAPIBaseURL
        case .rustyRed:
            return productionRustyRedBaseURL
        }
    }

    private static var configuredTenant: String? {
        nonEmpty(UserDefaults.standard.string(forKey: "searchTenant")
            ?? UserDefaults.standard.string(forKey: "theoremSearchTenant"))
    }

    // MARK: HTTP

    private func get<Response: Decodable>(
        _ path: String, queryItems: [URLQueryItem], response: Response.Type
    ) async throws -> Response {
        guard let base = URL(string: path, relativeTo: baseURL)?.absoluteURL,
              var components = URLComponents(url: base, resolvingAgainstBaseURL: false) else {
            throw TheoremSearchError.invalidEndpoint(path)
        }
        components.queryItems = queryItems
        guard let url = components.url else {
            throw TheoremSearchError.invalidEndpoint(path)
        }
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let (data, urlResponse) = try await dataLoader(request)
        if let http = urlResponse as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            throw TheoremSearchError.httpStatus(http.statusCode)
        }
        return try JSONDecoder().decode(Response.self, from: data)
    }

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

public enum TheoremSearchBackend: String, Sendable, Equatable {
    case indexAPI = "index-api"
    case rustyRed = "rustyred"
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

// MARK: - Wire types (ask / compose)

struct AskRequest: Encodable {
    let query: String
    let maxObjects: Int
    let mode = "full"
    let includeWeb = true
    let scope = "all"
    let renderHints: [String: String] = [:]

    enum CodingKeys: String, CodingKey {
        case query, mode, scope
        case maxObjects = "max_objects"
        case includeWeb = "include_web"
        case renderHints = "render_hints"
    }
}

struct AskEnqueueResponse: Decodable {
    let jobId: String?
    let streamURL: String?
    let error: String?
    let message: String?

    enum CodingKeys: String, CodingKey {
        case jobId = "job_id"
        case streamURL = "stream_url"
        case error, message
    }
}

struct AskComplete: Decodable {
    let answer: String?
}

struct AskErrorEvent: Decodable {
    let error: String?
}

/// The graph-grounded answer the substrate composed for a focus.
public struct AskResult: Sendable, Equatable {
    public let answer: String
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
    let weight: Double?
    let reason: String?

    enum CodingKeys: String, CodingKey {
        case source, target, kind, weight, reason
        case fromID = "from_id"
        case toID = "to_id"
        case edgeType = "edge_type"
    }
}

struct RustyRedSearchResponse: Codable, Hashable, Sendable {
    let ok: Bool?
    let tenant: String?
    let search: RustyRedSubstrateSearch?
    let error: String?
}

struct RustyRedSubstrateSearch: Codable, Hashable, Sendable {
    let query: String
    let hits: [RustyRedSearchHit]
    let links: [RustyRedSearchLink]
    let matchedCount: Int
    let keptCount: Int

    enum CodingKeys: String, CodingKey {
        case query, hits, links
        case matchedCount = "matched_count"
        case keptCount = "kept_count"
    }
}

struct RustyRedSearchHit: Codable, Hashable, Sendable {
    let nodeID: String
    let url: String
    let title: String
    let snippet: String
    let ring: Int
    let ringLabel: String
    let matchScore: Double

    enum CodingKeys: String, CodingKey {
        case nodeID = "node_id"
        case url, title, snippet, ring
        case ringLabel = "ring_label"
        case matchScore = "match_score"
    }
}

struct RustyRedSearchLink: Codable, Hashable, Sendable {
    let source: String
    let target: String
}
