#!/usr/bin/env python3
"""RunPod Serverless worker for Theorem code compiler burst jobs."""

from __future__ import annotations

import hashlib
import json
import math
import os
import sys
import time
from pathlib import Path
from typing import Any


FEATURE_VERSION = "code-compiler-ir-v1"
WORKER_VERSION = "theorem-code-compiler-worker-v1"
DEFAULT_MODEL_ID = "sentence-transformers/all-MiniLM-L6-v2"
FEATURE_NAMES = [
    "jaccard_coefficient",
    "bm25_score",
    "sbert_cosine",
    "tfidf_similarity",
    "shared_entity_count",
    "nli_entailment_score",
    "nli_contradiction_score",
    "kge_prediction_score",
    "same_object_type",
    "same_notebook",
    "time_gap_days",
    "shared_cluster",
    "gnn_structural_score",
    "gnn_edge_prediction",
    "rule_support_count",
    "rule_net_score",
    "source_entrenchment",
    "target_entrenchment",
    "deep_analogy_score",
    "rnd_novelty_score",
    "spacetime_temporal_score",
]

_EMBEDDER: Any | None = None
_EMBEDDER_ID = ""


def handler(job: dict[str, Any]) -> dict[str, Any]:
    started = time.time()
    job_input = job.get("input") or {}
    if not isinstance(job_input, dict):
        return {"error": "job input must be an object"}

    try:
        tenant_id = non_empty(job_input, "tenant_id")
        repo_id = non_empty(job_input, "repo_id")
        job_id = non_empty(job_input, "job_id")
        symbols = list_of_objects(job_input.get("symbols"), "symbols")
        dependency_edges = list_of_objects(
            job_input.get("dependency_edges"), "dependency_edges"
        )
        if not symbols:
            raise ValueError("symbols are required for code compiler burst jobs")

        model_id = str(
            job_input.get("model_id")
            or os.environ.get("THEOREM_CODE_COMPILER_MODEL_ID")
            or DEFAULT_MODEL_ID
        )
        allow_unlearned_fallback = truthy(job_input.get("allow_unlearned_fallback"))
        embedder = load_embedder(model_id, allow_unlearned_fallback)
        symbol_texts = [symbol_text(symbol) for symbol in symbols]
        embeddings = embedder.embed(symbol_texts)
        symbol_by_id = {str(symbol.get("symbol_id") or ""): symbol for symbol in symbols}
        embedding_by_id = {
            str(symbol.get("symbol_id") or ""): embeddings[idx]
            for idx, symbol in enumerate(symbols)
            if str(symbol.get("symbol_id") or "")
        }
        pairs = select_pairs(
            symbols,
            dependency_edges,
            int(job_input.get("max_pairs") or 512),
        )
        features = [
            feature_record(
                tenant_id=tenant_id,
                repo_id=repo_id,
                job_id=job_id,
                model_id=embedder.model_id,
                source=symbol_by_id[source_id],
                target=symbol_by_id[target_id],
                source_embedding=embedding_by_id[source_id],
                target_embedding=embedding_by_id[target_id],
                dependency_edges=dependency_edges,
            )
            for source_id, target_id in pairs
            if source_id in symbol_by_id
            and target_id in symbol_by_id
            and source_id in embedding_by_id
            and target_id in embedding_by_id
        ]
        annotations = [annotation_for_feature(record) for record in features]
        processes = detect_processes(symbols, dependency_edges, limit=32)
        completed_at_ms = int(time.time() * 1000)
        artifact_id = artifact_id_for(job_id, "summary")
        elapsed = round(time.time() - started, 3)
        return {
            "tenant_id": tenant_id,
            "repo_id": repo_id,
            "job_id": job_id,
            "worker_id": os.environ.get("RUNPOD_POD_ID")
            or os.environ.get("HOSTNAME")
            or WORKER_VERSION,
            "model_id": embedder.model_id,
            "processes": processes,
            "patterns": [],
            "features": features,
            "annotations": annotations,
            "artifacts": [
                {
                    "artifact_id": artifact_id,
                    "artifact_kind": "code_compiler_burst_summary",
                    "payload": {
                        "worker_version": WORKER_VERSION,
                        "model_id": embedder.model_id,
                        "symbol_count": len(symbols),
                        "dependency_edge_count": len(dependency_edges),
                        "pair_count": len(features),
                        "process_count": len(processes),
                        "elapsed_seconds": elapsed,
                    },
                    "provenance": {
                        "kind": "runpod_code_compiler_burst",
                        "job_id": job_id,
                        "repo_ref": job_input.get("repo_ref"),
                        "learned_embedding_model": not embedder.unlearned_fallback,
                    },
                }
            ],
            "completed_at_ms": completed_at_ms,
        }
    except Exception as exc:  # noqa: BLE001 - worker must return provider-readable errors.
        return {"error": str(exc), "error_type": type(exc).__name__}


