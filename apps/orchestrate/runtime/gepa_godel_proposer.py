from __future__ import annotations

import argparse
import dataclasses
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterable, Sequence
from urllib import request as urlrequest
from urllib.parse import urlparse


@dataclass(frozen=True)
class TrainExample:
    intent_id: str
    input: dict[str, Any]
    trace: dict[str, Any]
    outcome: dict[str, Any] | str | int | float | bool | None
    feedback: str
    score: float
    axes: dict[str, float]

    @classmethod
    def from_mapping(cls, value: dict[str, Any]) -> "TrainExample":
        return cls(
            intent_id=str(value["intent_id"]),
            input=dict(value.get("input") or {}),
            trace=dict(value.get("trace") or {}),
            outcome=value.get("outcome"),
            feedback=str(value.get("feedback") or ""),
            score=float(value.get("score") or 0.0),
            axes={str(key): float(score) for key, score in dict(value.get("axes") or {}).items()},
        )


@dataclass(frozen=True)
class HarnessRolloutOutput:
    value: Any
    score: float
    feedback: str
    error: str | None = None


@dataclass(frozen=True)
class HarnessTrajectory:
    instruction_key: str
    instruction: str
    input: dict[str, Any]
    trace: dict[str, Any]
    outcome: Any
    output: Any
    feedback: str
    error: str | None = None


@dataclass
class EvaluationBatch:
    outputs: list[HarnessRolloutOutput]
    scores: list[float]
    trajectories: list[HarnessTrajectory] | None = None
    objective_scores: list[dict[str, float]] | None = None
    num_metric_calls: int | None = None


Evaluator = Callable[[TrainExample, str], HarnessRolloutOutput]


class HarnessGepaAdapter:
    """GEPA adapter for offline Godel instruction proposal.

    The class follows GEPA's adapter protocol: evaluate a batch against a
    candidate text component, then build reflection records with Inputs,
    Generated Outputs, and Feedback. The real optimizer is loaded only by
    run_gepa_optimize so unit tests do not need the external package.
    """

    def __init__(
        self,
        instruction_key: str,
        evaluator: Evaluator | None = None,
    ) -> None:
        if not instruction_key.startswith("instruction."):
            raise ValueError(f"not an instruction key: {instruction_key}")
        self.instruction_key = instruction_key
        self._evaluator = evaluator or self._precomputed_evaluator

    def evaluate(
        self,
        batch: Sequence[TrainExample],
        candidate: dict[str, str],
        capture_traces: bool = False,
    ) -> EvaluationBatch:
        instruction = candidate[self.instruction_key]
        outputs: list[HarnessRolloutOutput] = []
        scores: list[float] = []
        trajectories: list[HarnessTrajectory] | None = [] if capture_traces else None
        objective_scores: list[dict[str, float]] = []

        for example in batch:
            try:
                output = self._evaluator(example, instruction)
            except Exception as exc:
                output = HarnessRolloutOutput(
                    value={"error": str(exc)},
                    score=0.0,
                    feedback=f"Candidate failed during harness evaluation: {exc}",
                    error=str(exc),
                )
            outputs.append(output)
            scores.append(float(output.score))
            objective_scores.append({"productivity": float(output.score)})
            if trajectories is not None:
                trajectories.append(
                    HarnessTrajectory(
                        instruction_key=self.instruction_key,
                        instruction=instruction,
                        input=example.input,
                        trace=example.trace,
                        outcome=example.outcome,
                        output=output.value,
                        feedback=output.feedback,
                        error=output.error,
                    )
                )

        return EvaluationBatch(
            outputs=outputs,
            scores=scores,
            trajectories=trajectories,
            objective_scores=objective_scores,
            num_metric_calls=len(batch),
        )

    def make_reflective_dataset(
        self,
        candidate: dict[str, str],
        eval_batch: EvaluationBatch,
        components_to_update: Sequence[str],
    ) -> dict[str, list[dict[str, Any]]]:
        records: dict[str, list[dict[str, Any]]] = {}
        trajectories = eval_batch.trajectories or []
        for component in components_to_update:
            component_records: list[dict[str, Any]] = []
            for index, trajectory in enumerate(trajectories):
                score = eval_batch.scores[index] if index < len(eval_batch.scores) else 0.0
                component_records.append(
                    {
                        "Inputs": {
                            "instruction_key": trajectory.instruction_key,
                            "candidate_instruction": candidate.get(component, ""),
                            "input": trajectory.input,
                            "trace": trajectory.trace,
                        },
                        "Generated Outputs": {
                            "output": trajectory.output,
                            "expected_outcome": trajectory.outcome,
                        },
                        "Feedback": f"Score: {score:.6f}\n{trajectory.feedback}",
                    }
                )
            records[component] = component_records
        return records

    def _precomputed_evaluator(
        self, example: TrainExample, _instruction: str
    ) -> HarnessRolloutOutput:
        return HarnessRolloutOutput(
            value=example.outcome,
            score=example.score,
            feedback=example.feedback,
        )


