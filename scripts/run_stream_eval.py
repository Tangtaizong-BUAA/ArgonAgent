#!/usr/bin/env python3
"""Executable DeepSeek/Qwen stream parser eval gate.

This mirrors the Rust stream parser contracts with standard-library Python so
fixture promotion is independent from unit tests.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]


def load_cases(name: str) -> list[dict[str, Any]]:
    return json.loads((ROOT / "eval" / "fixtures" / name / "stream_golden.json").read_text(encoding="utf-8"))


def sanitize(value: str) -> str:
    value = re.sub(r"sk-[A-Za-z0-9_-]+", "[REDACTED_SECRET]", value)
    value = re.sub(r"AKIA[A-Za-z0-9_-]+", "[REDACTED_SECRET]", value)
    return value.replace(".env", "[REDACTED_PATH]")


def json_string(payload: str, key: str) -> str | None:
    marker = f'"{key}":'
    index = payload.find(marker)
    if index == -1:
        return None
    rest = payload[index + len(marker) :].lstrip()
    if rest.startswith("null") or not rest.startswith('"'):
        return None
    rest = rest[1:]
    out: list[str] = []
    escaped = False
    for char in rest:
        if escaped:
            out.append({"n": "\n", "t": "\t", '"': '"', "\\": "\\"}.get(char, char))
            escaped = False
        elif char == "\\":
            escaped = True
        elif char == '"':
            return "".join(out)
        else:
            out.append(char)
    return None


def json_u64(payload: str, key: str) -> int | None:
    marker = f'"{key}":'
    index = payload.find(marker)
    if index == -1:
        return None
    rest = payload[index + len(marker) :].lstrip()
    digits = []
    for char in rest:
        if not char.isdigit():
            break
        digits.append(char)
    return int("".join(digits)) if digits else None


def payload_from_line(line: str) -> str:
    line = line.strip()
    if line.startswith("data:"):
        return line.removeprefix("data:").strip()
    return line


def parse_deepseek(lines: list[str]) -> tuple[dict[str, Any], str | None]:
    result: dict[str, Any] = {
        "valid": True,
        "reasoning_sanitized": "",
        "content": "",
        "tool_name": None,
        "tool_arguments": "",
        "done": False,
        "telemetry": {},
    }
    for line in lines:
        payload = payload_from_line(line)
        if payload == "[DONE]":
            result["done"] = True
        elif '"reasoning_content"' in payload:
            result["reasoning_sanitized"] += sanitize(json_string(payload, "reasoning_content") or "")
        elif '"tool_calls"' in payload:
            name = json_string(payload, "name")
            if name:
                result["tool_name"] = name
            result["tool_arguments"] += json_string(payload, "arguments") or ""
        elif '"usage"' in payload:
            for key in [
                "prompt_tokens",
                "completion_tokens",
                "reasoning_tokens",
                "prompt_cache_hit_tokens",
                "prompt_cache_miss_tokens",
            ]:
                value = json_u64(payload, key)
                if value is not None:
                    result["telemetry"][key] = value
        elif '"content"' in payload:
            result["content"] += json_string(payload, "content") or ""
    return result, None


def parse_qwen(lines: list[str]) -> tuple[dict[str, Any], str | None]:
    result: dict[str, Any] = {
        "valid": True,
        "deployment_model": None,
        "thinking_sanitized": "",
        "content": "",
        "tool_name": None,
        "tool_arguments": "",
        "done": False,
        "telemetry": {},
    }
    for line in lines:
        payload = payload_from_line(line)
        model = json_string(payload, "model")
        if model:
            result["deployment_model"] = model
        if payload == "[DONE]":
            result["done"] = True
        elif '"reasoning_content"' in payload or '"thinking"' in payload:
            result["thinking_sanitized"] += sanitize(
                json_string(payload, "reasoning_content") or json_string(payload, "thinking") or ""
            )
        elif '"tool_calls"' in payload:
            name = json_string(payload, "name")
            if name:
                result["tool_name"] = name
            result["tool_arguments"] += json_string(payload, "arguments") or ""
        elif '"usage"' in payload:
            for key in ["prompt_tokens", "completion_tokens", "total_tokens"]:
                value = json_u64(payload, key)
                if value is not None:
                    result["telemetry"][key] = value
        elif '"content"' in payload:
            result["content"] += json_string(payload, "content") or ""
    model = result.get("deployment_model")
    if model and "Qwen3.6-27B" not in str(model):
        result["valid"] = False
        return result, "Qwen native stream requires Qwen3.6-27B deployment"
    return result, None


def compare_case(family: str, case: dict[str, Any]) -> list[str]:
    parsed, error = (parse_deepseek if family == "deepseek" else parse_qwen)(case["lines"])
    expected = case["expected"]
    errors: list[str] = []
    if expected.get("valid") is False:
        if parsed.get("valid") is not False:
            errors.append(f"{case['id']} expected invalid stream")
        if expected.get("error_contains") and expected["error_contains"] not in str(error):
            errors.append(f"{case['id']} error {error!r} does not contain {expected['error_contains']!r}")
        return errors
    if error:
        return [f"{case['id']} unexpected error {error}"]
    for key, value in expected.items():
        if key == "telemetry":
            for telemetry_key, telemetry_value in value.items():
                if parsed["telemetry"].get(telemetry_key) != telemetry_value:
                    errors.append(
                        f"{case['id']} telemetry {telemetry_key} {parsed['telemetry'].get(telemetry_key)!r} != {telemetry_value!r}"
                    )
        elif parsed.get(key) != value:
            errors.append(f"{case['id']} {key} {parsed.get(key)!r} != {value!r}")
    raw_dump = json.dumps(parsed, sort_keys=True)
    if "sk-testsecret" in raw_dump or ".env" in raw_dump:
        errors.append(f"{case['id']} leaked raw secret/path in parsed stream")
    return errors


def main() -> int:
    errors: list[str] = []
    for family in ["deepseek", "qwen"]:
        for case in load_cases(family):
            errors.extend(compare_case(family, case))
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("stream parser eval passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