class LearnedEmbedder:
    def __init__(self, model_id: str, model: Any):
        self.model_id = model_id
        self.model = model
        self.unlearned_fallback = False

    def embed(self, texts: list[str]) -> list[list[float]]:
        vectors = self.model.encode(
            texts,
            normalize_embeddings=True,
            show_progress_bar=False,
        )
        return [[float(value) for value in vector] for vector in vectors]


class HashFallbackEmbedder:
    def __init__(self, model_id: str):
        self.model_id = f"unlearned-local-fallback:{model_id}"
        self.unlearned_fallback = True

    def embed(self, texts: list[str]) -> list[list[float]]:
        return [hash_embedding(text, 96) for text in texts]


def load_embedder(model_id: str, allow_unlearned_fallback: bool) -> LearnedEmbedder | HashFallbackEmbedder:
    global _EMBEDDER, _EMBEDDER_ID
    if _EMBEDDER is not None and _EMBEDDER_ID == model_id:
        return _EMBEDDER
    try:
        from sentence_transformers import SentenceTransformer

        model = SentenceTransformer(model_id)
        _EMBEDDER = LearnedEmbedder(model_id, model)
        _EMBEDDER_ID = model_id
        return _EMBEDDER
    except Exception as exc:  # noqa: BLE001
        if allow_unlearned_fallback:
            _EMBEDDER = HashFallbackEmbedder(model_id)
            _EMBEDDER_ID = model_id
            return _EMBEDDER
        raise RuntimeError(
            f"learned embedder {model_id!r} could not be loaded; "
            "set allow_unlearned_fallback=true only for local smoke"
        ) from exc


def select_pairs(
    symbols: list[dict[str, Any]],
    dependency_edges: list[dict[str, Any]],
    max_pairs: int,
) -> list[tuple[str, str]]:
    symbol_ids = {str(symbol.get("symbol_id") or "") for symbol in symbols}
    pairs: set[tuple[str, str]] = set()
    for edge in dependency_edges:
        source = str(edge.get("from_symbol_id") or "")
        target = str(edge.get("to_symbol_id") or "")
        if source in symbol_ids and target in symbol_ids and source != target:
            pairs.add((source, target))

    by_file: dict[str, list[str]] = {}
    for symbol in symbols:
        symbol_id = str(symbol.get("symbol_id") or "")
        file_path = str(symbol.get("file_path") or "")
        if symbol_id and file_path:
            by_file.setdefault(file_path, []).append(symbol_id)
    for ids in by_file.values():
        for left, right in zip(ids, ids[1:]):
            if left != right:
                pairs.add((left, right))

    ordered = sorted(pairs)
    return ordered[: max(1, max_pairs)]


