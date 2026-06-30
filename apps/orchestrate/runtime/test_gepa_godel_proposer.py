import contextlib
import io
import json
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

from apps.orchestrate.runtime.gepa_godel_proposer import (
    HarnessGepaAdapter,
    HarnessHttpEvaluator,
    HarnessRolloutOutput,
    TrainExample,
    candidate_payload_from_result,
    load_trainset_jsonl,
    load_trainset_url,
    main,
)


class FakeResult:
    best_candidate = {"instruction.user_prompt_improver": "Rewrite with crisp constraints."}
    best_candidate_id = "cand:7"
    parents = ["cand:2"]
    val_aggregate_scores = {"instance:1": 0.8}


class GepaGodelProposerTests(unittest.TestCase):
    def test_adapter_returns_scores_and_reflection_records(self):
        example = _example(score=0.7, feedback="source_independence dropped")

        adapter = HarnessGepaAdapter(
            "instruction.user_prompt_improver",
            evaluator=lambda item, instruction: HarnessRolloutOutput(
                value={"rewritten": instruction},
                score=item.score + 0.1,
                feedback=item.feedback,
            ),
        )
        batch = adapter.evaluate(
            [example],
            {"instruction.user_prompt_improver": "Rewrite precisely."},
            capture_traces=True,
        )
        reflective = adapter.make_reflective_dataset(
            {"instruction.user_prompt_improver": "Rewrite precisely."},
            batch,
            ["instruction.user_prompt_improver"],
        )

        self.assertAlmostEqual(batch.scores[0], 0.8)
        self.assertIn(
            "source_independence dropped",
            reflective["instruction.user_prompt_improver"][0]["Feedback"],
        )
        self.assertEqual(
            reflective["instruction.user_prompt_improver"][0]["Inputs"]["instruction_key"],
            "instruction.user_prompt_improver",
        )

    def test_candidate_payload_is_rust_delta_ready(self):
        payload = candidate_payload_from_result(
            FakeResult(),
            instruction_key="instruction.user_prompt_improver",
            gepa_run_id="run:42",
        )

        self.assertEqual(payload["gepa_run_id"], "run:42")
        self.assertEqual(payload["candidate_id"], "cand:7")
        self.assertEqual(payload["instruction_key"], "instruction.user_prompt_improver")
        self.assertEqual(payload["parents"], ["cand:2"])
        self.assertEqual(payload["val_subscores"], {"instance:1": 0.8})

    def test_dry_run_cli_round_trips_candidate_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            trainset_path = Path(tmp) / "trainset.jsonl"
            trainset_path.write_text(
                '{"intent_id":"intent:prompt","input":{},"trace":{},"outcome":{},'
                '"feedback":"ok","score":0.5,"axes":{}}\n',
                encoding="utf-8",
            )

            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                exit_code = main(
                    [
                        "--trainset",
                        str(trainset_path),
                        "--instruction-key",
                        "instruction.user_prompt_improver",
                        "--seed-candidate-json",
                        '{"instruction.user_prompt_improver":"Seed."}',
                        "--gepa-run-id",
                        "run:dry",
                        "--dry-run",
                    ]
                )
            self.assertEqual(exit_code, 0)
            self.assertIn('"candidate_id": "dry-run"', stdout.getvalue())
            self.assertEqual(load_trainset_jsonl(trainset_path)[0].score, 0.5)

    def test_http_trainset_and_evaluator_bridge(self):
        class Handler(BaseHTTPRequestHandler):
            request_payload = None

            def do_GET(self):
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.end_headers()
                self.wfile.write(
                    json.dumps({"examples": [_example_payload(score=0.42)]}).encode("utf-8")
                )

            def do_POST(self):
                length = int(self.headers.get("content-length", "0"))
                Handler.request_payload = json.loads(self.rfile.read(length).decode("utf-8"))
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.end_headers()
                self.wfile.write(
                    json.dumps(
                        {
                            "output": {"rewritten": "clearer"},
                            "score": 0.91,
                            "feedback": "served",
                        }
                    ).encode("utf-8")
                )

            def log_message(self, *_args):
                return

        server = HTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            base_url = f"http://127.0.0.1:{server.server_port}"
            examples = load_trainset_url(f"{base_url}/trainset", timeout_seconds=5.0)
            evaluator = HarnessHttpEvaluator(
                f"{base_url}/evaluate",
                "instruction.user_prompt_improver",
                timeout_seconds=5.0,
            )
            output = evaluator(examples[0], "Rewrite carefully.")
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)

        self.assertEqual(examples[0].score, 0.42)
        self.assertEqual(output.value, {"rewritten": "clearer"})
        self.assertEqual(output.score, 0.91)
        self.assertEqual(output.feedback, "served")
        self.assertEqual(
            Handler.request_payload["instruction_key"], "instruction.user_prompt_improver"
        )


def _example(score: float, feedback: str) -> TrainExample:
    return TrainExample(
        intent_id="intent:prompt",
        input={"user_prompt": "make this better"},
        trace={"run_id": "session:1"},
        outcome={"rewritten": "make this better, with constraints"},
        feedback=feedback,
        score=score,
        axes={"task_completion_rate": 1.0},
    )


def _example_payload(score: float) -> dict:
    return {
        "intent_id": "intent:prompt",
        "input": {"user_prompt": "make this better"},
        "trace": {"run_id": "session:1"},
        "outcome": {"rewritten": "make this better, with constraints"},
        "feedback": "ok",
        "score": score,
        "axes": {"task_completion_rate": 1.0},
    }


if __name__ == "__main__":
    unittest.main()
