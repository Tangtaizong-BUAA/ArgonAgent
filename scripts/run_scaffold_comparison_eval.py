#!/usr/bin/env python3
"""Deterministic scaffold comparison eval.

This is not a quality benchmark against live models. It is a release-blocking
configuration gate that prevents accidental profile regressions:

- Qwen must not inherit DeepSeek-sized prompt/tool scaffold.
- DeepSeek must retain its native reasoning/tool scaffold while respecting the
  256K safety cap.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "eval" / "fixtures" / "scaffold" / "scaffold_comparison_cases.json"


def budget(family: str, role: str) -> dict[str, Any]:
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
    for line in completed.stdout.splitlines():
        if line.startswith("{"):
            return json.loads(line)
    raise RuntimeError(f"missing budget json for {family}/{role}")


def qwen_lite_wins(case: dict[str, Any]) -> list[str]:
    qwen = budget("qwen", case["role"])
    deepseek_full = budget("deepseek", case["role"])
    errors: list[str] = []
    qwen_ratio = qwen["prompt_scaffold_tokens"] / qwen["max_context_tokens"]
    if qwen_ratio >= 0.10:
        errors.append(f"{case['id']} qwen prompt ratio {qwen_ratio:.4f} is not lite")
    if qwen["reasoning_replay_budget"] != 0:
        errors.append(f"{case['id']} qwen unexpectedly inherited DeepSeek reasoning replay")
    if qwen["max_active_tools"] > 7:
        errors.append(f"{case['id']} qwen active tools too broad: {qwen['max_active_tools']}")
    if deepseek_full["max_active_tools"] <= qwen["max_active_tools"]:
        errors.append(f"{case['id']} full scaffold is not broader than qwen mode")
    if deepseek_full["reasoning_replay_budget"] <= 0:
        errors.append(f"{case['id']} deepseek full scaffold missing reasoning replay")
    return errors


def deepseek_full_wins(case: dict[str, Any]) -> list[str]:
    deepseek = budget("deepseek", case["role"])
    qwen_lite = budget("qwen", "executor")
    errors: list[str] = []
    if deepseek["scaffold_level"] != "DeepSeekFull":
        errors.append(f"{case['id']} deepseek not full scaffold")
    if deepseek["max_context_tokens"] != 256_000:
        errors.append(f"{case['id']} deepseek safety cap changed: {deepseek['max_context_tokens']}")
    if deepseek["compaction_threshold"] > 192_000:
        errors.append(f"{case['id']} deepseek compaction threshold too high")
    if deepseek["reasoning_replay_budget"] < 12_000:
        errors.append(f"{case['id']} deepseek reasoning replay budget too small")
    if deepseek["output_reserve_tokens"] + deepseek["emergency_reserve_tokens"] < 32_000:
        errors.append(f"{case['id']} deepseek protected reserves too small")
    if deepseek["max_active_tools"] < 8:
        errors.append(f"{case['id']} deepseek tool budget too narrow")
    if deepseek["max_files_per_turn"] <= qwen_lite["max_files_per_turn"]:
        errors.append(f"{case['id']} deepseek file budget not broader than qwen executor")
    return errors


def main() -> int:
    cases = json.loads(FIXTURE.read_text(encoding="utf-8"))
    errors: list[str] = []
    for case in cases:
        if case["comparison"].startswith("qwen"):
            errors.extend(qwen_lite_wins(case))
        elif case["comparison"] == "deepseek_full_vs_lite":
            errors.extend(deepseek_full_wins(case))
        else:
            errors.append(f"{case['id']} unknown comparison {case['comparison']}")
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(json.dumps({"suite": "scaffold_comparison", "total": len(cases), "failures": 0}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
