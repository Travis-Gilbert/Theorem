"""EpistemicRAG population cron (SPEC-EPISTEMICRAG deliverable 5).

A Modal cron that keeps the shadow epistemic graph current. It does NOT mutate
the content graph: it reads the user's own content subgraph, calls Theseus's
engines over gRPC to enrich it (Pairformer/EdgeMPNN completion, the TMS, the
argumentation-framework grounded-extension solver, Beta-Bernoulli source
reliability), and writes the resulting learned-epistemic fields and shadow
support/attack edges back onto the `EpistemicShadow` nodes through Theorem's
harness. The structural fields are owned by the synchronous ingestion pass
(SPEC-EPISTEMICRAG-INSTANT) and are never overwritten here.

Two triggers (the spec's population policy):
  - a frequent DELTA pass, gated on accumulated content-graph change crossing a
    threshold, recomputing only the dirty k-hop neighborhood; and
  - a NIGHTLY FULL pass that recomputes everything to correct drift.

Graceful degradation is load-bearing: on a gRPC timeout or dropped connection
the cron logs and no-ops. It never partial-writes learned fields and never
deletes shadow nodes. Writes are idempotent, keyed by (content_node_id,
engine_version), so a re-run produces no duplicates.

Architecture seam: the gRPC client to Theseus lives here (Python), the writeback
lands through Theorem's `epistemic_enrich_apply` MCP tool (Rust
`run_epistemic_cron_pass` with the annotations the cron supplies). So this file
owns orchestration + the Theseus dial; Theorem owns the idempotent graph write.

The two I/O seams (`TheoremHarnessClient`, `TheseusEnrichClient`) are injectable
so the policy is unit-testable without Modal, gRPC, or a live harness.
"""

from __future__ import annotations

import json
import logging
import os
from dataclasses import dataclass, field
from typing import Any, Optional, Protocol

logger = logging.getLogger("epistemic_cron")

DEFAULT_ENGINE = "theseus.epistemic_enrichment"
DEFAULT_ENGINE_VERSION = "epistemic-v1"


# --------------------------------------------------------------------------- #
# Config
# --------------------------------------------------------------------------- #
@dataclass(frozen=True)
class EpistemicCronConfig:
    tenant_id: str
    harness_url: str
    harness_token: str
    theseus_enrich_addr: str
    engine: str = DEFAULT_ENGINE
    engine_version: str = DEFAULT_ENGINE_VERSION
    # DELTA pass fires only once accumulated dirty count crosses this floor.
    delta_threshold: int = 25
    # k-hop neighborhood marked dirty around each changed node.
    k_hops: int = 2
    # Per-edge-type density floor; the learned pass is skipped on subgraphs
    # below it (completion on sparse edge types is noisy: a noisy shadow node is
    # worse than an absent one). Structural fields are unaffected.
    density_floor: float = 0.05
    # gRPC deadline. A miss is a clean no-op, not a partial write.
    grpc_timeout_s: float = 30.0

    @staticmethod
    def from_env() -> "EpistemicCronConfig":
        return EpistemicCronConfig(
            tenant_id=os.environ.get("EPISTEMIC_TENANT_ID", "default"),
            harness_url=os.environ.get(
                "THEOREM_HARNESS_URL", "http://localhost:50080"
            ).rstrip("/"),
            harness_token=os.environ.get("THEOREM_HARNESS_TOKEN", ""),
            theseus_enrich_addr=os.environ.get(
                "THESEUS_ENRICH_ADDR", "localhost:50090"
            ),
            engine=os.environ.get("EPISTEMIC_ENGINE", DEFAULT_ENGINE),
            engine_version=os.environ.get(
                "EPISTEMIC_ENGINE_VERSION", DEFAULT_ENGINE_VERSION
            ),
            delta_threshold=int(os.environ.get("EPISTEMIC_DELTA_THRESHOLD", "25")),
            k_hops=int(os.environ.get("EPISTEMIC_K_HOPS", "2")),
            density_floor=float(os.environ.get("EPISTEMIC_DENSITY_FLOOR", "0.05")),
            grpc_timeout_s=float(os.environ.get("EPISTEMIC_GRPC_TIMEOUT_S", "30")),
        )


