import Foundation

public enum SampleScene {
    public static let package = ScenePackageV2(
        id: "sample-substrate-scene",
        manifestRef: "sample-ios-manifest",
        atoms: [
            atom("theorem", "Theorem", ring: 0, score: 0.92, kind: "core"),
            atom("rustyweb", "RustyWeb", ring: 1, score: 0.54, kind: "web"),
            atom("sceneos", "SceneOS", ring: 1, score: 0.48, kind: "tool"),
            atom("pushppr", "Push PPR", ring: 2, score: 0.34, kind: "core"),
            atom("safari", "Host Handoff", ring: 2, score: 0.22, kind: "web"),
            atom("island", "Dynamic Island", ring: 1, score: 0.41, kind: "tool"),
        ],
        relations: [
            relation("theorem", "rustyweb"),
            relation("theorem", "sceneos"),
            relation("theorem", "island"),
            relation("rustyweb", "pushppr"),
            relation("sceneos", "safari"),
        ],
        projection: ProjectionBinding(id: ProjectionID.forceGraph.rawValue, params: [:]),
        chrome: ChromeBinding(id: "dynamic_island_shell", params: [:])
    )

    private static func atom(
        _ id: String,
        _ label: String,
        ring: Int,
        score: Double,
        kind: String
    ) -> SceneAtom {
        SceneAtom(
            id: id,
            kind: kind,
            label: label,
            weight: score,
            metadata: [
                "ring": .double(Double(ring)),
                "matchScore": .double(score),
                "url": .string("https://theorem.local/\(id)"),
            ]
        )
    }

    private static func relation(_ source: String, _ target: String) -> SceneRelation {
        SceneRelation(
            id: "\(source)->\(target):links_to",
            sourceId: source,
            targetId: target,
            kind: "links_to",
            weight: 1.0
        )
    }
}
