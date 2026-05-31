import Foundation
import XCTest
@testable import TheoremKit

/// These fixtures are the BYTES the Rust serde layer emits, including its
/// `skip_serializing_if` omissions (no `metadata`/`sourceRefs`/`position` keys
/// when empty/None). A model that only decodes a "full" object would pass a
/// naive test and crash on the first real payload; these assert the omission
/// handling the spec depends on.
///
/// (Swift Testing's `Testing` module ships with full Xcode, not the standalone
/// command-line tools, so these use XCTest to stay CI-portable.)
final class WireModelTests: XCTestCase {

    // MARK: SubstrateSearch (snake_case)

    func testSubstrateSearchDecodesSnakeCase() throws {
        let json = """
        {
          "query": "knowledge graph",
          "hits": [
            {"node_id":"p1","url":"https://a.test/x","title":"X","snippet":"hello",
             "ring":0,"ring_label":"match","match_score":0.83},
            {"node_id":"p2","url":"https://b.test/","title":"b.test","snippet":"",
             "ring":1,"ring_label":"adjacent","match_score":0.12}
          ],
          "links": [{"source":"p1","target":"p2"}],
          "matched_count": 1,
          "kept_count": 2
        }
        """.data(using: .utf8)!

        let search = try JSONDecoder().decode(SubstrateSearch.self, from: json)
        XCTAssertEqual(search.query, "knowledge graph")
        XCTAssertEqual(search.hits.count, 2)
        XCTAssertEqual(search.hits[0].nodeID, "p1")
        XCTAssertEqual(search.hits[0].matchScore, 0.83)
        XCTAssertEqual(search.hits[0].ringLabel, "match")
        XCTAssertEqual(search.hits[1].snippet, "")
        XCTAssertEqual(search.links, [SearchLink(source: "p1", target: "p2")])
        XCTAssertEqual(search.matchedCount, 1)
        XCTAssertEqual(search.keptCount, 2)
    }

    func testNativeSearchConvertsToSubstrateScene() throws {
        let json = """
        {
          "query": "rusty web",
          "search_session_id": "searchsess:ios",
          "ranked_results": [
            {
              "id": "webdoc:1",
              "title": "RustyWeb result",
              "url": "https://example.test/rustyweb",
              "snippet": "RustyWeb found this page.",
              "score": 0.91,
              "source_type": "webdoc"
            }
          ],
          "graph_nodes": [
            {
              "id": "webdoc:2",
              "label": "Adjacent page",
              "properties": {
                "url": "https://example.test/adjacent",
                "score": 0.33
              }
            }
          ],
          "graph_edges": [
            {"source": "webdoc:1", "target": "webdoc:2", "kind": "links_to"}
          ]
        }
        """.data(using: .utf8)!

        let native = try JSONDecoder().decode(TheoremNativeSearchResponse.self, from: json)
        let search = SubstrateSearch(nativeSearch: native)
        XCTAssertEqual(search.query, "rusty web")
        XCTAssertEqual(search.matchedCount, 1)
        XCTAssertEqual(search.keptCount, 2)
        XCTAssertEqual(search.hits[0].nodeID, "webdoc:1")
        XCTAssertEqual(search.hits[0].ringLabel, "match")
        XCTAssertEqual(search.hits[1].ringLabel, "adjacent")
        XCTAssertEqual(search.links, [SearchLink(source: "webdoc:1", target: "webdoc:2")])

        let scene = search.scenePackage(id: "scene-1", manifestRef: "searchsess:ios")
        XCTAssertEqual(scene.atoms.count, 2)
        XCTAssertEqual(scene.relations.count, 1)
        XCTAssertEqual(scene.projection.id, ProjectionID.forceGraph)
        XCTAssertEqual(scene.provenance["source"]?.stringValue, "theorem-native-search")
        XCTAssertEqual(scene.atoms[0].metadata["match_score"]?.doubleValue, 0.91)
    }

    func test31BPromptUsesSearchEvidence() {
        let search = SubstrateSearch(
            query: "rusty web",
            hits: [
                SearchHit(
                    nodeID: "webdoc:1",
                    url: "https://example.test/rustyweb",
                    title: "RustyWeb result",
                    snippet: "RustyWeb found this page.",
                    ring: 0,
                    ringLabel: "match",
                    matchScore: 0.91
                ),
            ],
            links: [],
            matchedCount: 1,
            keptCount: 1
        )

        let prompt = TheoremSearchClient.build31BPrompt(query: "rusty web", search: search)
        XCTAssertTrue(prompt.contains("GL-Fusion 31B"))
        XCTAssertTrue(prompt.contains("Theorem search evidence"))
        XCTAssertTrue(prompt.contains("RustyWeb found this page."))
    }

