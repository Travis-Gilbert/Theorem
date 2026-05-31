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
