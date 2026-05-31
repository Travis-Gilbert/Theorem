import Foundation
import TheoremKit

// Runnable verification harness for TheoremKit.
//
// The standalone command-line-tools toolchain bundles neither `Testing` nor
// `XCTest` (both ship with full Xcode), so this executable IS the test runner
// that actually executes on a CLT-only machine: it runs the checks and exits
// non-zero on any failure. When Xcode is present, the same assertions live in
// Tests/TheoremKitTests as an XCTest suite run via `swift test` / XcodeBuildMCP.

var failures = 0
var passed = 0

@MainActor func check(_ condition: Bool, _ label: String) {
    if condition {
        passed += 1
    } else {
        failures += 1
        FileHandle.standardError.write(Data("FAIL  \(label)\n".utf8))
    }
}

@MainActor func section(_ name: String) { print("• \(name)") }

// ---------------------------------------------------------------------------
// SubstrateSearch (snake_case)
// ---------------------------------------------------------------------------
section("SubstrateSearch (snake_case wire)")
do {
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
    check(search.query == "knowledge graph", "query")
    check(search.hits.count == 2, "hits count")
    check(search.hits[0].nodeID == "p1", "node_id -> nodeID")
    check(search.hits[0].matchScore == 0.83, "match_score -> matchScore")
    check(search.hits[0].ringLabel == "match", "ring_label -> ringLabel")
    check(search.hits[1].snippet == "", "empty snippet")
    check(search.links == [SearchLink(source: "p1", target: "p2")], "links")
    check(search.matchedCount == 1, "matched_count -> matchedCount")
    check(search.keptCount == 2, "kept_count -> keptCount")
} catch {
    check(false, "SubstrateSearch decode threw: \(error)")
}

// ---------------------------------------------------------------------------
// ScenePackageV2 (camelCase, kebab enums, omitted-empty fields)
// ---------------------------------------------------------------------------
section("ScenePackageV2 (camelCase wire, omitted-empty fields)")
do {
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
    check(pkg.version == "scene-package-v2", "version")
    check(pkg.manifestRef == "manifest-1", "manifestRef")
    check(pkg.atoms.count == 2, "atoms count")
    check(pkg.projection.id == "patent_diagram", "projection.id")
    check(pkg.projection.params.isEmpty, "omitted params -> empty")
    check(pkg.chrome.id == "patent_plate_shell", "chrome.id")
    check(pkg.actions.isEmpty, "omitted actions -> empty")
    check(pkg.transitions == nil, "omitted transitions -> nil")
    check(pkg.terminalState == nil, "omitted terminalState -> nil")

    let a = pkg.atoms[0]
    check(a.kind == "evidence", "atom.kind")
    check(a.lifecycle == .present, "kebab lifecycle -> .present")
    check(a.position == nil, "omitted position -> nil")
    check(a.metadata.isEmpty, "omitted metadata -> empty")
    check(a.sourceRefs.count == 1, "sourceRefs count")
    check(a.sourceRefs[0].id == "42", "sourceRef.id")
    check(a.sourceRefs[0].metadata["score"]?.doubleValue == 0.91, "nested metadata number")

    let b = pkg.atoms[1]
    check(b.position?.space == .graph, "kebab space -> .graph")
    check(b.position?.x == 10.0, "position.x")
    check(b.ring == 2, "ring read from metadata")
    check(b.magnitude == 0.4, "magnitude from metadata match_score")

    let rel = pkg.relations[0]
    check(rel.sourceId == "a", "sourceId")
    check(rel.targetId == "b", "targetId")
    check(rel.kind == "supports", "relation.kind")
} catch {
    check(false, "ScenePackageV2 decode threw: \(error)")
}

// ---------------------------------------------------------------------------
// Minimal atom + round-trip
// ---------------------------------------------------------------------------
section("Minimal atom + round-trip")
do {
    let json = #"{"id":"x","kind":"evidence","lifecycle":"present"}"#.data(using: .utf8)!
    let atom = try JSONDecoder().decode(SceneAtom.self, from: json)
    check(atom.id == "x", "minimal atom id")
    check(atom.position == nil && atom.metadata.isEmpty && atom.sourceRefs.isEmpty,
          "minimal atom defaults")
    check(atom.magnitude == 1, "magnitude default 1")
    check(atom.ring == nil, "ring nil without metadata")

    let pkg = ScenePackageV2(
        id: "pkg-2", manifestRef: "m2",
        atoms: [SceneAtom(id: "a", kind: "source", label: "Alpha",
                          position: AtomPosition(x: 1, y: 2, space: .graph),
                          weight: 3)],
        relations: [SceneRelation(id: "a->a", sourceId: "a", targetId: "a", kind: "self")],
        projection: ProjectionBinding(id: "force_graph"),
        chrome: ChromeBinding(id: "document_rail"))
    let data = try JSONEncoder().encode(pkg)
    let back = try ScenePackageV2.decode(from: data)
    check(back == pkg, "ScenePackageV2 round-trips")
} catch {
    check(false, "minimal/round-trip threw: \(error)")
}