    // MARK: ScenePackageV2 (camelCase, kebab enums, omitted-empty fields)

    func testScenePackageDecodesWireShape() throws {
        // Mirrors scene_os_core's serialized output: camelCase keys, kebab
        // `lifecycle`, projection.params OMITTED (empty), atom.metadata/position
        // OMITTED, relation carries sourceId/targetId.
        let json = """
        {
          "version": "scene-package-v2",
          "id": "pkg-1",
          "manifestRef": "manifest-1",
          "atoms": [
            {"id":"a","kind":"evidence","opacity":0.8,"lifecycle":"present",
             "sourceRefs":[{"kind":"Object","id":"42","metadata":{"score":0.91}}]},
            {"id":"b","kind":"claim","lifecycle":"present",
             "position":{"x":10.0,"y":20.0,"space":"graph"},
             "metadata":{"ring":2,"match_score":0.4}}
          ],
          "relations": [
            {"id":"a->b","sourceId":"a","targetId":"b","kind":"supports","weight":1.0,"lifecycle":"present"}
          ],
          "projection": {"id":"patent_diagram"},
          "chrome": {"id":"patent_plate_shell"}
        }
        """.data(using: .utf8)!

        let pkg = try ScenePackageV2.decode(from: json)
        XCTAssertEqual(pkg.version, "scene-package-v2")
        XCTAssertEqual(pkg.manifestRef, "manifest-1")
        XCTAssertEqual(pkg.atoms.count, 2)
        XCTAssertEqual(pkg.projection.id, "patent_diagram")
        XCTAssertTrue(pkg.projection.params.isEmpty)        // omitted -> default
        XCTAssertEqual(pkg.chrome.id, "patent_plate_shell")
        XCTAssertTrue(pkg.actions.isEmpty)                  // omitted -> default
        XCTAssertNil(pkg.transitions)                       // omitted -> nil
        XCTAssertNil(pkg.terminalState)

        let a = pkg.atoms[0]
        XCTAssertEqual(a.kind, "evidence")
        XCTAssertEqual(a.lifecycle, .present)
        XCTAssertNil(a.position)                            // omitted -> nil
        XCTAssertTrue(a.metadata.isEmpty)                   // omitted -> default
        XCTAssertEqual(a.sourceRefs.count, 1)
        XCTAssertEqual(a.sourceRefs[0].id, "42")
        XCTAssertEqual(a.sourceRefs[0].metadata["score"]?.doubleValue, 0.91)

        let b = pkg.atoms[1]
        XCTAssertEqual(b.position?.space, .graph)
        XCTAssertEqual(b.position?.x, 10.0)
        XCTAssertEqual(b.ring, 2)                           // read from metadata
        XCTAssertEqual(b.magnitude, 0.4)                    // match_score via metadata

        let rel = pkg.relations[0]
        XCTAssertEqual(rel.sourceId, "a")
        XCTAssertEqual(rel.targetId, "b")
        XCTAssertEqual(rel.kind, "supports")
    }

    func testMinimalAtomDecodesWithDefaults() throws {
        // The leanest atom the wire can carry: kind+lifecycle defaulted,
        // everything else omitted.
        let json = #"{"id":"x","kind":"evidence","lifecycle":"present"}"#.data(using: .utf8)!
        let atom = try JSONDecoder().decode(SceneAtom.self, from: json)
        XCTAssertEqual(atom.id, "x")
        XCTAssertEqual(atom.kind, "evidence")
        XCTAssertEqual(atom.lifecycle, .present)
        XCTAssertNil(atom.position)
        XCTAssertTrue(atom.metadata.isEmpty)
        XCTAssertTrue(atom.sourceRefs.isEmpty)
        XCTAssertEqual(atom.magnitude, 1)                   // no weight/score -> 1
        XCTAssertNil(atom.ring)
    }

    func testScenePackageRoundTrips() throws {
        let pkg = ScenePackageV2(
            id: "pkg-2",
            manifestRef: "m2",
            atoms: [
                SceneAtom(id: "a", kind: "source", label: "Alpha",
                          position: AtomPosition(x: 1, y: 2, space: .graph),
                          weight: 3, lifecycle: .present),
            ],
            relations: [
                SceneRelation(id: "a->a", sourceId: "a", targetId: "a", kind: "self"),
            ],
            projection: ProjectionBinding(id: "force_graph"),
            chrome: ChromeBinding(id: "document_rail")
        )
        let data = try JSONEncoder().encode(pkg)
        let back = try ScenePackageV2.decode(from: data)
        XCTAssertEqual(back, pkg)
    }
}
