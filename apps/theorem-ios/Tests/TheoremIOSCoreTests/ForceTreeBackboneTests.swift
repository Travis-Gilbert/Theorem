import XCTest
@testable import TheoremIOSCore

/// Pins the spanning-tree backbone that turns the relation knot into a legible
/// force-tree. Four properties matter: an existing tree is preserved, a hub-star
/// stays a depth-1 star, disconnected atoms attach to the root (never float off),
/// and heavier edges win so the tree follows the strongest relationships.
final class ForceTreeBackboneTests: XCTestCase {
    private func atom(_ id: String, score: Double) -> SceneAtom {
        SceneAtom(id: id, kind: "evidence", label: id, weight: score)
    }

    private func relation(_ from: String, _ to: String, weight: Double = 1.0) -> SceneRelation {
        SceneRelation(id: "\(from)->\(to)", sourceId: from, targetId: to, kind: "links_to", weight: weight)
    }

    private func edgeSet(_ backbone: ForceTreeBackbone) -> Set<String> {
        Set(backbone.edges.map { "\($0.parent)->\($0.child)" })
    }

    /// The sample scene is already a rooted tree; the backbone must reproduce it
    /// exactly (root = highest score, every original edge kept, correct depths).
    func testExistingTreeIsPreserved() {
        let backbone = ForceTree.backbone(
            atoms: SampleScene.package.atoms,
            relations: SampleScene.package.relations
        )

        XCTAssertEqual(backbone.rootID, "theorem")
        XCTAssertEqual(backbone.edges.count, 5)
        XCTAssertEqual(edgeSet(backbone), [
            "theorem->rustyweb", "theorem->sceneos", "theorem->island",
            "rustyweb->pushppr", "sceneos->safari",
        ])
        XCTAssertEqual(backbone.depth["theorem"], 0)
        XCTAssertEqual(backbone.depth["rustyweb"], 1)
        XCTAssertEqual(backbone.depth["pushppr"], 2)
        XCTAssertEqual(backbone.depth["safari"], 2)
        XCTAssertEqual(backbone.childCount["theorem"], 3)
        XCTAssertEqual(backbone.childCount["pushppr"] ?? 0, 0) // leaf
    }

    /// A hub linked to leaves stays a flat star: every leaf is a depth-1 child of
    /// the hub, the hub is the root, and no leaf becomes another leaf's parent.
    func testHubStarStaysDepthOne() {
        let atoms = [
            atom("hub", score: 1.0),
            atom("a", score: 0.5),
            atom("b", score: 0.4),
            atom("c", score: 0.3),
        ]
        let relations = [relation("hub", "a"), relation("hub", "b"), relation("hub", "c")]

        let backbone = ForceTree.backbone(atoms: atoms, relations: relations)

        XCTAssertEqual(backbone.rootID, "hub")
        XCTAssertEqual(backbone.childCount["hub"], 3)
        for leaf in ["a", "b", "c"] {
            XCTAssertEqual(backbone.depth[leaf], 1, "\(leaf) should sit one ring out from the hub")
            XCTAssertEqual(backbone.childCount[leaf] ?? 0, 0)
        }
    }

    /// A frontier atom with no shown edge must attach to the root, not vanish or
    /// drift off-canvas. This is the live-search case: ring-1 frontier nodes with
    /// no substrate link among the displayed set.
    func testDisconnectedAtomAttachesToRoot() {
        let atoms = [
            atom("root", score: 1.0),
            atom("linked", score: 0.5),
            atom("orphan", score: 0.4),
        ]
        let relations = [relation("root", "linked")]

        let backbone = ForceTree.backbone(atoms: atoms, relations: relations)

        XCTAssertEqual(backbone.rootID, "root")
        XCTAssertEqual(backbone.edges.count, 2)
        XCTAssertEqual(edgeSet(backbone), ["root->linked", "root->orphan"])
        XCTAssertEqual(backbone.depth["orphan"], 1)
        XCTAssertEqual(backbone.childCount["root"], 2)
    }

    /// Max-weight selection: a heavy a->b edge must pull b under a, even though b
    /// also touches the root by a lighter edge. The tree follows the strongest tie.
    func testHeavierEdgeWins() {
        let atoms = [
            atom("root", score: 1.0),
            atom("a", score: 0.6),
            atom("b", score: 0.5),
        ]
        let relations = [
            relation("root", "a", weight: 1.0),
            relation("root", "b", weight: 1.0),
            relation("a", "b", weight: 10.0),
        ]

        let backbone = ForceTree.backbone(atoms: atoms, relations: relations)

        XCTAssertEqual(backbone.rootID, "root")
        XCTAssertTrue(edgeSet(backbone).contains("a->b"), "b should hang off a via the heavy edge")
        XCTAssertFalse(edgeSet(backbone).contains("root->b"), "the light root->b edge must lose")
        XCTAssertEqual(backbone.depth["b"], 2)
    }

    /// A single isolated atom yields an empty edge set with itself as root, and an
    /// empty scene yields the empty backbone (no crash on the degenerate inputs).
    func testDegenerateInputs() {
        let single = ForceTree.backbone(atoms: [atom("solo", score: 1.0)], relations: [])
        XCTAssertEqual(single.rootID, "solo")
        XCTAssertTrue(single.edges.isEmpty)
        XCTAssertEqual(single.depth["solo"], 0)

        XCTAssertEqual(ForceTree.backbone(atoms: [], relations: []), .empty)
    }
}
