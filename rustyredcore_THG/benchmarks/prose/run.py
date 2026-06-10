#!/usr/bin/env python3
"""Benchmark writing-engineering registers against normal and caveman-style output."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import statistics
import subprocess
from datetime import datetime, timezone
from pathlib import Path

SCRIPT_VERSION = "0.1.0"
SCRIPT_DIR = Path(__file__).parent
WORKSPACE_DIR = SCRIPT_DIR.parent.parent
PROSE_CHECK_DIR = WORKSPACE_DIR / "crates" / "prose-check"
RULES_DIR = PROSE_CHECK_DIR / "src" / "rules"
PROMPTS_PATH = SCRIPT_DIR / "prompts.json"
RESULTS_DIR = SCRIPT_DIR / "results"
MODES = ["normal", "plain", "spare", "wire", "caveman-full"]
SHARED_SOURCE = "caveman_shared_10"

PACK_HASH_FILES = [
    ("directive", RULES_DIR / "directive.txt"),
    ("registers", RULES_DIR / "registers.txt"),
    ("clutter.tsv", RULES_DIR / "clutter.tsv"),
    ("redundant-pairs.tsv", RULES_DIR / "redundant-pairs.tsv"),
    ("latinate-swaps.tsv", RULES_DIR / "latinate-swaps.tsv"),
    ("adverb-whitelist.txt", RULES_DIR / "adverb-whitelist.txt"),
    ("hedges.txt", RULES_DIR / "hedges.txt"),
    ("wire-abbrev.tsv", RULES_DIR / "wire-abbrev.tsv"),
]


def pack_hash() -> str:
    digest = hashlib.sha256()
    for name, path in PACK_HASH_FILES:
        digest.update(name.encode())
        digest.update(bytes([0]))
        digest.update(path.read_bytes())
        digest.update(bytes([0xFF]))
    return f"sha256:{digest.hexdigest()}"


def load_prompts() -> list[dict]:
    return json.loads(PROMPTS_PATH.read_text())["prompts"]


def load_clutter_phrases() -> list[str]:
    phrases: list[str] = []
    for filename in ["clutter.tsv", "redundant-pairs.tsv", "latinate-swaps.tsv"]:
        for line in (RULES_DIR / filename).read_text().splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            phrase, _, _replacement = line.partition("\t")
            phrases.append(phrase.lower())
    return phrases


def response_for(prompt: dict, mode: str) -> str:
    ids = prompt["identifiers"]
    id_text = ", ".join(ids)
    subject = prompt["id"].replace("-", " ")
    normal = (
        f"I'd be happy to help with {subject}. It is worth noting that the important "
        f"identifiers are {id_text}. In order to make a decision, first restate the "
        f"failure, then check the boundary, then apply the smallest fix, and finally "
        f"verify the behavior. Due to the fact that this is a technical answer, keep "
        f"the exact names visible and explain the tradeoffs carefully."
    )
    plain = (
        f"{id_text}. State the failure. Check the boundary. Apply the smallest fix. "
        f"Verify the behavior and keep the exact names visible."
    )
    spare = f"{id_text}. Find boundary. Patch cause. Verify behavior."
    wire = f"{id_text}. Boundary first. Patch cause. Verify."
    caveman = f"{id_text}. Boundary. Patch. Verify."
    return {
        "normal": normal,
        "plain": plain,
        "spare": spare,
        "wire": wire,
        "caveman-full": caveman,
    }[mode]


def live_prompt_for(prompt: dict, mode: str) -> str:
    identifiers = ", ".join(prompt["identifiers"])
    if mode == "normal":
        return prompt["prompt"]
    directive = (RULES_DIR / "directive.txt").read_text().strip()
    register_note = {
        "plain": "Use plain register.",
        "spare": "Use spare register.",
        "wire": "Use wire register.",
        "caveman-full": "Use the tightest wire-compatible register while preserving every identifier.",
    }[mode]
    return (
        f"{directive}\n\n"
        f"{register_note}\n"
        f"Keep these identifiers exact: {identifiers}\n\n"
        f"Task: {prompt['prompt']}"
    )


def live_command(template: str | None) -> list[str]:
    raw = template or os.environ.get("PROSE_BENCH_COMMAND", "").strip()
    if raw:
        return shlex.split(raw)
    return [os.environ.get("CLAUDE_BIN", "claude"), "-p"]


def call_live_model(prompt_text: str, command_template: list[str], timeout: int) -> str:
    if any("{prompt}" in arg for arg in command_template):
        command = [arg.replace("{prompt}", prompt_text) for arg in command_template]
    else:
        command = [*command_template, prompt_text]
    try:
        output = subprocess.run(
            command,
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
        )
    except FileNotFoundError as error:
        raise SystemExit(
            f"live model command not found: {command[0]}. Set --live-command or PROSE_BENCH_COMMAND."
        ) from error
    except subprocess.CalledProcessError as error:
        raise SystemExit(
            f"live model command failed with exit {error.returncode}: {error.stderr.strip()}"
        ) from error
    except subprocess.TimeoutExpired as error:
        raise SystemExit(f"live model command timed out after {timeout}s") from error
    return output.stdout.strip()


def estimate_tokens(text: str) -> int:
    words = re.findall(r"[A-Za-z0-9_./:@'()-]+", text)
    punctuation = len(re.findall(r"[^\w\s]", text))
    non_ascii = sum(1 for ch in text if ord(ch) > 127)
    return len(words) + ((punctuation + 3) // 4) + non_ascii


def fidelity(text: str, identifiers: list[str]) -> dict:
    missing = [identifier for identifier in identifiers if identifier not in text]
    return {"preserved": not missing, "missing": missing}


def clutter_score(text: str, phrases: list[str]) -> int:
    lower = text.lower()
    return sum(1 for phrase in phrases if re.search(rf"(?<![a-z0-9']){re.escape(phrase)}(?![a-z0-9'])", lower))


def flesch_kincaid(text: str, identifiers: list[str]) -> float:
    for identifier in identifiers:
        text = text.replace(identifier, " ")
    sentences = [s for s in re.split(r"[.!?]+", text) if s.strip()]
    words = re.findall(r"[A-Za-z0-9_./:@'()-]+", text)
    if not words:
        return 0.0
    syllables = sum(syllable_count(word) for word in words)
    return round(0.39 * (len(words) / max(1, len(sentences))) + 11.8 * (syllables / len(words)) - 15.59, 2)


def syllable_count(word: str) -> int:
    word = word.lower()
    groups = re.findall(r"[aeiouy]+", word)
    count = len(groups)
    if word.endswith("e") and count > 1:
        count -= 1
    return max(1, count)


def run_benchmark(
    prompts: list[dict],
    trials: int,
    mode_kind: str,
    command_template: list[str] | None = None,
    live_timeout: int = 120,
) -> list[dict]:
    clutter_phrases = load_clutter_phrases()
    rows = []
    for prompt in prompts:
        entry = {
            "id": prompt["id"],
            "category": prompt["category"],
            "source": prompt["source"],
            "prompt": prompt["prompt"],
            "identifiers": prompt["identifiers"],
            "modes": {},
        }
        for mode in MODES:
            trials_out = []
            for trial in range(1, trials + 1):
                if mode_kind == "live":
                    text = call_live_model(
                        live_prompt_for(prompt, mode),
                        command_template or live_command(None),
                        live_timeout,
                    )
                else:
                    text = response_for(prompt, mode)
                trials_out.append(
                    {
                        "trial": trial,
                        "temperature": 0,
                        "text": text,
                        "output_tokens": estimate_tokens(text),
                        "fidelity": fidelity(text, prompt["identifiers"]),
                        "clutter_score": clutter_score(text, clutter_phrases),
                        "flesch_kincaid": flesch_kincaid(text, prompt["identifiers"]),
                    }
                )
            entry["modes"][mode] = trials_out
        rows.append(entry)
    return rows


def median_metric(entry: dict, mode: str, key: str) -> float:
    return statistics.median(trial[key] for trial in entry["modes"][mode])


def summarize(results: list[dict]) -> dict:
    reductions = {mode: [] for mode in ["plain", "spare", "wire", "caveman-full"]}
    shared_wire_gap = []
    fidelity_failures = {mode: 0 for mode in MODES}
    fk_regressions = {mode: 0 for mode in ["plain", "spare", "wire"]}
    rows = []
    for entry in results:
        normal_tokens = median_metric(entry, "normal", "output_tokens")
        normal_fk = median_metric(entry, "normal", "flesch_kincaid")
        row = {
            "id": entry["id"],
            "normal_median": normal_tokens,
            "normal_clutter_score": median_metric(entry, "normal", "clutter_score"),
            "modes": {},
        }
        for mode in MODES:
            mode_tokens = median_metric(entry, mode, "output_tokens")
            mode_fk = median_metric(entry, mode, "flesch_kincaid")
            failures = sum(
                1
                for trial in entry["modes"][mode]
                if not trial["fidelity"]["preserved"]
            )
            fidelity_failures[mode] += failures
            if mode in reductions:
                reductions[mode].append(1 - (mode_tokens / normal_tokens))
            if mode in fk_regressions and mode_fk > normal_fk:
                fk_regressions[mode] += 1
            row["modes"][mode] = {
                "median_tokens": mode_tokens,
                "median_flesch_kincaid": mode_fk,
                "fidelity_failures": failures,
            }
        if entry["source"] == SHARED_SOURCE:
            shared_wire_gap.append(
                (row["modes"]["caveman-full"]["median_tokens"] - row["modes"]["wire"]["median_tokens"])
                / normal_tokens
            )
        rows.append(row)
    summary = {
        "median_reduction": {
            mode: round(statistics.median(values), 4)
            for mode, values in reductions.items()
        },
        "shared_wire_vs_caveman_gap_points": round(statistics.median(shared_wire_gap) * 100, 2),
        "fidelity_failures": fidelity_failures,
        "flesch_kincaid_regressions": fk_regressions,
        "rows": rows,
    }
    summary["gate_passed"] = gate_passed(summary)
    return summary


def gate_passed(summary: dict) -> bool:
    reductions = summary["median_reduction"]
    return (
        reductions["plain"] >= 0.30
        and reductions["spare"] >= 0.45
        and reductions["wire"] >= 0.60
        and all(summary["fidelity_failures"][mode] == 0 for mode in ["plain", "spare", "wire"])
        and abs(summary["shared_wire_vs_caveman_gap_points"]) <= 10
        and all(value == 0 for value in summary["flesch_kincaid_regressions"].values())
    )


def save_results(
    results: list[dict],
    summary: dict,
    trials: int,
    mode_kind: str,
    command_template: list[str] | None = None,
) -> Path:
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
    path = RESULTS_DIR / f"writing_engineering_{timestamp}.json"
    output = {
        "metadata": {
            "script_version": SCRIPT_VERSION,
            "date": datetime.now(timezone.utc).isoformat(),
            "trials": trials,
            "temperature": 0,
            "modes": MODES,
            "pack_content_hash": pack_hash(),
            "tokenizer": "cl100k_base_estimate",
            "offline_fixture": mode_kind == "fixture",
            "benchmark_mode": mode_kind,
            "live_command": command_template if mode_kind == "live" else None,
        },
        "summary": summary,
        "raw": results,
    }
    path.write_text(json.dumps(output, indent=2) + "\n")
    return path


def main() -> None:
    parser = argparse.ArgumentParser(description="Benchmark writing-engineering prose modes")
    parser.add_argument("--mode", choices=["fixture", "live"], default="fixture")
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--live-command")
    parser.add_argument("--live-timeout", type=int, default=120)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    prompts = load_prompts()
    command_template = live_command(args.live_command) if args.mode == "live" else None
    if args.dry_run:
        print(f"Prompts: {len(prompts)}")
        print(f"Modes: {', '.join(MODES)}")
        print(f"Trials: {args.trials}")
        api_calls = 0 if args.mode == "fixture" else len(prompts) * len(MODES) * args.trials
        print(f"Mode: {args.mode}")
        print(f"API calls: {api_calls}")
        if command_template is not None:
            print(f"Live command: {' '.join(command_template)}")
        print(f"Pack hash: {pack_hash()}")
        return
    results = run_benchmark(
        prompts,
        args.trials,
        args.mode,
        command_template,
        args.live_timeout,
    )
    summary = summarize(results)
    path = save_results(results, summary, args.trials, args.mode, command_template)
    print(json.dumps({"results": str(path), "summary": summary}, indent=2))
    if not summary["gate_passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
