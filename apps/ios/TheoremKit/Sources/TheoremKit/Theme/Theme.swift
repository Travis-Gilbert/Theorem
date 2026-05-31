import Foundation

/// Semantic color roles. Color encodes meaning in the graph (node kind, ring),
/// so the theme is a set of ROLES, not raw hex (spec "Theming"). The user picks
/// colors; colors fill roles; renderers read roles. Meaning survives any palette
/// because the role -> color indirection is fixed.
public enum ColorRole: String, CaseIterable, Sendable {
    case nodeCore       // engine core knowledge (amber)
    case nodeWeb        // substrate / web pages (teal)
    case nodeTool       // tooling / methods (purple)
    case nodeDimmed     // de-emphasized (gray)
    case edge           // relations
    case ringMatch      // ring 0 — direct match (hot accent)
    case ringAdjacent   // ring 1
    case ringNearby     // ring 2+
    case background     // app ground
    case surface        // cards / panels
    case textPrimary
    case textSecondary
}

/// A named palette: role -> color. Renderers resolve colors through `color(_:)`
/// so a node's hue follows its role, not a hardcoded literal.
public struct Theme: Hashable, Sendable {
    public var name: String
    public var colors: [ColorRole: RGBAColor]

    public init(name: String, colors: [ColorRole: RGBAColor]) {
        self.name = name
        self.colors = colors
    }

    /// Resolve a role to a color, falling back visibly when a palette omits one.
    public func color(_ role: ColorRole) -> RGBAColor {
        colors[role] ?? .fallback
    }

    /// Map an atom kind to a node color role. The substrate's kinds (source,
    /// claim, concept, method, person, ...) collapse onto the four node roles.
    /// Overridable by replacing the palette; this is the default semantics.
    public func role(forKind kind: String) -> ColorRole {
        switch kind.lowercased() {
        case "source", "page", "web", "url", "document", "link":
            return .nodeWeb
        case "method", "tool", "skill", "agent", "model", "process":
            return .nodeTool
        case "claim", "concept", "evidence", "idea", "hunch", "note", "core":
            return .nodeCore
        default:
            return .nodeWeb
        }
    }

    /// Map a ring (hop distance from match) to a ring color role.
    public func role(forRing ring: Int) -> ColorRole {
        switch ring {
        case 0: return .ringMatch
        case 1: return .ringAdjacent
        default: return .ringNearby
        }
    }

    /// Color for an atom: ring color when the atom carries a ring (the
    /// search-derived path, where hop-distance-from-match is the salient
    /// signal), else its kind color.
    public func colorForAtom(kind: String, ring: Int?) -> RGBAColor {
        if let ring { return color(role(forRing: ring)) }
        return color(role(forKind: kind))
    }
}

public extension Theme {
    /// The default Theorem palette (spec "Theming" + the mockups): teal for
    /// substrate/web, terracotta/coral accent for the hottest ring, amber for
    /// engine core, purple for tooling, gray for dimmed, over a warm-dark ground
    /// (graph viz reads best on dark). Hexes are non-nil by construction.
    static let theorem = Theme(name: "Theorem", colors: [
        .nodeCore:      RGBAColor(hex: "#D9A441")!,   // amber — engine core
        .nodeWeb:       RGBAColor(hex: "#2D8C9E")!,   // teal — substrate/web
        .nodeTool:      RGBAColor(hex: "#7A6BA8")!,   // purple — tooling
        .nodeDimmed:    RGBAColor(hex: "#8A8782")!,   // gray — dimmed
        .edge:          RGBAColor(hex: "#5A6B72")!,   // muted ink
        .ringMatch:     RGBAColor(hex: "#C75D38")!,   // terracotta — direct match
        .ringAdjacent:  RGBAColor(hex: "#C08A5A")!,   // warm mid
        .ringNearby:    RGBAColor(hex: "#9FB0A8")!,   // cool fade
        .background:    RGBAColor(hex: "#16161A")!,   // warm-dark ground
        .surface:       RGBAColor(hex: "#212129")!,   // panel
        .textPrimary:   RGBAColor(hex: "#F4F1EA")!,   // cream
        .textSecondary: RGBAColor(hex: "#A8A29A")!,   // muted cream
    ])
}
