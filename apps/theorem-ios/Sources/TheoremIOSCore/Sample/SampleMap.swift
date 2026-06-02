import Foundation

/// A recorded codebase map for the Maps surface (harness UI spec, Part 3, the
/// proven first case). The entries are real orientation facts about the Theorem
/// repo (its CLAUDE.md navigation rules), not invented filler: where the boundary
/// is, what to read first, what to avoid, how to verify. When the map compiler
/// ports to Rust and a transport ships, a live `MapArtifact` renders through the
/// same view.
public enum SampleMap {
    public static let theoremCodebase = MapArtifact(
        id: "map:codebase:theorem",
        mapKind: "CodebaseMap",
        scopeKind: "repo",
        scopeRef: "Theorem",
        entries: [
            MapEntry(
                id: "repo-boundary", kind: "repo", title: "Repo boundary",
                summary: "Theorem is the Rust-native substrate spine: the Rust projection that mirrors the Theseus / Index-API workspace."
            ),
            MapEntry(
                id: "task-target", kind: "target", title: "Task target",
                summary: "Port the harness to Rust (theorem-harness-core/runtime) and build the iOS run surfaces over its contract."
            ),
            MapEntry(
                id: "read-first:1", kind: "read_first", title: "CLAUDE.md",
                summary: "The navigation truth for the repo. Read it first: layout, the mirror rule, the crate map, the gotchas."
            ),
            MapEntry(
                id: "read-first:2", kind: "read_first", title: "docs/plans/harness-rust-port/",
                summary: "The port plan, the parity corpora (kernel, toolgraph, context), and the transport handoff."
            ),
            MapEntry(
                id: "read-first:3", kind: "read_first", title: "theorem-harness-core",
                summary: "The kernel: run state machine, events, replay/fork, toolgraph. Parity-green against the Python reference."
            ),
            MapEntry(
                id: "do-not:1", kind: "do_not", title: "Workspace members",
                summary: "Don't add reconstruction-engine to the workspace members; it path-deps another repo, and CI must stay uncoupled."
            ),
            MapEntry(
                id: "do-not:2", kind: "do_not", title: "Commits on a shared tree",
                summary: "Don't bare-commit; Codex is active here. Path-scoped commits only, never sweep another agent's staged files."
            ),
            MapEntry(
                id: "focused-validators", kind: "validators", title: "Focused validators",
                summary: "cargo test -p theorem-harness-core; swift build --package-path apps/theorem-ios."
            ),
            MapEntry(
                id: "rustyred-thg-mcp", kind: "tool", title: "rustyred-thg-mcp",
                summary: "The substrate-as-MCP surface: graph reads and algorithms without a Python process in the loop."
            ),
        ]
    )
}
