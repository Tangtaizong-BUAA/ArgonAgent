#!/usr/bin/env python3
"""Deterministic scaffold/context-budget eval for native DeepSeek/Qwen modes."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "eval" / "fixtures" / "scaffold" / "scaffold_cases.json"


def run_budget_show(family: str, role: str) -> dict[str, Any]:
    completed = subprocess.run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "researchcode-cli",
            "--",
            "context-budget-show",
            family,
            role,
        ],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr.strip() or completed.stdout.strip())
    json_lines = [line for line in completed.stdout.splitlines() if line.startswith("{")]
    if not json_lines:
        raise RuntimeError(f"missing budget json for {family}/{role}: {completed.stdout}")
    return json.loads(json_lines[-1])


def check_case(case: dict[str, Any]) -> list[str]:
    budget = run_budget_show(case["family"], case["role"])
    errors: list[str] = []
    case_id = case["id"]
    if budget["scaffold_level"] != case["expected_scaffold_level"]:
        errors.append(
            f"{case_id} scaffold {budget['scaffold_level']!r} != {case['expected_scaffold_level']!r}"
        )
    if budget["output_reserve_tokens"] + budget["emergency_reserve_tokens"] <= 0:
        errors.append(f"{case_id} missing protected reserve")
    ratio = budget["prompt_scaffold_tokens"] / budget["max_context_tokens"]
    if "max_prompt_context_ratio" in case and ratio >= case["max_prompt_context_ratio"]:
        errors.append(f"{case_id} prompt ratio {ratio:.4f} exceeds guardrail")
    for key, op, actual_key in [
        ("min_max_context_tokens", "min", "max_context_tokens"),
        ("max_max_context_tokens", "max", "max_context_tokens"),
        ("min_prompt_scaffold_tokens", "min", "prompt_scaffold_tokens"),
        ("min_dynamic_context_tokens", "min", "dynamic_context_tokens"),
        ("min_protected_reserve_tokens", "min", None),
        ("min_reasoning_replay_budget", "min", "reasoning_replay_budget"),
        ("max_compaction_threshold_tokens", "max", "compaction_threshold"),
        ("max_active_tools_at_least", "min", "max_active_tools"),
        ("max_active_tools_at_most", "max", "max_active_tools"),
        ("max_files_per_turn_at_most", "max", "max_files_per_turn"),
    ]:
        if key not in case:
            continue
        actual = (
            budget["output_reserve_tokens"] + budget["emergency_reserve_tokens"]
            if actual_key is None
            else budget[actual_key]
        )
        expected = case[key]
        if op == "min" and actual < expected:
            errors.append(f"{case_id} {key} actual {actual} < expected {expected}")
        if op == "max" and actual > expected:
            errors.append(f"{case_id} {key} actual {actual} > expected {expected}")
    return errors


def main() -> int:
    cases = json.loads(FIXTURE.read_text(encoding="utf-8"))
    errors: list[str] = []
    for case in cases:
        errors.extend(check_case(case))
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(json.dumps({"suite": "scaffold", "total": len(cases), "failures": 0}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
