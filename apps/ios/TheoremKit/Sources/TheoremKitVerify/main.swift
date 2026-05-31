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
print("")
if failures > 0 {
    print("\(passed) passed, \(failures) FAILED")
    exit(1)
}
print("\(passed) checks passed")
