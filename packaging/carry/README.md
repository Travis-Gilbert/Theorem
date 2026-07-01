# Theorem Carry

Theorem Carry is the local, no-account memory wedge for Claude Code and Codex.
It registers lifecycle hooks that write session observations to the local
RustyRed MCP node and read a bounded memory capsule at session start.

Install from a checkout:

```bash
packaging/carry/install.sh install
```

Or through the product wrapper:

```bash
theorem carry install
theorem carry up
```

Uninstall:

```bash
packaging/carry/install.sh uninstall
```

The managed hook commands are marked with `THEOREM_CARRY_MANAGED=1` and are
removed without touching unrelated user hooks. The hook path requires no tenant,
token, or hosted network service; by default it targets
`http://127.0.0.1:8380/mcp`.