// ---------------------------------------------------------------------------
// Theme: hex parsing, role resolution, palette completeness
// ---------------------------------------------------------------------------
section("Theme (roles, palette, hex)")
do {
    check(RGBAColor(hex: "#2D8C9E") != nil, "parses #RRGGBB")
    check(RGBAColor(hex: "fff") != nil, "parses #RGB (no hash)")
    check(RGBAColor(hex: "#11223344")?.alpha == Double(0x44) / 255.0, "parses #RRGGBBAA alpha")
    check(RGBAColor(hex: "nope") == nil, "rejects non-hex")

    let theme = Theme.theorem
    // Every role must have a palette entry (no silent fallback magenta in prod).
    let missing = ColorRole.allCases.filter { theme.colors[$0] == nil }
    check(missing.isEmpty, "default palette covers all \(ColorRole.allCases.count) roles")

    // Kind -> role semantics.
    check(theme.role(forKind: "source") == .nodeWeb, "source -> nodeWeb")
    check(theme.role(forKind: "method") == .nodeTool, "method -> nodeTool")
    check(theme.role(forKind: "claim") == .nodeCore, "claim -> nodeCore")
    // Ring -> role semantics.
    check(theme.role(forRing: 0) == .ringMatch, "ring 0 -> ringMatch")
    check(theme.role(forRing: 1) == .ringAdjacent, "ring 1 -> ringAdjacent")
    check(theme.role(forRing: 5) == .ringNearby, "ring 5 -> ringNearby")
    // Atom color prefers ring when present.
    check(theme.colorForAtom(kind: "source", ring: 0) == theme.color(.ringMatch),
          "atom with ring uses ring color")
    check(theme.colorForAtom(kind: "method", ring: nil) == theme.color(.nodeTool),
          "atom without ring uses kind color")
}

// ---------------------------------------------------------------------------
// Bundled display fonts (Berthold Akzidenz-Grotesk) register from the module
// ---------------------------------------------------------------------------
section("Bundled display fonts (Berthold Akzidenz-Grotesk)")
let registered = TheoremTypography.registerBundledFonts()
check(registered.contains(TheoremTypography.displayRegularName),
      "registered \(TheoremTypography.displayRegularName)")
check(registered.contains(TheoremTypography.displayMediumName),
      "registered \(TheoremTypography.displayMediumName)")

// ---------------------------------------------------------------------------
// Reprojection sliver: honest-shape rule (the spec differentiator)
// ---------------------------------------------------------------------------
func atom(_ id: String, ring: Int? = nil, score: Double? = nil, kind: String = "source") -> SceneAtom {
    var meta: [String: JSONValue] = [:]
    if let ring { meta["ring"] = .number(Double(ring)) }
    if let score { meta["match_score"] = .number(score) }
    return SceneAtom(id: id, kind: kind, metadata: meta)
}
func rel(_ s: String, _ t: String) -> SceneRelation {
    SceneRelation(id: "\(s)->\(t)", sourceId: s, targetId: t)
}
func makeScene(_ atoms: [SceneAtom], _ rels: [SceneRelation]) -> ScenePackageV2 {
    ScenePackageV2(id: "t", manifestRef: "m", atoms: atoms, relations: rels,
                   projection: ProjectionBinding(id: "force_graph"),
                   chrome: ChromeBinding(id: "document_rail"))
}

let engine = StubReprojectionEngine()

