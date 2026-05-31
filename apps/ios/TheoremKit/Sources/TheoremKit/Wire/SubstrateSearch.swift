import Foundation

/// One page in the result neighbourhood, annotated with how it got there.
///
/// Mirrors `rustyred_web::search::SearchHit` (snake_case JSON; the struct has no
/// serde `rename_all`). `matchScore` is PPR mass — the single most important
/// field for the UI: the highest-`matchScore` node is "the center of what you
/// searched", and the Dynamic Island names it (spec "data contracts").
public struct SearchHit: Codable, Hashable, Sendable, Identifiable {
    /// The substrate node id of the page. Doubles as the SwiftUI identity.
    public let nodeID: String
    public let url: String
    public let title: String
    /// Page excerpt; empty for a discovered-but-unfetched link target.
    public let snippet: String
    /// Hop distance to the nearest direct match. 0 = the page itself matched.
    public let ring: Int
    /// Plain-language ring name: match / adjacent / nearby / distant / browse.
    public let ringLabel: String
    /// Graph-aware relevance score (PPR mass). The centrality signal.
    public let matchScore: Double

    public var id: String { nodeID }

    enum CodingKeys: String, CodingKey {
        case nodeID = "node_id"
        case url
        case title
        case snippet
        case ring
        case ringLabel = "ring_label"
        case matchScore = "match_score"
    }

    public init(
        nodeID: String,
        url: String,
        title: String,
        snippet: String,
        ring: Int,
        ringLabel: String,
        matchScore: Double
    ) {
        self.nodeID = nodeID
        self.url = url
        self.title = title
        self.snippet = snippet
        self.ring = ring
        self.ringLabel = ringLabel
        self.matchScore = matchScore
    }
}

/// A `LINKS_TO` edge whose both endpoints are in `hits`.
/// Mirrors `rustyred_web::search::SearchLink` (snake_case: `source`, `target`).
public struct SearchLink: Codable, Hashable, Sendable {
    public let source: String
    public let target: String

    public init(source: String, target: String) {
        self.source = source
        self.target = target
    }
}

/// The result of a substrate search: matched pages + their link neighbourhood.
/// Mirrors `rustyred_web::search::SubstrateSearch` (snake_case JSON).
public struct SubstrateSearch: Codable, Hashable, Sendable {
    /// The normalized query (trimmed, lower-cased). Empty = browse mode.
    public let query: String
    /// Pages in the neighbourhood, ordered by (ring, node_id) for determinism.
    public let hits: [SearchHit]
    /// `LINKS_TO` edges whose both endpoints are in `hits`.
    public let links: [SearchLink]
    /// How many pages directly matched the query (ring 0).
    public let matchedCount: Int
    /// Total pages in the returned neighbourhood.
    public let keptCount: Int

    enum CodingKeys: String, CodingKey {
        case query
        case hits
        case links
        case matchedCount = "matched_count"
        case keptCount = "kept_count"
    }

    public init(
        query: String,
        hits: [SearchHit],
        links: [SearchLink],
        matchedCount: Int,
        keptCount: Int
    ) {
        self.query = query
        self.hits = hits
        self.links = links
        self.matchedCount = matchedCount
        self.keptCount = keptCount
    }
}