def feature_record(
    *,
    tenant_id: str,
    repo_id: str,
    job_id: str,
    model_id: str,
    source: dict[str, Any],
    target: dict[str, Any],
    source_embedding: list[float],
    target_embedding: list[float],
    dependency_edges: list[dict[str, Any]],
) -> dict[str, Any]:
    source_id = str(source["symbol_id"])
    target_id = str(target["symbol_id"])
    source_tokens = tokens(symbol_text(source))
    target_tokens = tokens(symbol_text(target))
    dependency_edge = dependency_edge_type(dependency_edges, source_id, target_id)
    sbert_cosine = round6(cosine(source_embedding, target_embedding))
    same_file = str(source.get("file_path") or "") == str(target.get("file_path") or "")
    same_kind = str(source.get("kind") or "") == str(target.get("kind") or "")
    shared = source_tokens & target_tokens
    union = source_tokens | target_tokens
    features = zero_features()
    features.update(
        {
            "jaccard_coefficient": round6(len(shared) / max(1, len(union))),
            "bm25_score": round6(len(shared) / max(1, len(target_tokens))),
            "sbert_cosine": sbert_cosine,
            "tfidf_similarity": round6(len(shared) / max(1, math.sqrt(len(source_tokens) * len(target_tokens)))),
            "shared_entity_count": float(len(shared)),
            "same_object_type": 1.0 if same_kind else 0.0,
            "same_notebook": 1.0 if same_file else 0.0,
            "shared_cluster": 1.0 if same_file else 0.0,
            "gnn_structural_score": 1.0 if dependency_edge else 0.0,
            "gnn_edge_prediction": max(0.0, sbert_cosine) if dependency_edge else max(0.0, sbert_cosine * 0.5),
            "rule_support_count": 1.0 if dependency_edge else 0.0,
            "rule_net_score": 1.0 if dependency_edge else 0.0,
            "source_entrenchment": float(len(source.get("dependency_names") or [])),
            "target_entrenchment": float(len(target.get("call_names") or [])),
            "deep_analogy_score": max(0.0, sbert_cosine),
            "rnd_novelty_score": round6(1.0 - max(0.0, sbert_cosine)),
            "spacetime_temporal_score": 1.0 if same_file else 0.0,
        }
    )
    feature_id = "code:feature:" + stable_hash(
        [job_id, source_id, target_id, model_id, FEATURE_VERSION]
    )
    return {
        "feature_id": feature_id,
        "source_symbol_id": source_id,
        "target_symbol_id": target_id,
        "feature_version": FEATURE_VERSION,
        "model_id": model_id,
        "features": features,
        "provenance": {
            "kind": "runpod_code_compiler_burst",
            "tenant_id": tenant_id,
            "repo_id": repo_id,
            "job_id": job_id,
            "model_id": model_id,
            "dependency_edge_type": dependency_edge,
        },
    }


def annotation_for_feature(feature: dict[str, Any]) -> dict[str, Any]:
    values = feature["features"]
    active = [
        {"feature": name, "value": float(values[name]), "importance": abs(float(values[name]))}
        for name in FEATURE_NAMES
        if abs(float(values.get(name, 0.0))) >= 0.03
    ]
    active.sort(key=lambda item: (-item["importance"], item["feature"]))
    top_features = active[:8]
    evidence_count = float(len(active))
    epistemic_uncertainty = 1.0 if evidence_count <= 2.0 else 1.0 - min(1.0, (evidence_count - 2.0) / 48.0)
    aleatoric_uncertainty = (
        float(values.get("nli_contradiction_score", 0.0))
        + float(values.get("rnd_novelty_score", 0.0))
    ) / 2.0
    annotation_id = "code:annotation:" + stable_hash(
        [
            feature["feature_id"],
            evidence_count,
            epistemic_uncertainty,
            aleatoric_uncertainty,
            "runpod-edl-ebl-v1",
        ]
    )
    explanation = "RunPod learned code embedding evidence is driven by "
    explanation += ", ".join(item["feature"] for item in top_features) if top_features else "no active features"
    return {
        "annotation_id": annotation_id,
        "feature_id": feature["feature_id"],
        "epistemic_uncertainty": round6(epistemic_uncertainty),
        "aleatoric_uncertainty": round6(aleatoric_uncertainty),
        "evidence_count": evidence_count,
        "active_feature_count": len(active),
        "explanation": explanation + ".",
        "top_features": [
            {
                "feature": item["feature"],
                "value": round6(item["value"]),
                "importance": round6(item["importance"]),
            }
            for item in top_features
        ],
        "calibration_version": "runpod-edl-ebl-v1",
    }


def detect_processes(
    symbols: list[dict[str, Any]],
    dependency_edges: list[dict[str, Any]],
    limit: int,
) -> list[dict[str, Any]]:
    symbol_by_id = {str(symbol.get("symbol_id") or ""): symbol for symbol in symbols}
    outgoing: dict[str, list[str]] = {}
    for edge in dependency_edges:
        source = str(edge.get("from_symbol_id") or "")
        target = str(edge.get("to_symbol_id") or "")
        if source in symbol_by_id and target in symbol_by_id:
            outgoing.setdefault(source, []).append(target)
    entries = [
        symbol
        for symbol in symbols
        if entry_trigger(symbol)
    ]
    flows = []
    for entry in entries[:limit]:
        entry_id = str(entry["symbol_id"])
        seen = {entry_id}
        queue: list[tuple[str, int]] = [(entry_id, 0)]
        steps = []
        while queue and len(steps) < 24:
            symbol_id, depth = queue.pop(0)
            symbol = symbol_by_id[symbol_id]
            steps.append(
                {
                    "symbol_id": symbol_id,
                    "name": str(symbol.get("name") or symbol_id),
                    "kind": str(symbol.get("kind") or "symbol"),
                    "file_path": str(symbol.get("file_path") or ""),
                    "line": symbol.get("line"),
                    "depth": depth,
                }
            )
            if depth >= 4:
                continue
            for target_id in outgoing.get(symbol_id, []):
                if target_id in seen:
                    continue
                seen.add(target_id)
                queue.append((target_id, depth + 1))
        flows.append(
            {
                "process_id": "code:process:" + stable_hash([entry_id, [step["symbol_id"] for step in steps]]),
                "entry_symbol_id": entry_id,
                "entry_name": str(entry.get("name") or entry_id),
                "entry_file_path": str(entry.get("file_path") or ""),
                "entry_line": entry.get("line"),
                "trigger": entry_trigger(entry),
                "confidence": round6(min(1.0, 0.6 + (len(steps) * 0.04))),
                "steps": steps,
            }
        )
    return flows


