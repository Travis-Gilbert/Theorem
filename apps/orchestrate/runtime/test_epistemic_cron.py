"""Unit tests for the EpistemicRAG cron policy (no Modal, gRPC, or harness).

Proves the load-bearing behaviors: the delta threshold gate, the graceful
gRPC-drop no-op (no writeback attempted), and the happy path writeback.
Run: python -m pytest apps/orchestrate/runtime/test_epistemic_cron.py
"""

from __future__ import annotations

import epistemic_cron as ec


def _cfg(**kw):
    base = dict(
        tenant_id="t",
        harness_url="http://h",
        harness_token="",
        theseus_enrich_addr="a:1",
        delta_threshold=5,
    )
    base.update(kw)
    return ec.EpistemicCronConfig(**base)


class FakeHarness:
    def __init__(self, dirty, subgraph):
        self._dirty = dirty
        self._subgraph = subgraph
        self.applied = None

    def dirty_content_ids(self, tenant, k_hops):
        return list(self._dirty)

    def compile_subgraph(self, tenant, content_ids):
        return self._subgraph

    def apply_annotations(self, tenant, annotations, engine, engine_version):
        self.applied = (annotations, engine, engine_version)
        return {
            "shadows_written": len(annotations.get("annotations", [])),
            "shadow_edges_written": 0,
        }


class OkTheseus:
    def enrich(self, tenant, subgraph, mode, engine_version, timeout_s):
        return {"annotations": [{"content_node_id": "a"}], "support_relations": [], "attack_relations": []}


class DroppingTheseus:
    def enrich(self, tenant, subgraph, mode, engine_version, timeout_s):
        raise TimeoutError("deadline exceeded")


def test_delta_below_threshold_is_noop():
    cfg = _cfg(delta_threshold=10)
    harness = FakeHarness(dirty=["x", "y"], subgraph={"nodes": [{"id": "x"}], "edges": []})
    report = ec.run_pass("delta", cfg, harness, OkTheseus())
    assert report.no_op
    assert "below_delta_threshold" in report.skipped_reason
    assert harness.applied is None  # nothing written


def test_grpc_drop_is_clean_noop_without_writeback():
    cfg = _cfg(delta_threshold=1)
    harness = FakeHarness(dirty=["a", "b"], subgraph={"nodes": [{"id": "a"}], "edges": []})
    report = ec.run_pass("delta", cfg, harness, DroppingTheseus())
    assert report.attempted
    assert report.no_op
    assert not report.grpc_ok
    assert report.skipped_reason.startswith("grpc_drop:")
    assert harness.applied is None  # the key guarantee: no partial writeback


def test_happy_path_writes_back():
    cfg = _cfg(delta_threshold=1)
    harness = FakeHarness(dirty=["a"], subgraph={"nodes": [{"id": "a"}], "edges": []})
    report = ec.run_pass("delta", cfg, harness, OkTheseus())
    assert report.grpc_ok
    assert report.annotations_received == 1
    assert report.shadows_written == 1
    assert harness.applied is not None


def test_full_pass_ignores_threshold():
    cfg = _cfg(delta_threshold=1000)
    harness = FakeHarness(dirty=[], subgraph={"nodes": [{"id": "a"}], "edges": []})
    report = ec.run_pass("full", cfg, harness, OkTheseus())
    assert not report.no_op
    assert report.shadows_written == 1