public extension SubstrateSearch {
    init(nativeSearch response: TheoremNativeSearchResponse, fallbackQuery: String = "") {
        var seen = Set<String>()
        var hits: [SearchHit] = []

        for (index, result) in (response.rankedResults ?? []).enumerated() {
            let nodeID = Self.searchNodeID(
                id: result.id,
                url: result.url,
                fallback: "ranked-\(index + 1)"
            )
            guard seen.insert(nodeID).inserted else { continue }
            hits.append(SearchHit(
                nodeID: nodeID,
                url: result.url ?? result.metadata?["url"]?.stringValue ?? "",
                title: result.title ?? result.metadata?["title"]?.stringValue ?? "",
                snippet: result.snippet ?? "",
                ring: 0,
                ringLabel: "match",
                matchScore: result.score ?? 0
            ))
        }

        for (index, node) in (response.graphNodes ?? []).enumerated() {
            let nodeID = Self.searchNodeID(
                id: node.id,
                url: node.properties?["url"]?.stringValue,
                fallback: "graph-\(index + 1)"
            )
            guard seen.insert(nodeID).inserted else { continue }
            hits.append(SearchHit(
                nodeID: nodeID,
                url: node.properties?["url"]?.stringValue ?? "",
                title: node.title ?? node.label ?? "",
                snippet: node.properties?["snippet"]?.stringValue ?? "",
                ring: 1,
                ringLabel: "adjacent",
                matchScore: node.properties?["score"]?.doubleValue ?? 0
            ))
        }

        let ids = Set(hits.map(\.nodeID))
        let links = (response.graphEdges ?? []).compactMap { edge -> SearchLink? in
            let source = edge.source ?? edge.fromID ?? ""
            let target = edge.target ?? edge.toID ?? ""
            guard ids.contains(source), ids.contains(target), source != target else {
                return nil
            }
            return SearchLink(source: source, target: target)
        }

        self.init(
            query: response.query ?? fallbackQuery,
            hits: hits,
            links: links,
            matchedCount: (response.rankedResults ?? []).count,
            keptCount: hits.count
        )
    }

    init(evidenceSearch response: TheoremEvidenceSearchResponse, fallbackQuery: String = "") {
        let hits = (response.results ?? []).enumerated().map { index, result in
            let nodeID = Self.searchNodeID(
                id: result.objectID ?? result.webdocID,
                url: result.url ?? result.canonicalURL,
                fallback: "evidence-\(index + 1)"
            )
            let snippet = result.snippet ?? String((result.markdown ?? "").prefix(360))
            return SearchHit(
                nodeID: nodeID,
                url: result.url ?? result.canonicalURL ?? "",
                title: result.title ?? "",
                snippet: snippet,
                ring: 0,
                ringLabel: "match",
                matchScore: result.rankScore ?? result.sourceQuality ?? 0
            )
        }

        self.init(
            query: fallbackQuery,
            hits: hits,
            links: [],
            matchedCount: hits.count,
            keptCount: hits.count
        )
    }

    func scenePackage(id: String, manifestRef: String) -> ScenePackageV2 {
        let atoms = hits.map { hit in
            SceneAtom(
                id: hit.nodeID,
                kind: "source",
                label: hit.title.isEmpty ? hit.url : hit.title,
                weight: max(hit.matchScore, 0.05),
                lifecycle: .present,
                metadata: [
                    "url": .string(hit.url),
                    "snippet": .string(hit.snippet),
                    "ring": .number(Double(hit.ring)),
                    "ring_label": .string(hit.ringLabel),
                    "match_score": .number(hit.matchScore),
                ],
                sourceRefs: [
                    SourceRef(
                        kind: "WebDoc",
                        id: hit.nodeID,
                        label: hit.title,
                        metadata: ["url": .string(hit.url)]
                    ),
                ]
            )
        }

        let relations = links.map { link in
            SceneRelation(
                id: "\(link.source)->\(link.target)",
                sourceId: link.source,
                targetId: link.target,
                kind: "links_to"
            )
        }

        let actions = hits.compactMap { hit -> ActionDescriptor? in
            guard !hit.url.isEmpty else { return nil }
            return ActionDescriptor(
                id: "open-\(hit.nodeID)",
                label: "Open",
                actionType: "open-url",
                interaction: "tap",
                target: hit.nodeID,
                payload: ["url": .string(hit.url)]
            )
        }

        return ScenePackageV2(
            id: id,
            manifestRef: manifestRef,
            atoms: atoms,
            relations: relations,
            projection: ProjectionBinding(id: ProjectionID.forceGraph),
            chrome: ChromeBinding(id: "search_scene"),
            actions: actions,
            provenance: [
                "source": .string("theorem-native-search"),
                "query": .string(query),
                "matched_count": .number(Double(matchedCount)),
                "kept_count": .number(Double(keptCount)),
            ]
        )
    }

    private static func searchNodeID(id: String?, url: String?, fallback: String) -> String {
        let cleanID = (id ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        if !cleanID.isEmpty { return cleanID }

        let cleanURL = (url ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        if !cleanURL.isEmpty { return cleanURL }

        return fallback
    }
}
