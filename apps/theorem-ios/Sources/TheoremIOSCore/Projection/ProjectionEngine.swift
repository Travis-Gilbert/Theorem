import Foundation

public enum ProjectionID: String, CaseIterable, Identifiable, Sendable {
    case forceGraph = "force_graph"
    case radialRings = "radial_rings"
    case treeLayout = "tree_layout"
    case fractalExpansion = "fractal_expansion"

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .forceGraph:
            // The live Grape projection is now a force-tree (backbone-driven), so
            // it carries the "Tree" name; the rigid static layout becomes "Tiers".
            "Tree"
        case .radialRings:
            "Rings"
        case .treeLayout:
            "Tiers"
        case .fractalExpansion:
            "Fractal"
        }
    }
}

public struct ProjectionAvailability: Equatable, Identifiable, Sendable {
    public var id: ProjectionID
    public var available: Bool
    public var reason: String
}

public enum CentralityMode: Sendable {
    case pprMass
    case degree
}

public struct ProjectedAtomPosition: Equatable, Identifiable, Sendable {
    public var id: String
    public var x: Double
    public var y: Double
    public var z: Double
}

public struct ReprojectResult: Equatable, Sendable {
    public var projectionID: ProjectionID
    public var positions: [ProjectedAtomPosition]
}

public enum TheoremProjectionEngine {
    public static func availableProjections(for package: ScenePackageV2) -> [ProjectionAvailability] {
        ProjectionID.allCases.map { projection in
            let report = availability(of: projection, atoms: package.atoms, relations: package.relations)
            return ProjectionAvailability(id: projection, available: report.available, reason: report.reason)
        }
    }

    public static func centerNodeID(in package: ScenePackageV2, mode: CentralityMode) -> String? {
        switch mode {
        case .pprMass:
            package.atoms.max(by: compareByPPRMass)?.id
        case .degree:
            degreeMap(package.relations).max { left, right in
                if left.value == right.value {
                    return pprMass(for: atom(id: left.key, in: package.atoms)) < pprMass(for: atom(id: right.key, in: package.atoms))
                }
                return left.value < right.value
            }?.key ?? package.atoms.max(by: compareByPPRMass)?.id
        }
    }

    public static func reproject(_ package: ScenePackageV2, as projection: ProjectionID) throws -> ReprojectResult {
        let report = availability(of: projection, atoms: package.atoms, relations: package.relations)
        guard report.available else {
            throw ProjectionError.unavailable(report.reason)
        }

        let positions: [ProjectedAtomPosition]
        switch projection {
        case .forceGraph:
            positions = forcePositions(package.atoms)
        case .radialRings:
            positions = radialPositions(package.atoms)
        case .treeLayout:
            positions = try treePositions(package.atoms, relations: package.relations)
        case .fractalExpansion:
            positions = fractalPositions(package.atoms)
        }

        return ReprojectResult(projectionID: projection, positions: positions)
    }
}

public enum ProjectionError: Error, Equatable, LocalizedError {
    case unavailable(String)

    public var errorDescription: String? {
        switch self {
        case let .unavailable(reason):
            reason
        }
    }
}

private struct AvailabilityReport {
    var available: Bool
    var reason: String
}

private enum TreeReport {
    case valid([[String]])
    case invalid(String)
}

private extension TheoremProjectionEngine {
    static func availability(
        of projection: ProjectionID,
        atoms: [SceneAtom],
        relations: [SceneRelation]
    ) -> AvailabilityReport {
        switch projection {
        case .forceGraph:
            if atoms.count < 2 {
                return AvailabilityReport(available: false, reason: "needs at least two atoms")
            }
            if knownRelations(atoms: atoms, relations: relations).isEmpty {
                return AvailabilityReport(available: false, reason: "needs at least one relation")
            }
            return AvailabilityReport(available: true, reason: "relations define a graph")

        case .radialRings:
            let missingRing = atoms.filter { ring(for: $0) == nil }.count
            if missingRing > 0 {
                return AvailabilityReport(available: false, reason: "\(missingRing) atom(s) lack ring metadata")
            }
            return AvailabilityReport(available: true, reason: "atoms carry substrate ring metadata")

        case .treeLayout:
            switch treeReport(atoms: atoms, relations: relations) {
            case .valid:
                return AvailabilityReport(available: true, reason: "links form a rooted tree")
            case let .invalid(reason):
                return AvailabilityReport(available: false, reason: reason)
            }

        case .fractalExpansion:
            if knownRelations(atoms: atoms, relations: relations).isEmpty {
                return AvailabilityReport(available: false, reason: "needs graph relations")
            }
            if atoms.first(where: { ring(for: $0) == 0 }) == nil {
                return AvailabilityReport(available: false, reason: "needs a ring-0 match seed")
            }
            return AvailabilityReport(available: true, reason: "ring-0 seeds can replay push PPR")
        }
    }

    static func forcePositions(_ atoms: [SceneAtom]) -> [ProjectedAtomPosition] {
        rankedAtoms(atoms).enumerated().map { index, atom in
            let radius = 36.0 + sqrt(Double(index)) * 28.0
            let angle = Double(index) * .pi * (3.0 - sqrt(5.0))
            return ProjectedAtomPosition(
                id: atom.id,
                x: radius * cos(angle),
                y: radius * sin(angle),
                z: 0
            )
        }
    }

