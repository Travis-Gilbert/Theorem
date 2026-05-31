import TheoremIOSCore

let availability = TheoremProjectionEngine.availableProjections(for: SampleScene.package)
precondition(availability.count == 4, "expected four iOS v1 projections")
precondition(availability.allSatisfy(\.available), "sample scene should support all projections")

let cyclic = ScenePackageV2(
    id: "cyclic-scene",
    manifestRef: "cyclic-manifest",
    atoms: [
        atom("a", ring: 0, score: 0.9),
        atom("b", ring: 1, score: 0.4),
        atom("c", ring: 1, score: 0.3),
    ],
    relations: [
        relation("a", "b"),
        relation("b", "c"),
        relation("c", "a"),
    ],
    projection: ProjectionBinding(id: ProjectionID.forceGraph.rawValue, params: [:]),
    chrome: ChromeBinding(id: "dynamic_island_shell", params: [:])
)

let tree = TheoremProjectionEngine.availableProjections(for: cyclic)
    .first { $0.id == .treeLayout }
precondition(tree?.available == false, "tree projection must reject cycles")

let centerScene = ScenePackageV2(
    id: "center-scene",
    manifestRef: "center-manifest",
    atoms: [
        atom("dense", ring: 1, score: 0.2),
        atom("center", ring: 0, score: 0.95),
        atom("leaf", ring: 1, score: 0.1),
    ],
    relations: [
        relation("dense", "leaf"),
        relation("center", "leaf"),
    ],
    projection: ProjectionBinding(id: ProjectionID.forceGraph.rawValue, params: [:]),
    chrome: ChromeBinding(id: "dynamic_island_shell", params: [:])
)

precondition(
    TheoremProjectionEngine.centerNodeID(in: centerScene, mode: .pprMass) == "center",
    "PPR mass should control center node selection"
)

print("TheoremIOSSmoke passed")

private func atom(_ id: String, ring: Int, score: Double) -> SceneAtom {
    SceneAtom(
        id: id,
        kind: "concept",
        label: id,
        weight: score,
        metadata: [
            "ring": .double(Double(ring)),
            "matchScore": .double(score),
        ]
    )
}

private func relation(_ source: String, _ target: String) -> SceneRelation {
    SceneRelation(
        id: "\(source)->\(target):links_to",
        sourceId: source,
        targetId: target,
        kind: "links_to",
        weight: 1.0
    )
}
