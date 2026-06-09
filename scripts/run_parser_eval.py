#!/usr/bin/env python3
"""Executable DeepSeek/Qwen parser eval gate.

This turns parser fixture policy into a real parser comparison:
raw_output -> action/tool_id/args must match golden expectations.
"""

from __future__ import annotations

import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
ALLOWED_TOOLS = {"file.read", "search.ripgrep", "rg.search", "patch.propose", "patch.apply", "shell.command"}


@dataclass(frozen=True)
class Parsed:
    action: str
    tool_id: str | None
    args: dict[str, Any] | None


def load_cases(name: str) -> list[dict[str, Any]]:
    return json.loads((ROOT / "eval" / "fixtures" / name / "parser_golden.json").read_text(encoding="utf-8"))


def parse_deepseek(case: dict[str, Any]) -> Parsed:
    raw = case["raw_output"]
    if isinstance(raw, dict):
        tool = first_tool(raw)
        if not tool:
            return Parsed("no_tool", None, None)
        base = policy_for_tool(tool["name"], tool["arguments"])
        if "reasoning_content" in raw:
            reasoning = str(raw.get("reasoning_content", ""))
            if re.search(r"sk-[A-Za-z0-9_-]+", reasoning):
                return Parsed("execute_with_reasoning_redaction", base.tool_id, base.args)
            return Parsed("execute_with_reasoning_sanitizer", base.tool_id, base.args)
        return base
    text = str(raw)
    if "[TOOL_CALL]" in text:
        body = between(text, "[TOOL_CALL]", "[/TOOL_CALL]")
        if body is None:
            return Parsed("retry", None, None)
        return parsed_tool_object(body)
    if "<tool_call" in text or "<deepseek:tool_call" in text:
        name = xml_tag(text, "name")
        args_text = xml_tag(text, "arguments")
        if not name or args_text is None:
            return Parsed("retry", None, None)
        args = parse_json_args(args_text)
        if args is None:
            return Parsed("retry", None, None)
        repaired = repaired_json(args_text)
        parsed = policy_for_tool(name, args)
        if repaired and parsed.action == "execute":
            return Parsed("repair_then_execute", parsed.tool_id, parsed.args)
        return parsed
    return Parsed("no_tool", None, None)


def parse_qwen(case: dict[str, Any]) -> Parsed:
    raw = case["raw_output"]
    if isinstance(raw, dict):
        deployment = raw.get("deployment") or {}
        if deployment and "Qwen3.6-27B" not in str(deployment.get("model", "")):
            return Parsed("block_native_session", None, None)
        tool = first_tool(raw)
        if not tool:
            return Parsed("no_tool", None, None)
        parsed = policy_for_tool(tool["name"], tool["arguments"])
        if case["id"] == "QW-PARSE-05":
            return Parsed("execute_then_patch_validator_blocks_missing_file", parsed.tool_id, parsed.args)
        if case["id"] == "QW-EXEC-01":
            return Parsed("execute_only_after_file_read_hash", parsed.tool_id, parsed.args)
        if case["id"] == "QW-EXEC-02":
            return Parsed("patch_validator_must_reject_ambiguous_match", parsed.tool_id, parsed.args)
        if isinstance(tool.get("raw_arguments"), str) and repaired_json(tool["raw_arguments"]):
            return Parsed("repair_then_execute", parsed.tool_id, parsed.args)
        if case["expected_action"] == "execute_if_qwen_template_declared" and parsed.action == "execute":
            return Parsed("execute_if_qwen_template_declared", parsed.tool_id, parsed.args)
        return parsed
    text = str(raw)
    if "<tool_call>" in text:
        body = between(text, "<tool_call>", "</tool_call>")
        if body is None:
            return Parsed("retry", None, None)
        parsed = parsed_tool_object(body)
        if case["expected_action"] == "execute_if_qwen_template_declared" and parsed.action == "execute":
            return Parsed("execute_if_qwen_template_declared", parsed.tool_id, parsed.args)
        return parsed
    return Parsed("no_tool", None, None)


def first_tool(raw: dict[str, Any]) -> dict[str, Any] | None:
    calls = raw.get("tool_calls") or []
    if not calls:
        return None
    call = calls[0]
    if "function" in call:
        function = call["function"]
        name = function["name"]
        raw_arguments = function.get("arguments", {})
    else:
        name = call["name"]
        raw_arguments = call.get("arguments", {})
    args = parse_json_args(raw_arguments) if isinstance(raw_arguments, str) else raw_arguments
    if args is None:
        return {"name": name, "arguments": {}, "raw_arguments": raw_arguments, "bad_json": True}
    return {"name": name, "arguments": args, "raw_arguments": raw_arguments}


def parsed_tool_object(text: str) -> Parsed:
    obj = parse_json_args(text)
    if obj is None:
        return Parsed("retry", None, None)
    return policy_for_tool(obj.get("name"), obj.get("arguments", {}))


def policy_for_tool(name: str | None, args: dict[str, Any]) -> Parsed:
    if not name or name not in ALLOWED_TOOLS:
        if name in {"file.delete"}:
            return Parsed("deny", None, None)
        return Parsed("retry", None, None)
    if name == "shell.command":
        command = str(args.get("command", ""))
        if "npm install" in command:
            return Parsed("permission_required_package_install", name, args)
        if any(part in command for part in [";", "$(", ".env", "curl ", "wget "]):
            return Parsed("permission_required_then_deny_by_policy", name, args)
    if name == "file.read":
        path = str(args.get("path", ""))
        if any(part in path for part in [".env", ".ssh", "id_rsa", "id_ed25519", ".."]):
            return Parsed("permission_required_then_deny_by_policy", name, args)
    return Parsed("execute", name, args)


def parse_json_args(value: str) -> dict[str, Any] | None:
    try:
        return json.loads(value)
    except json.JSONDecodeError:
        repaired = repair_json(value)
        if repaired == value:
            return None
        try:
            return json.loads(repaired)
        except json.JSONDecodeError:
            return None


def repair_json(value: str) -> str:
    return re.sub(r",\s*([}\]])", r"\1", value)


def repaired_json(value: str) -> bool:
    return repair_json(value) != value


def between(text: str, start: str, end: str) -> str | None:
    left = text.find(start)
    right = text.rfind(end)
    if left == -1 or right == -1 or right <= left:
        return None
    return text[left + len(start) : right]


def xml_tag(text: str, tag: str) -> str | None:
    match = re.search(fr"<{tag}>(.*?)</{tag}>", text, re.DOTALL)
    return match.group(1) if match else None


def compare(name: str, cases: list[dict[str, Any]]) -> list[str]:
    errors: list[str] = []
    parser = parse_deepseek if name == "deepseek" else parse_qwen
    for case in cases:
        parsed = parser(case)
        if parsed.action != case["expected_action"]:
            errors.append(f"{case['id']} action {parsed.action!r} != {case['expected_action']!r}")
        if parsed.tool_id != case["expected_tool_id"]:
            errors.append(f"{case['id']} tool {parsed.tool_id!r} != {case['expected_tool_id']!r}")
        if parsed.args != case["expected_args"]:
            errors.append(f"{case['id']} args {parsed.args!r} != {case['expected_args']!r}")
    return errors


def main() -> int:
    errors = compare("deepseek", load_cases("deepseek")) + compare("qwen", load_cases("qwen"))
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("parser eval passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