def run_gepa_optimize(
    *,
    seed_candidate: dict[str, str],
    trainset: Sequence[TrainExample],
    adapter: HarnessGepaAdapter,
    reflection_lm: str,
    max_metric_calls: int,
    valset: Sequence[TrainExample] | None = None,
) -> Any:
    try:
        import gepa
    except ImportError as exc:
        raise RuntimeError(
            "Install the GEPA sidecar dependency first, for example `pip install gepa`."
        ) from exc

    return gepa.optimize(
        seed_candidate=seed_candidate,
        trainset=list(trainset),
        valset=list(valset or trainset),
        adapter=adapter,
        reflection_lm=reflection_lm,
        max_metric_calls=max_metric_calls,
    )


class HarnessHttpEvaluator:
    def __init__(
        self,
        evaluator_url: str,
        instruction_key: str,
        timeout_seconds: float = 60.0,
    ) -> None:
        self.evaluator_url = _require_http_url(evaluator_url, field="evaluator_url")
        self.instruction_key = instruction_key
        self.timeout_seconds = timeout_seconds

    def __call__(self, example: TrainExample, instruction: str) -> HarnessRolloutOutput:
        payload = {
            "instruction_key": self.instruction_key,
            "instruction": instruction,
            "example": dataclasses.asdict(example),
        }
        body = json.dumps(payload).encode("utf-8")
        req = urlrequest.Request(
            self.evaluator_url,
            data=body,
            headers={"content-type": "application/json"},
            method="POST",
        )
        with urlrequest.urlopen(req, timeout=self.timeout_seconds) as response:
            parsed = json.loads(response.read().decode("utf-8"))
        return HarnessRolloutOutput(
            value=parsed.get("output", parsed.get("value")),
            score=float(parsed.get("score") or 0.0),
            feedback=str(parsed.get("feedback") or ""),
            error=parsed.get("error"),
        )


def candidate_payload_from_result(
    result: Any,
    *,
    instruction_key: str,
    gepa_run_id: str,
    fallback_candidate: dict[str, str] | None = None,
) -> dict[str, Any]:
    candidate = _best_candidate(result) or fallback_candidate or {}
    if instruction_key not in candidate:
        raise ValueError(f"GEPA result did not contain instruction key {instruction_key}")
    return {
        "gepa_run_id": gepa_run_id,
        "candidate_id": str(_get(result, "best_candidate_id", "best")),
        "instruction_key": instruction_key,
        "optimized_instruction": str(candidate[instruction_key]),
        "parents": _best_candidate_parents(result),
        "val_subscores": _val_subscores(result),
        "lineage": _jsonable(result),
    }


def load_trainset_jsonl(path: Path) -> list[TrainExample]:
    examples: list[TrainExample] = []
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                examples.append(TrainExample.from_mapping(json.loads(line)))
    return examples


def load_trainset_url(url: str, timeout_seconds: float = 60.0) -> list[TrainExample]:
    url = _require_http_url(url, field="trainset_url")
    with urlrequest.urlopen(url, timeout=timeout_seconds) as response:
        text = response.read().decode("utf-8")
    stripped = text.lstrip()
    if stripped.startswith("{"):
        payload = json.loads(text)
        examples = payload.get("examples", [])
        return [TrainExample.from_mapping(dict(example)) for example in examples]
    return [
        TrainExample.from_mapping(json.loads(line))
        for line in text.splitlines()
        if line.strip()
    ]


def _best_candidate(result: Any) -> dict[str, str] | None:
    if isinstance(result, dict):
        value = result.get("best_candidate") or result.get("candidate")
    else:
        value = getattr(result, "best_candidate", None) or getattr(result, "candidate", None)
    if value is None:
        return None
    return {str(key): str(item) for key, item in dict(value).items()}


def _best_candidate_parents(result: Any) -> list[str]:
    raw = _get(result, "parents", [])
    if raw is None:
        return []
    if isinstance(raw, dict):
        selected = raw.get(str(_best_index(result)), raw.get(_best_candidate_id(result), []))
        return _parent_indexes_to_ids(selected, result)
    if isinstance(raw, list) and raw and all(isinstance(item, (str, int)) for item in raw):
        return [str(item) for item in raw]
    if isinstance(raw, (list, tuple)):
        best_idx = _best_index(result)
        selected = raw[best_idx] if 0 <= best_idx < len(raw) else []
        return _parent_indexes_to_ids(selected, result)
    return []


