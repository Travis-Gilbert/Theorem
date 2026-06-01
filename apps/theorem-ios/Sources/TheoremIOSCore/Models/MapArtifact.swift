import Foundation

/// A MapArtifact (harness UI spec, Part 3): an orientation primitive, a peer to
/// the context artifact. A map answers "where am I and what matters here", not
/// "list everything". It mirrors the runtime's `MapArtifactState`: a kind
/// (CodebaseMap, RuleMap, ...), a scope, and a list of entries whose `kind`
/// encodes the orientation role (read first, do not, boundary, verify, tool).
///
/// The UI renders the map by ROLE, not as a file tree: the structure that
/// matters, what to trust (read first), what not to trust (avoid), the rules,
/// how to verify. That grouping is what makes it orientation rather than a dump.

/// The orientation role an entry plays, derived from its `kind`. This is the
/// grouping the map view renders.
public enum MapEntryRole: String, Sendable, CaseIterable {
    case structure
    case readFirst
    case avoid
    case rules
    case verify
    case tools
    case note

    public init(forEntryKind kind: String) {
        switch kind {
        case "repo", "target", "repo_boundary": self = .structure
        case "read_first": self = .readFirst
        case "do_not": self = .avoid
        case "risk_mode", "policy", "active_policy", "recall_policy": self = .rules
        case "validators": self = .verify
        case "tool": self = .tools
        default: self = .note
        }
    }

    /// Section label (eyebrow), uppercase.
    public var label: String {
        switch self {
        case .structure: "BOUNDARY"
        case .readFirst: "READ FIRST"
        case .avoid: "AVOID"
        case .rules: "RULES"
        case .verify: "VERIFY"
        case .tools: "TOOLS"
        case .note: "NOTES"
        }
    }

    /// Display order: orient (where am I, what to read) before caveats and tools.
    public var order: Int {
        switch self {
        case .structure: 0
        case .readFirst: 1
        case .avoid: 2
        case .rules: 3
        case .verify: 4
        case .tools: 5
        case .note: 6
        }
    }
}

public struct MapEntry: Identifiable, Equatable, Sendable {
    public let id: String
    public let kind: String
    public let title: String
    public let summary: String

    public init(id: String, kind: String, title: String, summary: String) {
        self.id = id
        self.kind = kind
        self.title = title
        self.summary = summary
    }

    public var role: MapEntryRole { MapEntryRole(forEntryKind: kind) }
}

public struct MapArtifact: Identifiable, Equatable, Sendable {
    public let id: String
    /// The map kind, e.g. `CodebaseMap`.
    public let mapKind: String
    public let scopeKind: String
    public let scopeRef: String
    public let entries: [MapEntry]

    public init(id: String, mapKind: String, scopeKind: String, scopeRef: String, entries: [MapEntry]) {
        self.id = id
        self.mapKind = mapKind
        self.scopeKind = scopeKind
        self.scopeRef = scopeRef
        self.entries = entries
    }

    /// A readable name for the map, e.g. "Codebase · Theorem".
    public var title: String {
        let kind = mapKind
            .replacingOccurrences(of: "Map", with: "")
            .replacingOccurrences(of: "_", with: " ")
        let scope = scopeRef.isEmpty ? scopeKind : scopeRef
        return scope.isEmpty ? kind : "\(kind) · \(scope)"
    }

    /// Entries grouped by orientation role, in display order, skipping empty
    /// roles.
    public var sections: [(role: MapEntryRole, entries: [MapEntry])] {
        MapEntryRole.allCases
            .sorted { $0.order < $1.order }
            .compactMap { role in
                let matching = entries.filter { $0.role == role }
                return matching.isEmpty ? nil : (role, matching)
            }
    }
}
