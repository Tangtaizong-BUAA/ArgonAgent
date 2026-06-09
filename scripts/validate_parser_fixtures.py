#!/usr/bin/env python3
"""Validate DeepSeek/Qwen parser fixture policy gates."""

from __future__ import annotations

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def load(name: str) -> list[dict]:
    return json.loads((ROOT / "eval" / "fixtures" / name / "parser_golden.json").read_text(encoding="utf-8"))


def validate_deepseek(cases: list[dict]) -> list[str]:
    errors: list[str] = []
    if len(cases) < 12:
        errors.append("DeepSeek parser fixtures must have at least 12 cases")
    wrong_tool_cases = [case for case in cases if case["id"] in {"DS-PARSE-04", "DS-PARSE-07"}]
    for case in wrong_tool_cases:
        if case["expected_action"] not in {"deny", "retry"}:
            errors.append(f"{case['id']} must deny or retry")
    low_conf_exec = [
        case["id"]
        for case in cases
        if case.get("confidence") == "low" and "execute" in case.get("expected_action", "")
    ]
    if low_conf_exec:
        errors.append(f"low-confidence DeepSeek repairs execute: {low_conf_exec}")
    if not any("reasoning" in case["id"].lower() or "reasoning" in case.get("notes", "") for case in cases):
        errors.append("DeepSeek fixtures must include reasoning sanitizer/redaction cases")
    return errors


def validate_qwen(cases: list[dict]) -> list[str]:
    errors: list[str] = []
    if len(cases) < 10:
        errors.append("Qwen parser fixtures must have at least 10 cases")
    qwen2 = [case for case in cases if case["id"] == "QW-PARSE-10"]
    if not qwen2 or qwen2[0]["expected_action"] != "block_native_session":
        errors.append("Qwen2/Qwen2-7B mismatch must block native session")
    required_capabilities = {case.get("required_capability") for case in cases}
    for capability in ["qwen3_reasoning_parser", "qwen3_coder_tool_parser", "qwen_chat_template"]:
        if capability not in required_capabilities:
            errors.append(f"missing Qwen capability fixture: {capability}")
    low_conf_exec = [
        case["id"]
        for case in cases
        if case.get("confidence") == "low" and "execute" in case.get("expected_action", "")
    ]
    if low_conf_exec:
        errors.append(f"low-confidence Qwen repairs execute: {low_conf_exec}")
    return errors


def main() -> int:
    errors = validate_deepseek(load("deepseek")) + validate_qwen(load("qwen"))
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("parser fixture policy checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

