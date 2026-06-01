import XCTest
@testable import TheoremIOSCore

/// Locks in the knowledge-frontier-map data path: the substrate backend tags
/// each hit with `provenance` ("corpus" = fetched/known, "frontier" =
/// discovered-but-unfetched/new), and the iOS scene mapping must thread that
/// value into each node atom's metadata so `ForceGraphView` can render corpus
/// hollow/white and frontier filled/black. A hit from a backend that predates
/// the field decodes nil and carries no provenance metadata (view falls back to
/// the degree rule). This test is backend-independent: it pins the contract the
/// deployed `/search.json` must keep emitting.
final class RustyRedProvenanceTests: XCTestCase {
    /// The `search` payload shape returned by rustyred-thg-server `/search.json`,
    /// with one fetched hit, one frontier hit, and one legacy hit (no provenance).
    private let searchJSON = """
    {
      "query": "graph",
      "hits": [
        {
          "node_id": "p_corpus",
          "url": "http://ex.com/knowledge-graph",
          "title": "Knowledge graph",
          "snippet": "A knowledge graph stores entities and their relations.",
          "ring": 0,
          "ring_label": "match",
          "match_score": 0.46,
          "provenance": "corpus"
        },
        {
          "node_id": "p_frontier",
          "url": "http://ex.com/graph-neural-network",
          "title": "Graph neural network",
          "snippet": "",
          "ring": 0,
          "ring_label": "match",
          "match_score": 0.005,
          "provenance": "frontier"
        },
        {
          "node_id": "p_legacy",
          "url": "http://ex.com/logical-graph",
          "title": "Logical graph",
          "snippet": "Older payload with no provenance field.",
          "ring": 1,
          "ring_label": "adjacent",
          "match_score": 0.01
        }
      ],
      "links": [
        { "source": "p_corpus", "target": "p_frontier" }
      ],
      "matched_count": 1,
      "kept_count": 3
    }
    """

    func testProvenanceDecodesIncludingLegacyNil() throws {
        let substrate = try JSONDecoder().decode(
            RustyRedSubstrateSearch.self, from: Data(searchJSON.utf8))
        let byID = Dictionary(uniqueKeysWithValues: substrate.hits.map { ($0.nodeID, $0) })

        XCTAssertEqual(byID["p_corpus"]?.provenance, "corpus")
        XCTAssertEqual(byID["p_frontier"]?.provenance, "frontier")
        // A backend that predates the field => nil, not a crash or empty string.
        XCTAssertNil(byID["p_legacy"]?.provenance ?? nil)
    }

    func testSceneThreadsProvenanceIntoAtomMetadata() throws {
        let substrate = try JSONDecoder().decode(
            RustyRedSubstrateSearch.self, from: Data(searchJSON.utf8))
        let scene = TheoremSearchClient.scene(from: substrate, tenant: nil, query: "graph")
        let atomByID = Dictionary(uniqueKeysWithValues: scene.atoms.map { ($0.id, $0) })

        // Fetched corpus => provenance "corpus" in metadata (renders hollow/white).
        XCTAssertEqual(atomByID["p_corpus"]?.metadata["provenance"]?.stringValue, "corpus")
        // Unfetched frontier => "frontier" (renders filled/black).
        XCTAssertEqual(atomByID["p_frontier"]?.metadata["provenance"]?.stringValue, "frontier")
        // Legacy hit => no provenance key at all (view uses the degree fallback).
        XCTAssertNil(atomByID["p_legacy"]?.metadata["provenance"])

        // Match score survives into metadata as the radius (centrality) signal.
        XCTAssertEqual(atomByID["p_corpus"]?.metadata["matchScore"]?.doubleValue, 0.46)
        XCTAssertEqual(atomByID["p_frontier"]?.metadata["matchScore"]?.doubleValue, 0.005)

        // The query seed is present and is not tagged with provenance.
        let queryAtom = scene.atoms.first { $0.kind == "query" }
        XCTAssertNotNil(queryAtom)
        XCTAssertNil(queryAtom?.metadata["provenance"])
    }
}