    static func radialPositions(_ atoms: [SceneAtom]) -> [ProjectedAtomPosition] {
        let grouped = Dictionary(grouping: atoms, by: { ring(for: $0) ?? 0 })
        return grouped.keys.sorted().flatMap { ringValue in
            let ringAtoms = rankedAtoms(grouped[ringValue] ?? [])
            let radius = ringValue == 0 ? 0.0 : Double(ringValue) * 96.0
            let count = max(ringAtoms.count, 1)
            return ringAtoms.enumerated().map { index, atom in
                let angle = radius == 0 ? 0 : Double(index) / Double(count) * 2.0 * .pi
                return ProjectedAtomPosition(
                    id: atom.id,
                    x: radius * cos(angle),
                    y: radius * sin(angle),
                    z: 0
                )
            }
        }
    }

    static func treePositions(_ atoms: [SceneAtom], relations: [SceneRelation]) throws -> [ProjectedAtomPosition] {
        switch treeReport(atoms: atoms, relations: relations) {
        case let .invalid(reason):
            throw ProjectionError.unavailable(reason)
        case let .valid(levels):
            return levels.enumerated().flatMap { depth, ids in
                let startX = -Double(max(ids.count - 1, 0)) * 58.0
                return ids.enumerated().map { index, id in
                    return ProjectedAtomPosition(
                        id: id,
                        x: startX + Double(index) * 116.0,
                        y: Double(depth) * 104.0,
                        z: 0
                    )
                }
            }
        }
    }

    static func fractalPositions(_ atoms: [SceneAtom]) -> [ProjectedAtomPosition] {
        let rankByID = Dictionary(uniqueKeysWithValues: rankedAtoms(atoms).enumerated().map { ($0.element.id, $0.offset) })
        return radialPositions(atoms).map { position in
            ProjectedAtomPosition(
                id: position.id,
                x: position.x,
                y: position.y,
                z: Double(rankByID[position.id] ?? 0)
            )
        }
    }

    static func treeReport(atoms: [SceneAtom], relations: [SceneRelation]) -> TreeReport {
        guard let root = atoms.max(by: compareByPPRMass) else {
            return .invalid("tree layout needs at least one atom")
        }
        guard atoms.count > 1 else {
            return .valid([[root.id]])
        }

        let ids = Set(atoms.map(\.id))
        let known = knownRelations(atoms: atoms, relations: relations)
        guard known.count == atoms.count - 1 else {
            return .invalid("tree layout needs exactly n-1 links")
        }

        var parentCount = Dictionary(uniqueKeysWithValues: ids.map { ($0, 0) })
        var children = Dictionary(uniqueKeysWithValues: ids.map { ($0, [String]()) })
        for relation in known {
            parentCount[relation.targetId, default: 0] += 1
            children[relation.sourceId, default: []].append(relation.targetId)
        }

        guard parentCount[root.id, default: 0] == 0 else {
            return .invalid("PPR center has an incoming link")
        }
        if let invalid = parentCount.first(where: { $0.key != root.id && $0.value != 1 }) {
            return .invalid("\(invalid.key) does not have exactly one parent")
        }

        var seen = Set<String>()
        var queue: [(String, Int)] = [(root.id, 0)]
        var levels = [[String]]()
        while let next = queue.first {
            queue.removeFirst()
            guard seen.insert(next.0).inserted else {
                return .invalid("links form a cycle or cross-edge")
            }
            while levels.count <= next.1 {
                levels.append([])
            }
            levels[next.1].append(next.0)
            queue.append(contentsOf: children[next.0, default: []].map { ($0, next.1 + 1) })
        }

        guard seen.count == atoms.count else {
            return .invalid("not every atom is reachable from the PPR center")
        }
        return .valid(levels)
    }

    static func rankedAtoms(_ atoms: [SceneAtom]) -> [SceneAtom] {
        atoms.sorted { left, right in
            if pprMass(for: left) == pprMass(for: right) {
                return left.id < right.id
            }
            return pprMass(for: left) > pprMass(for: right)
        }
    }

    static func compareByPPRMass(_ left: SceneAtom, _ right: SceneAtom) -> Bool {
        pprMass(for: left) < pprMass(for: right)
    }

    static func pprMass(for atom: SceneAtom?) -> Double {
        guard let atom else {
            return 0
        }
        return atom.metadata["matchScore"]?.doubleValue ?? atom.weight ?? 0
    }

    static func ring(for atom: SceneAtom) -> Int? {
        atom.metadata["ring"]?.intValue
    }

    static func atom(id: String, in atoms: [SceneAtom]) -> SceneAtom? {
        atoms.first { $0.id == id }
    }

    static func knownRelations(atoms: [SceneAtom], relations: [SceneRelation]) -> [SceneRelation] {
        let ids = Set(atoms.map(\.id))
        return relations.filter { ids.contains($0.sourceId) && ids.contains($0.targetId) }
    }

    static func degreeMap(_ relations: [SceneRelation]) -> [String: Int] {
        var degree = [String: Int]()
        for relation in relations {
            degree[relation.sourceId, default: 0] += 1
            degree[relation.targetId, default: 0] += 1
        }
        return degree
    }
}
