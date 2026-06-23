# Integrations Decision

**Phase:** 6, baseline closure
**Status:** MCP-first, with one proof connector path

## Recommendation

Prefer native MCP connector affordances over an OAuth catalog build for v1. The harness already treats MCP tools as learnable affordances through `tools/list`, tool invocation, and affordance outcome recording. A catalog can be useful later for discovery, consent screens, and account UX, but it is not required to prove the substrate.

## Why MCP First

- It keeps integrations on the same tool surface used by hosted and local harness targets.
- It avoids building account, OAuth, and catalog administration before the desktop baseline is stable.
- It allows proof connectors to be registered and exercised as affordances without adding a parallel integration runtime.

## What A Catalog Buys

- User-facing browsing of available services.
- OAuth lifecycle management.
- Standard consent and revocation flows.

## What It Costs

- A new account and permissions surface.
- More secrets handling before the keychain and local node are fully proven.
- A second registry beside the harness affordance graph.

## Proof Connector

The desktop backend exposes `connector_proof_run`, which invokes the existing `code_search` MCP affordance path (`theorem_grpc.code_search.search`) from the rail/settings surface. Success records that a connector-style affordance can be called from desktop without building a catalog.