def _parent_indexes_to_ids(value: Any, result: Any) -> list[str]:
    if value is None:
        return []
    if not isinstance(value, (list, tuple)):
        value = [value]
    candidate_ids = _candidate_ids(result)
    parents: list[str] = []
    for item in value:
        if item is None:
            continue
        if isinstance(item, int) and 0 <= item < len(candidate_ids):
            parents.append(candidate_ids[item])
        else:
            parents.append(str(item))
    return parents


def _candidate_ids(result: Any) -> list[str]:
    raw = _get(result, "candidate_ids", [])
    if isinstance(raw, (list, tuple)):
        return [str(item) for item in raw]
    return []


def _best_index(result: Any) -> int:
    raw = _get(result, "best_idx", _get(result, "best_index", 0))
    try:
        return int(raw)
    except (TypeError, ValueError):
        return 0


def _best_candidate_id(result: Any) -> str:
    return str(_get(result, "best_candidate_id", _best_index(result)))


def _val_subscores(result: Any) -> dict[str, float]:
    raw = _get(result, "val_subscores", None)
    if raw is not None:
        selected = _select_candidate_indexed_value(raw, result)
        if isinstance(selected, dict):
            return {str(key): float(value) for key, value in selected.items()}
        if isinstance(selected, list):
            return {f"instance:{index}": float(value) for index, value in enumerate(selected)}
    raw = _get(result, "val_aggregate_scores", {})
    if isinstance(raw, dict):
        return {str(key): float(value) for key, value in raw.items()}
    if isinstance(raw, list):
        best_idx = _best_index(result)
        if 0 <= best_idx < len(raw):
            return {"aggregate": float(raw[best_idx])}
    return {}


def _select_candidate_indexed_value(raw: Any, result: Any) -> Any:
    if isinstance(raw, dict):
        candidate_id = _best_candidate_id(result)
        if candidate_id in raw:
            return raw[candidate_id]
        index_key = str(_best_index(result))
        return raw.get(index_key, raw)
    if isinstance(raw, (list, tuple)):
        best_idx = _best_index(result)
        if 0 <= best_idx < len(raw):
            return raw[best_idx]
    return raw


def _require_http_url(url: str, *, field: str) -> str:
    parsed = urlparse(url)
    if parsed.scheme.lower() not in {"http", "https"} or not parsed.netloc:
        raise ValueError(f"{field} must be an http(s) URL")
    return url


def _get(value: Any, attr: str, fallback: Any) -> Any:
    if isinstance(value, dict):
        return value.get(attr, fallback)
    return getattr(value, attr, fallback)


def _jsonable(value: Any) -> Any:
    if dataclasses.is_dataclass(value):
        return _jsonable(dataclasses.asdict(value))
    if isinstance(value, dict):
        return {str(key): _jsonable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_jsonable(item) for item in value]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    if hasattr(value, "__dict__"):
        return _jsonable(vars(value))
    return str(value)


def main(argv: Iterable[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Offline GEPA proposer for Godel instructions.")
    parser.add_argument("--trainset", type=Path)
    parser.add_argument("--trainset-url")
    parser.add_argument("--instruction-key", required=True)
    parser.add_argument("--seed-candidate-json", required=True)
    parser.add_argument("--gepa-run-id", required=True)
    parser.add_argument("--reflection-lm", default="openai/gpt-4o")
    parser.add_argument("--max-metric-calls", type=int, default=50)
    parser.add_argument("--evaluator-url")
    parser.add_argument("--timeout-seconds", type=float, default=60.0)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(list(argv) if argv is not None else None)

    if bool(args.trainset) == bool(args.trainset_url):
        parser.error("provide exactly one of --trainset or --trainset-url")
    trainset = (
        load_trainset_jsonl(args.trainset)
        if args.trainset
        else load_trainset_url(args.trainset_url, args.timeout_seconds)
    )
    seed_candidate = json.loads(args.seed_candidate_json)
    evaluator = (
        HarnessHttpEvaluator(args.evaluator_url, args.instruction_key, args.timeout_seconds)
        if args.evaluator_url
        else None
    )
    adapter = HarnessGepaAdapter(args.instruction_key, evaluator=evaluator)
    if args.dry_run:
        result = {"best_candidate": seed_candidate, "best_candidate_id": "dry-run"}
    else:
        result = run_gepa_optimize(
            seed_candidate=seed_candidate,
            trainset=trainset,
            adapter=adapter,
            reflection_lm=args.reflection_lm,
            max_metric_calls=args.max_metric_calls,
        )
    payload = candidate_payload_from_result(
        result,
        instruction_key=args.instruction_key,
        gepa_run_id=args.gepa_run_id,
        fallback_candidate=seed_candidate,
    )
    print(json.dumps(payload, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