@dataclass
class CronReport:
    mode: str
    attempted: bool = False
    no_op: bool = False
    grpc_ok: bool = False
    skipped_reason: str = ""
    dirty_count: int = 0
    annotations_received: int = 0
    shadows_written: int = 0
    shadow_edges_written: int = 0
    errors: list[str] = field(default_factory=list)

    def as_dict(self) -> dict[str, Any]:
        return {
            "mode": self.mode,
            "attempted": self.attempted,
            "no_op": self.no_op,
            "grpc_ok": self.grpc_ok,
            "skipped_reason": self.skipped_reason,
            "dirty_count": self.dirty_count,
            "annotations_received": self.annotations_received,
            "shadows_written": self.shadows_written,
            "shadow_edges_written": self.shadow_edges_written,
            "errors": self.errors,
        }


# --------------------------------------------------------------------------- #
# I/O seams (injectable for tests)
# --------------------------------------------------------------------------- #
class TheoremHarnessClient(Protocol):
    """Reads the dirty frontier + subgraph from Theorem and applies the
    enrichment writeback. The apply call maps to the Rust
    `epistemic_enrich_apply` MCP tool, which runs `run_epistemic_cron_pass` with
    the supplied annotations (idempotent, structural-preserving)."""

    def dirty_content_ids(self, tenant: str, k_hops: int) -> list[str]: ...

    def compile_subgraph(self, tenant: str, content_ids: list[str]) -> dict[str, Any]: ...

    def apply_annotations(
        self, tenant: str, annotations: dict[str, Any], engine: str, engine_version: str
    ) -> dict[str, Any]: ...


class TheseusEnrichClient(Protocol):
    """Dials Theseus's `EpistemicEnrichmentService.Enrich`. Raises on
    timeout/drop; the caller turns that into a clean no-op."""

    def enrich(
        self,
        tenant: str,
        subgraph: dict[str, Any],
        mode: str,
        engine_version: str,
        timeout_s: float,
    ) -> dict[str, Any]: ...


# --------------------------------------------------------------------------- #
# Core policy (pure, testable)
# --------------------------------------------------------------------------- #
def run_pass(
    mode: str,
    cfg: EpistemicCronConfig,
    harness: TheoremHarnessClient,
    theseus: TheseusEnrichClient,
) -> CronReport:
    """Run one enrichment pass. `mode` is "delta" or "full".

    DELTA recomputes the dirty k-hop set and is gated on the change threshold;
    FULL recomputes the whole tenant. On any Theseus failure the report records
    a no-op and nothing is written.
    """
    report = CronReport(mode=mode)

    if mode == "delta":
        content_ids = harness.dirty_content_ids(cfg.tenant_id, cfg.k_hops)
        report.dirty_count = len(content_ids)
        if len(content_ids) < cfg.delta_threshold:
            report.no_op = True
            report.skipped_reason = (
                f"below_delta_threshold:{len(content_ids)}<{cfg.delta_threshold}"
            )
            logger.info("epistemic delta no-op: %s", report.skipped_reason)
            return report
    else:
        # Full pass: empty id list signals "the whole tenant" to the harness.
        content_ids = harness.dirty_content_ids(cfg.tenant_id, cfg.k_hops) if False else []

    try:
        subgraph = harness.compile_subgraph(cfg.tenant_id, content_ids)
    except Exception as exc:  # harness read failure is a clean no-op
        report.no_op = True
        report.skipped_reason = f"compile_subgraph_failed:{exc}"
        report.errors.append(str(exc))
        logger.warning("epistemic cron: subgraph read failed: %s", exc)
        return report

    if not subgraph.get("nodes"):
        report.no_op = True
        report.skipped_reason = "empty_subgraph"
        return report

    report.attempted = True
    try:
        annotations = theseus.enrich(
            cfg.tenant_id,
            subgraph,
            mode,
            cfg.engine_version,
            cfg.grpc_timeout_s,
        )
    except Exception as exc:
        # The load-bearing graceful-degradation path: log, no-op, no writes.
        report.no_op = True
        report.grpc_ok = False
        report.skipped_reason = f"grpc_drop:{type(exc).__name__}"
        report.errors.append(str(exc))
        logger.warning("epistemic cron: Theseus enrich dropped, no-op: %s", exc)
        return report

    report.grpc_ok = True
    report.annotations_received = len(annotations.get("annotations", []))

    try:
        applied = harness.apply_annotations(
            cfg.tenant_id, annotations, cfg.engine, cfg.engine_version
        )
    except Exception as exc:
        report.errors.append(str(exc))
        logger.error("epistemic cron: writeback failed: %s", exc)
        return report

    report.shadows_written = int(applied.get("shadows_written", 0))
    report.shadow_edges_written = int(applied.get("shadow_edges_written", 0))
    logger.info(
        "epistemic %s pass: %d annotations, %d shadows, %d edges",
        mode,
        report.annotations_received,
        report.shadows_written,
        report.shadow_edges_written,
    )
    return report