def entry_trigger(symbol: dict[str, Any]) -> str:
    name = str(symbol.get("name") or "").lower()
    signature = str(symbol.get("signature") or "").lower()
    if name == "main" or "#[tokio::main]" in signature or "#[main]" in signature:
        return "main_entrypoint"
    if name.endswith("_handler"):
        return "handler"
    if "route" in name or "endpoint" in name:
        return "route"
    if "process" in name or "compile" in name or "run" in name:
        return "process"
    return ""


def dependency_edge_type(
    dependency_edges: list[dict[str, Any]],
    source_id: str,
    target_id: str,
) -> str:
    for edge in dependency_edges:
        if (
            str(edge.get("from_symbol_id") or "") == source_id
            and str(edge.get("to_symbol_id") or "") == target_id
        ):
            return str(edge.get("edge_type") or "dependency")
    return ""


def non_empty(value: dict[str, Any], key: str) -> str:
    text = str(value.get(key) or "").strip()
    if not text:
        raise ValueError(f"{key} is required")
    return text


def list_of_objects(value: Any, name: str) -> list[dict[str, Any]]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"{name} must be an array")
    result = []
    for item in value:
        if not isinstance(item, dict):
            raise ValueError(f"{name} entries must be objects")
        result.append(item)
    return result


def symbol_text(symbol: dict[str, Any]) -> str:
    parts = [
        str(symbol.get("file_path") or ""),
        str(symbol.get("kind") or ""),
        str(symbol.get("name") or ""),
        str(symbol.get("signature") or ""),
        " ".join(str(value) for value in symbol.get("call_names") or []),
        " ".join(str(value) for value in symbol.get("dependency_names") or []),
    ]
    return " ".join(part for part in parts if part.strip())


def tokens(text: str) -> set[str]:
    cleaned = "".join(ch.lower() if ch.isalnum() else " " for ch in text)
    return {part for part in cleaned.split() if len(part) > 1}


def zero_features() -> dict[str, float]:
    return {name: 0.0 for name in FEATURE_NAMES}


def cosine(left: list[float], right: list[float]) -> float:
    numerator = sum(a * b for a, b in zip(left, right))
    left_norm = math.sqrt(sum(a * a for a in left)) or 1.0
    right_norm = math.sqrt(sum(b * b for b in right)) or 1.0
    return numerator / (left_norm * right_norm)


def hash_embedding(text: str, dims: int) -> list[float]:
    values = []
    for idx in range(dims):
        digest = hashlib.sha256(f"{idx}:{text}".encode("utf-8")).digest()
        value = int.from_bytes(digest[:4], "big") / 0xFFFFFFFF
        values.append((value * 2.0) - 1.0)
    norm = math.sqrt(sum(value * value for value in values)) or 1.0
    return [value / norm for value in values]


def stable_hash(value: Any) -> str:
    body = json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")
    return hashlib.sha256(body).hexdigest()


def artifact_id_for(job_id: str, kind: str) -> str:
    return "code:artifact:" + stable_hash([job_id, kind])


def round6(value: float) -> float:
    return round(float(value), 6)


def truthy(value: Any) -> bool:
    return str(value).strip().lower() in {"1", "true", "yes", "on"}


def run_local(input_path: str) -> int:
    payload = json.loads(Path(input_path).read_text())
    result = handler({"input": payload})
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if not result.get("error") else 1


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--local-input":
        raise SystemExit(run_local(sys.argv[2]))
    import runpod

    runpod.serverless.start({"handler": handler})