section("Reprojection — valid tree scene")
do {
    // a is the match (ring 0, top score), star/tree beneath it. Acyclic.
    let tree = makeScene(
        [atom("a", ring: 0, score: 0.9), atom("b", ring: 1, score: 0.5),
         atom("c", ring: 1, score: 0.4), atom("d", ring: 2, score: 0.2)],
        [rel("a", "b"), rel("a", "c"), rel("b", "d")])
    let avail = Dictionary(uniqueKeysWithValues:
        engine.availableProjections(tree).map { ($0.projectionID, $0) })
    check(avail[ProjectionID.forceGraph]?.available == true, "force_graph available")
    check(avail[ProjectionID.radialRings]?.available == true, "radial available (rings present)")
    check(avail[ProjectionID.treeLayout]?.available == true, "tree available (acyclic)")
    check(avail[ProjectionID.fractalExpansion]?.available == true, "fractal available")

    check(engine.centerNodeID(tree, mode: .pprMass) == "a", "center by PPR mass = a")
    check(engine.centerNodeID(tree, mode: .degree) == "a", "center by degree = a (star hub)")

    let treeResult = try engine.reproject(tree, projectionID: ProjectionID.treeLayout)
    check(treeResult.positions.count == 4, "tree lays out all atoms")
    check(treeResult.positions["a"]?.y == 0, "tree root a at depth 0 (y=0)")
    check(treeResult.coordinateSpace == .diagram, "tree space = diagram")

    let radialResult = try engine.reproject(tree, projectionID: ProjectionID.radialRings)
    check(radialResult.positions["a"] == LayoutPoint(x: 0, y: 0), "radial: lone ring-0 at center")
    check(radialResult.positions.count == 4, "radial lays out all atoms")
} catch {
    check(false, "valid-tree reprojection threw: \(error)")
}

section("Reprojection — cyclic scene greys out tree")
do {
    // x -> y -> z -> x : a cycle. No ring metadata.
    let cyclic = makeScene(
        [atom("x", score: 0.8), atom("y", score: 0.6), atom("z", score: 0.5)],
        [rel("x", "y"), rel("y", "z"), rel("z", "x")])
    let avail = Dictionary(uniqueKeysWithValues:
        engine.availableProjections(cyclic).map { ($0.projectionID, $0) })
    check(avail[ProjectionID.treeLayout]?.available == false, "tree UNavailable on a cycle")
    check(avail[ProjectionID.treeLayout]?.reason?.contains("cycle") == true,
          "tree reason names the cycle")
    check(avail[ProjectionID.forceGraph]?.available == true, "force_graph still available")
    check(avail[ProjectionID.radialRings]?.available == false, "radial UNavailable (no rings)")

    // reproject must REFUSE to fabricate a hierarchy the data lacks.
    var threw = false
    do { _ = try engine.reproject(cyclic, projectionID: ProjectionID.treeLayout) }
    catch ReprojectError.shapeRejected(let pid, let reason) {
        threw = true
        check(pid == ProjectionID.treeLayout, "rejection names tree projection")
        check(reason.contains("cycle"), "rejection reason names the cycle")
    } catch { check(false, "wrong error type: \(error)") }
    check(threw, "reproject(tree) on a cycle throws shapeRejected")

    // ...but the always-available force graph lays out fine.
    let forced = try engine.reproject(cyclic, projectionID: ProjectionID.forceGraph)
    check(forced.positions.count == 3, "force seed lays out the cyclic scene")
} catch {
    check(false, "cyclic reprojection section threw: \(error)")
}

section("Reprojection — guards")
do {
    let one = makeScene([atom("solo")], [])
    let availSolo = Dictionary(uniqueKeysWithValues:
        engine.availableProjections(one).map { ($0.projectionID, $0.available) })
    check(availSolo[ProjectionID.forceGraph] == false, "force_graph needs >= 2 nodes + a link")

    var unknownThrew = false
    do { _ = try engine.reproject(makeScene([atom("a"), atom("b")], [rel("a", "b")]),
                                  projectionID: "made_up") }
    catch ReprojectError.unknownProjection { unknownThrew = true } catch {}
    check(unknownThrew, "unknown projection id throws")

    var emptyThrew = false
    do { _ = try engine.reproject(makeScene([], []), projectionID: ProjectionID.forceGraph) }
    catch ReprojectError.emptyScene { emptyThrew = true } catch {}
    check(emptyThrew, "empty scene throws emptyScene")

    // Self-loop is not a tree.
    let selfLoop = makeScene([atom("a"), atom("b")], [rel("a", "a"), rel("a", "b")])
    let treeAvail = engine.availableProjections(selfLoop)
        .first { $0.projectionID == ProjectionID.treeLayout }?.available
    check(treeAvail == false, "self-loop greys out tree")
}

// ---------------------------------------------------------------------------
print("")
if failures > 0 {
    print("\(passed) passed, \(failures) FAILED")
    exit(1)
}
print("\(passed) checks passed")