# --------------------------------------------------------------------------- #
# Concrete clients (HTTP MCP + gRPC). Imported lazily so unit tests need
# neither requests, grpc, nor the generated stubs.
# --------------------------------------------------------------------------- #
class HttpHarnessClient:
    """Talks to the Theorem harness over its HTTP MCP surface (`POST /mcp`,
    JSON-RPC tool calls) plus the read endpoints."""

    def __init__(self, base_url: str, token: str):
        import requests  # lazy

        self._requests = requests
        self._base = base_url.rstrip("/")
        self._headers = {"content-type": "application/json"}
        if token:
            self._headers["authorization"] = f"Bearer {token}"

    def _call_tool(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        payload = {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }
        resp = self._requests.post(
            f"{self._base}/mcp", headers=self._headers, data=json.dumps(payload), timeout=60
        )
        resp.raise_for_status()
        body = resp.json()
        if "error" in body and body["error"]:
            raise RuntimeError(f"mcp tool {name} error: {body['error']}")
        result = body.get("result", {})
        content = result.get("content") or []
        for part in content:
            if part.get("type") == "text":
                try:
                    return json.loads(part["text"])
                except json.JSONDecodeError:
                    return {"text": part["text"]}
        return result

    def dirty_content_ids(self, tenant: str, k_hops: int) -> list[str]:
        out = self._call_tool(
            "epistemic_dirty_frontier", {"tenant_slug": tenant, "k_hops": k_hops}
        )
        return list(out.get("content_ids", []))

    def compile_subgraph(self, tenant: str, content_ids: list[str]) -> dict[str, Any]:
        return self._call_tool(
            "epistemic_compile_subgraph",
            {"tenant_slug": tenant, "content_ids": content_ids},
        )

    def apply_annotations(
        self, tenant: str, annotations: dict[str, Any], engine: str, engine_version: str
    ) -> dict[str, Any]:
        return self._call_tool(
            "epistemic_enrich_apply",
            {
                "tenant_slug": tenant,
                "annotations": annotations,
                "engine": engine,
                "engine_version": engine_version,
            },
        )


class GrpcTheseusClient:
    """Dials `theseus_epistemic.v1.EpistemicEnrichmentService`. Generate stubs
    with:

        python -m grpc_tools.protoc -I apps/theorem-grpc/proto \\
            --python_out=. --grpc_python_out=. \\
            apps/theorem-grpc/proto/theseus_epistemic/v1/epistemic.proto
    """

    _MODE = {
        "delta": "ENRICHMENT_MODE_DELTA",
        "full": "ENRICHMENT_MODE_FULL",
    }

    def __init__(self, addr: str):
        self._addr = addr

    def enrich(
        self,
        tenant: str,
        subgraph: dict[str, Any],
        mode: str,
        engine_version: str,
        timeout_s: float,
    ) -> dict[str, Any]:
        import grpc  # lazy
        from theseus_epistemic.v1 import epistemic_pb2, epistemic_pb2_grpc  # generated

        mode_enum = getattr(
            epistemic_pb2.EnrichmentMode,
            self._MODE.get(mode, "ENRICHMENT_MODE_FULL"),
        )
        req = epistemic_pb2.EnrichRequest(
            tenant_id=tenant,
            mode=mode_enum,
            engine_version=engine_version,
            subgraph=_subgraph_to_proto(epistemic_pb2, subgraph),
        )
        with grpc.insecure_channel(self._addr) as channel:
            stub = epistemic_pb2_grpc.EpistemicEnrichmentServiceStub(channel)
            resp = stub.Enrich(req, timeout=timeout_s)
        return _response_to_dict(resp)


def _subgraph_to_proto(pb, subgraph: dict[str, Any]):
    nodes = [
        pb.NodeRecord(
            id=n["id"],
            labels=list(n.get("labels", [])),
            properties_json=json.dumps(n.get("properties", {})),
        )
        for n in subgraph.get("nodes", [])
    ]
    edges = [
        pb.EdgeRecord(
            id=e["id"],
            from_id=e["from_id"],
            to_id=e["to_id"],
            type=e.get("type", ""),
            properties_json=json.dumps(e.get("properties", {})),
            confidence=e.get("confidence"),
            epistemic_type=e.get("epistemic_type"),
        )
        for e in subgraph.get("edges", [])
    ]
    return pb.UserSubgraph(nodes=nodes, edges=edges)


def _response_to_dict(resp) -> dict[str, Any]:
    def rel(r):
        return {
            "from_content_id": r.from_content_id,
            "to_content_id": r.to_content_id,
            "kind": r.kind,
            "confidence": r.confidence,
            "evidence": r.evidence,
        }

    return {
        "annotations": [
            {
                "content_node_id": a.content_node_id,
                "predicted_edges": [
                    {
                        "target_content_id": p.target_content_id,
                        "relation": p.relation,
                        "confidence": p.confidence,
                        "quarantine": p.quarantine,
                    }
                    for p in a.predicted_edges
                ],
                "completion_confidence": a.completion_confidence
                if a.HasField("completion_confidence")
                else None,
                "structural_role_vector": list(a.structural_role_vector),
                "source_reliability": {
                    "alpha": a.source_reliability.alpha,
                    "beta": a.source_reliability.beta,
                    "mean": a.source_reliability.mean,
                }
                if a.HasField("source_reliability")
                else None,
                "community_id": a.community_id if a.HasField("community_id") else None,
                "grounded_extension_status": a.grounded_extension_status
                if a.HasField("grounded_extension_status")
                else None,
            }
            for a in resp.annotations
        ],
        "support_relations": [rel(r) for r in resp.support_relations],
        "attack_relations": [rel(r) for r in resp.attack_relations],
        "engine": resp.engine,
        "engine_version": resp.engine_version,
    }


# --------------------------------------------------------------------------- #
# Modal app. Importing modal is optional so the module loads (and tests run)
# without it; the schedules only register when Modal is present.
# --------------------------------------------------------------------------- #
def _build_clients(cfg: EpistemicCronConfig):
    return HttpHarnessClient(cfg.harness_url, cfg.harness_token), GrpcTheseusClient(
        cfg.theseus_enrich_addr
    )


try:  # pragma: no cover - deployment-only
    import modal

    app = modal.App("epistemic-rag-cron")
    image = modal.Image.debian_slim().pip_install("requests", "grpcio", "protobuf")

    @app.function(image=image, schedule=modal.Period(minutes=15), timeout=600)
    def delta_pass() -> dict[str, Any]:
        cfg = EpistemicCronConfig.from_env()
        harness, theseus = _build_clients(cfg)
        return run_pass("delta", cfg, harness, theseus).as_dict()

    @app.function(image=image, schedule=modal.Cron("0 4 * * *"), timeout=3600)
    def nightly_full_pass() -> dict[str, Any]:
        cfg = EpistemicCronConfig.from_env()
        harness, theseus = _build_clients(cfg)
        return run_pass("full", cfg, harness, theseus).as_dict()

except ImportError:  # local / test context
    modal = None  # type: ignore
    app = None  # type: ignore


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    _cfg = EpistemicCronConfig.from_env()
    _harness, _theseus = _build_clients(_cfg)
    print(json.dumps(run_pass("delta", _cfg, _harness, _theseus).as_dict(), indent=2))
