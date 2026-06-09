#!/usr/bin/env python3
"""Usable local ResearchCode agent runner.

This runner is intentionally standard-library only. It gives the repository a
real end-to-end coding loop before the full Tauri/Rust runtime is finished:

- DeepSeek native mode through configurable DeepSeek endpoint/env.
- Qwen3.6-27B native mode through configurable Qwen endpoint/env.
- Compatible OpenAI-style endpoint for manual testing.
- Mock provider for deterministic local tests.
- Tools: file.read, search.ripgrep, patch.apply, shell.command, finish.
- Safety: sensitive path deny, stale/ambiguous patch deny, shell classifier.
- Audit: JSONL events and artifacts under runs/.
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


DENY_PATH_PARTS = [".env", ".ssh", "id_rsa", "id_ed25519", "private_key"]
DENY_COMMAND_PARTS = [
    "rm -rf",
    "curl ",
    "wget ",
    ".env",
    "~/.ssh",
    "id_rsa",
    "id_ed25519",
    "git push",
    "--force",
    "sudo ",
    "chmod 777",
]
ALLOW_COMMAND_PREFIXES = [
    ("rg",),
    ("find",),
    ("ls",),
    ("wc",),
    ("cargo", "test"),
    ("npm", "test"),
    ("python3", "scripts/validate_kernel_schemas.py"),
    ("python3", "scripts/check_all.py"),
]
PACKAGE_INSTALL_PREFIXES = [
    ("npm", "install"),
    ("pnpm", "install"),
    ("yarn", "add"),
    ("pip", "install"),
    ("pip3", "install"),
    ("python", "-m", "pip", "install"),
    ("python3", "-m", "pip", "install"),
    ("cargo", "install"),
]


def now_id() -> str:
    return time.strftime("%Y%m%d_%H%M%S")


def stable_hash(text: str) -> str:
    value = 0xCBF29CE484222325
    for byte in text.encode("utf-8"):
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"fnv64_{value:016x}"


def is_sensitive_path(path: str) -> bool:
    lowered = path.lower()
    return any(part in lowered for part in DENY_PATH_PARTS)


def safe_join(cwd: Path, path: str) -> Path:
    candidate = (cwd / path).resolve()
    root = cwd.resolve()
    if root != candidate and root not in candidate.parents:
        raise ValueError(f"path escapes workspace: {path}")
    if is_sensitive_path(str(candidate)):
        raise ValueError(f"sensitive path denied: {path}")
    return candidate


def classify_command(command: str) -> str:
    lowered = command.lower()
    if any(part in lowered for part in DENY_COMMAND_PARTS):
        return "deny"
    if any(meta in command for meta in [";", "&&", "||", "$(", "`"]):
        return "deny"
    tokens = shlex.split(command)
    if not tokens:
        return "deny"
    if any(tuple(tokens[: len(prefix)]) == prefix for prefix in PACKAGE_INSTALL_PREFIXES):
        return "ask_package_install"
    if any(tuple(tokens[: len(prefix)]) == prefix for prefix in ALLOW_COMMAND_PREFIXES):
        return "allow"
    return "ask"


@dataclass
class EventLog:
    path: Path
    project_id: str = "local"
    session_id: str = field(default_factory=lambda: f"sess_{now_id()}")
    sequence: int = 0
    prev_hash: str | None = None

    def append(self, event_type: str, actor: str, payload: dict[str, Any]) -> None:
        self.sequence += 1
        event_hash = stable_hash(f"{self.session_id}:{self.sequence}:{event_type}:{json.dumps(payload, sort_keys=True)}")
        event = {
            "event_id": f"evt_{self.sequence:04d}",
            "schema_version": "v0",
            "project_id": self.project_id,
            "session_id": self.session_id,
            "task_id": "task_local",
            "sequence": self.sequence,
            "event_type": event_type,
            "actor": actor,
            "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "payload": payload,
            "prev_hash": self.prev_hash,
            "hash": event_hash,
        }
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(event, ensure_ascii=False, sort_keys=True) + "\n")
        self.prev_hash = event_hash


class ToolRuntime:
    def __init__(self, cwd: Path, run_dir: Path, event_log: EventLog, auto_apply: bool) -> None:
        self.cwd = cwd.resolve()
        self.run_dir = run_dir
        self.event_log = event_log
        self.auto_apply = auto_apply
        self.artifact_dir = run_dir / "artifacts"
        self.artifact_dir.mkdir(parents=True, exist_ok=True)

    def run_tool(self, name: str, args: dict[str, Any]) -> dict[str, Any]:
        self.event_log.append("tool.call_requested", "agent", {"tool_id": name, "arguments": redact_args(args)})
        try:
            if name == "file.read":
                result = self.file_read(args)
            elif name == "search.ripgrep":
                result = self.search(args)
            elif name == "patch.apply":
                result = self.patch_apply(args)
            elif name == "shell.command":
                result = self.shell_command(args)
            elif name == "finish":
                result = {"ok": True, "final": str(args.get("message", ""))}
            else:
                result = {"ok": False, "error": f"unknown tool: {name}"}
        except Exception as error:  # noqa: BLE001 - boundary should return tool result
            result = {"ok": False, "error": str(error)}
        self.event_log.append("tool.call_completed", "tool", {"tool_id": name, "ok": bool(result.get("ok")), "result": preview(result)})
        return result

    def file_read(self, args: dict[str, Any]) -> dict[str, Any]:
        path = safe_join(self.cwd, str(args["path"]))
        max_bytes = int(args.get("max_bytes", 8000))
        data = path.read_bytes()
        truncated = len(data) > max_bytes
        text = data[:max_bytes].decode("utf-8", errors="replace")
        return {
            "ok": True,
            "path": str(path.relative_to(self.cwd)),
            "content": text,
            "truncated": truncated,
            "size_bytes": len(data),
            "content_hash": stable_hash(path.read_text(encoding="utf-8", errors="replace")),
        }

    def search(self, args: dict[str, Any]) -> dict[str, Any]:
        root = safe_join(self.cwd, str(args.get("root", ".")))
        pattern = str(args["pattern"])
        max_results = int(args.get("max_results", 50))
        if not pattern:
            return {"ok": False, "error": "empty search pattern"}
        matches: list[dict[str, Any]] = []
        for path in root.rglob("*"):
            if len(matches) >= max_results:
                break
            if path.is_dir() or any(part in path.parts for part in [".git", "target", "node_modules", ".venv", "__pycache__"]):
                continue
            if is_sensitive_path(str(path)):
                continue
            try:
                lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
            except OSError:
                continue
            for index, line in enumerate(lines, start=1):
                if pattern in line:
                    matches.append({"path": str(path.relative_to(self.cwd)), "line": index, "text": line[:500]})
                    if len(matches) >= max_results:
                        break
        artifact = self.write_artifact("search_result", {"pattern": pattern, "matches": matches})
        return {"ok": True, "matches": matches, "artifact": artifact}

    def patch_apply(self, args: dict[str, Any]) -> dict[str, Any]:
        path = safe_join(self.cwd, str(args["path"]))
        old_string = str(args.get("old_string", ""))
        new_string = str(args.get("new_string", ""))
        base_hash = str(args.get("base_hash", ""))
        if path.exists():
            current = path.read_text(encoding="utf-8", errors="replace")
            current_hash = stable_hash(current)
            if base_hash and base_hash != "__compute__" and base_hash != current_hash:
                return {"ok": False, "error": "stale_file", "current_hash": current_hash}
            if not old_string:
                return {"ok": False, "error": "create_requested_but_file_exists"}
            count = current.count(old_string)
            if count == 0:
                return {"ok": False, "error": "old_string_missing", "current_hash": current_hash}
            if count > 1:
                return {"ok": False, "error": "old_string_ambiguous", "count": count, "current_hash": current_hash}
            proposed = current.replace(old_string, new_string, 1)
        else:
            if old_string:
                return {"ok": False, "error": "file_missing_for_replace"}
            current_hash = ""
            proposed = new_string
        diff_artifact = self.write_artifact(
            "patch_preview",
            {"path": str(path.relative_to(self.cwd)), "old_string": old_string, "new_string": new_string, "base_hash": base_hash or current_hash},
        )
        if not self.auto_apply:
            return {"ok": False, "error": "patch_requires_auto_apply", "artifact": diff_artifact}
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(proposed, encoding="utf-8")
        return {"ok": True, "path": str(path.relative_to(self.cwd)), "artifact": diff_artifact, "new_hash": stable_hash(proposed)}

    def shell_command(self, args: dict[str, Any]) -> dict[str, Any]:
        command = str(args["command"])
        decision = classify_command(command)
        if decision != "allow":
            return {"ok": False, "error": f"command_not_allowed:{decision}", "decision": decision}
        completed = subprocess.run(
            shlex.split(command),
            cwd=self.cwd,
            text=True,
            capture_output=True,
            timeout=int(args.get("timeout_seconds", 60)),
            check=False,
        )
        artifact = self.write_artifact(
            "command_output",
            {"command": command, "exit_code": completed.returncode, "stdout": completed.stdout, "stderr": completed.stderr},
        )
        return {
            "ok": completed.returncode == 0,
            "exit_code": completed.returncode,
            "stdout": completed.stdout[-4000:],
            "stderr": completed.stderr[-4000:],
            "artifact": artifact,
        }

    def write_artifact(self, kind: str, payload: dict[str, Any]) -> str:
        text = json.dumps({"artifact_kind": kind, **payload}, ensure_ascii=False, indent=2, sort_keys=True)
        content_hash = stable_hash(text)
        path = self.artifact_dir / f"{kind}_{content_hash}.json"
        path.write_text(text, encoding="utf-8")
        return str(path.relative_to(self.run_dir))


def redact_args(args: dict[str, Any]) -> dict[str, Any]:
    redacted = dict(args)
    for key in list(redacted):
        if any(secret in key.lower() for secret in ["key", "token", "password", "secret"]):
            redacted[key] = "<redacted>"
    return redacted


def preview(value: Any, max_chars: int = 1200) -> Any:
    text = json.dumps(value, ensure_ascii=False, sort_keys=True)
    if len(text) <= max_chars:
        return value
    return {"preview": text[:max_chars], "truncated": True}


def sanitize_model_payload(value: Any) -> Any:
    if isinstance(value, dict):
        return {key: sanitize_model_payload(item) for key, item in value.items()}
    if isinstance(value, list):
        return [sanitize_model_payload(item) for item in value]
    if isinstance(value, str):
        return redact_sensitive_text(value)
    return value


def redact_sensitive_text(text: str) -> str:
    redacted = text.replace(".env", "[REDACTED_PATH]")
    output = ""
    rest = redacted
    while True:
        index = rest.find("sk-")
        if index == -1:
            output += rest
            return output
        output += rest[:index] + "[REDACTED_SECRET]"
        after = rest[index + 3 :]
        end = 0
        while end < len(after) and not after[end].isspace() and after[end] not in {'"', "'", ",", "}"}:
            end += 1
        rest = after[end:]


def adapter_id_for_provider(provider: str) -> str:
    if provider == "deepseek":
        return "deepseek-v4-native"
    if provider == "qwen":
        return "qwen3-6-27b-native"
    if provider == "mock":
        return "mock-provider"
    return "compatible-provider"


def display_model_for_provider(provider: str, args: argparse.Namespace) -> str:
    if args.model:
        return args.model
    if provider == "deepseek":
        return os.environ.get("DEEPSEEK_MODEL", "deepseek-v4-flash")
    if provider == "qwen":
        return os.environ.get("QWEN_MODEL", "Qwen/Qwen3.6-27B")
    if provider == "mock":
        return "mock-model"
    return os.environ.get("COMPATIBLE_MODEL", "compatible-model")


def system_prompt(provider: str) -> str:
    native = "DeepSeek" if provider == "deepseek" else "Qwen3.6-27B" if provider == "qwen" else "compatible model"
    return f"""You are ResearchCode Coworker running in {native} mode.
Return either native OpenAI-style tool_calls or a JSON object:
{{"tool_calls":[{{"name":"file.read","arguments":{{...}}}}]}}
Available tools:
- file.read: {{"path":"relative/path","max_bytes":8000}}
- search.ripgrep: {{"root":".","pattern":"text","max_results":20}}
- patch.apply: {{"path":"relative/path","old_string":"exact unique text","new_string":"replacement","base_hash":"hash from file.read"}}
- shell.command: {{"command":"cargo test --workspace","timeout_seconds":60}}
- finish: {{"message":"final answer"}}
Rules:
- Read files before patching them.
- Patches must be small and exact; use base_hash from file.read.
- Do not request package install, network, secrets, or destructive commands.
- Prefer search/file.read before editing.
- Finish with concise summary and tests run."""


def build_messages(provider: str, task: str, tool_results: list[dict[str, Any]]) -> list[dict[str, str]]:
    messages = [
        {"role": "system", "content": system_prompt(provider)},
        {"role": "user", "content": task},
    ]
    if tool_results:
        messages.append({"role": "user", "content": "Tool results:\n" + json.dumps(tool_results, ensure_ascii=False, indent=2)})
    return messages


def endpoint_config(provider: str, args: argparse.Namespace) -> tuple[str, str, str | None]:
    if provider == "deepseek":
        url = args.base_url or os.environ.get(
            "DEEPSEEK_ANTHROPIC_BASE_URL",
            os.environ.get("DEEPSEEK_BASE_URL", "https://api.deepseek.com/anthropic"),
        )
        model = args.model or os.environ.get("DEEPSEEK_MODEL", "deepseek-v4-flash")
        key = os.environ.get("DEEPSEEK_API_KEY")
        return url, model, key
    if provider == "qwen":
        url = args.base_url or os.environ.get("QWEN_BASE_URL")
        model = args.model or os.environ.get("QWEN_MODEL", "Qwen/Qwen3.6-27B")
        key = os.environ.get("QWEN_API_KEY")
        if not url:
            raise ValueError("Qwen requires --base-url or QWEN_BASE_URL for your Qwen3.6-27B endpoint")
        return url, model, key
    if provider == "compatible":
        url = args.base_url or os.environ.get("COMPATIBLE_BASE_URL")
        model = args.model or os.environ.get("COMPATIBLE_MODEL")
        key = os.environ.get("COMPATIBLE_API_KEY")
        if not url or not model:
            raise ValueError("compatible provider requires --base-url and --model")
        return url, model, key
    raise ValueError(f"unsupported live provider: {provider}")


def call_model(provider: str, args: argparse.Namespace, messages: list[dict[str, str]]) -> dict[str, Any]:
    if provider == "mock":
        if args.mock_response:
            return json.loads(args.mock_response)
        if args.mock_response_file:
            return json.loads(Path(args.mock_response_file).read_text(encoding="utf-8"))
        return {"tool_calls": [{"name": "finish", "arguments": {"message": "mock finished"}}]}
    url, model, api_key = endpoint_config(provider, args)
    if provider == "deepseek":
        return call_deepseek_anthropic(url, model, api_key, args, messages)
    payload = {
        "model": model,
        "messages": messages,
        "temperature": 0.1,
    }
    data = json.dumps(payload).encode("utf-8")
    headers = {"Content-Type": "application/json"}
    if api_key:
        headers["Authorization"] = f"Bearer {api_key}"
    request = urllib.request.Request(url, data=data, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(request, timeout=args.timeout_seconds) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"model HTTP {error.code}: {body}") from error


def call_deepseek_anthropic(
    base_url: str,
    model: str,
    api_key: str | None,
    args: argparse.Namespace,
    messages: list[dict[str, str]],
) -> dict[str, Any]:
    if not api_key:
        raise ValueError("DeepSeek requires DEEPSEEK_API_KEY in the environment")
    system = "\n\n".join(message["content"] for message in messages if message["role"] == "system")
    chat_messages = [
        {
            "role": message["role"],
            "content": [{"type": "text", "text": message["content"]}],
        }
        for message in messages
        if message["role"] != "system"
    ]
    payload = {
        "model": model,
        "max_tokens": 2048,
        "system": system,
        "messages": chat_messages,
        "stream": False,
    }
    url = deepseek_messages_url(base_url)
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        method="POST",
        headers={
            "Content-Type": "application/json",
            "x-api-key": api_key,
            "anthropic-version": "2023-06-01",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=args.timeout_seconds) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"deepseek anthropic HTTP {error.code}: {body}") from error


def deepseek_messages_url(base_url: str) -> str:
    stripped = base_url.rstrip("/")
    if stripped.endswith("/v1/messages"):
        return stripped
    return stripped + "/v1/messages"


def extract_tool_calls(response: dict[str, Any]) -> list[dict[str, Any]]:
    if "tool_calls" in response:
        return normalize_tool_calls(response["tool_calls"])
    choice = (response.get("choices") or [{}])[0]
    message = choice.get("message") or {}
    if message.get("tool_calls"):
        return normalize_tool_calls(message["tool_calls"])
    content = message.get("content") or response.get("content") or ""
    if isinstance(content, list):
        content = "\n".join(str(part.get("text", part)) for part in content)
    parsed = parse_json_from_text(str(content))
    if parsed and "tool_calls" in parsed:
        return normalize_tool_calls(parsed["tool_calls"])
    if parsed and "actions" in parsed:
        return normalize_tool_calls(parsed["actions"])
    return [{"name": "finish", "arguments": {"message": str(content).strip() or "No tool call returned."}}]


def normalize_tool_calls(calls: list[Any]) -> list[dict[str, Any]]:
    normalized = []
    for call in calls:
        if "function" in call:
            function = call["function"]
            arguments = function.get("arguments", {})
            if isinstance(arguments, str):
                arguments = json.loads(arguments or "{}")
            normalized.append({"name": function["name"], "arguments": arguments})
        else:
            arguments = call.get("arguments", {})
            if isinstance(arguments, str):
                arguments = json.loads(arguments or "{}")
            normalized.append({"name": call["name"], "arguments": arguments})
    return normalized


def parse_json_from_text(text: str) -> dict[str, Any] | None:
    stripped = text.strip()
    if stripped.startswith("```"):
        stripped = stripped.strip("`")
        if "\n" in stripped:
            stripped = stripped.split("\n", 1)[1]
    start = stripped.find("{")
    end = stripped.rfind("}")
    if start == -1 or end == -1 or end <= start:
        return None
    try:
        return json.loads(stripped[start : end + 1])
    except json.JSONDecodeError:
        return None


def run(args: argparse.Namespace) -> int:
    cwd = Path(args.cwd).resolve()
    run_dir = cwd / "runs" / f"researchcode_{now_id()}"
    event_log = EventLog(run_dir / "events.jsonl")
    runtime = ToolRuntime(cwd, run_dir, event_log, auto_apply=args.auto_apply)
    event_log.append("session.created", "runtime", {"provider": args.provider, "cwd": str(cwd)})
    tool_results: list[dict[str, Any]] = []
    final_message = ""
    for turn in range(1, args.max_turns + 1):
        call_id = f"model_turn_{turn}"
        event_log.append(
            "model.call_started",
            "runtime",
            {
                "call_id": call_id,
                "provider": args.provider,
                "adapter_id": adapter_id_for_provider(args.provider),
                "actual_model_name": display_model_for_provider(args.provider, args),
                "role": "executor",
                "live": args.provider != "mock",
            },
        )
        response = call_model(args.provider, args, build_messages(args.provider, args.task, tool_results))
        sanitized_response = sanitize_model_payload(response)
        response_text = json.dumps(
            {"artifact_kind": "model_response", **sanitized_response},
            ensure_ascii=False,
            sort_keys=True,
        )
        response_hash = stable_hash(response_text)
        response_artifact = runtime.write_artifact("model_response", sanitized_response)
        event_log.append(
            "model.call_completed",
            "runtime",
            {
                "call_id": call_id,
                "provider": args.provider,
                "ok": True,
                "artifact_id": response_artifact,
                "content_hash": response_hash,
                "preview": preview(sanitized_response),
            },
        )
        calls = extract_tool_calls(response)
        if not calls:
            final_message = "No model action returned."
            break
        turn_results = []
        for call in calls:
            result = runtime.run_tool(call["name"], call.get("arguments", {}))
            turn_results.append({"tool": call["name"], "result": result})
            if call["name"] == "finish":
                final_message = str(result.get("final", ""))
                break
        tool_results.extend(turn_results)
        if final_message:
            break
    if not final_message:
        final_message = f"Stopped after max_turns={args.max_turns}."
    summary = {"final": final_message, "run_dir": str(run_dir), "events": str(event_log.path)}
    (run_dir / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    event_log.append("session.completed", "runtime", summary)
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="ResearchCode local agent runner")
    sub = parser.add_subparsers(dest="command", required=True)
    run_parser = sub.add_parser("run")
    run_parser.add_argument("--provider", choices=["deepseek", "qwen", "compatible", "mock"], required=True)
    run_parser.add_argument("--task", required=True)
    run_parser.add_argument("--cwd", default=".")
    run_parser.add_argument("--model")
    run_parser.add_argument("--base-url")
    run_parser.add_argument("--max-turns", type=int, default=8)
    run_parser.add_argument("--timeout-seconds", type=int, default=120)
    run_parser.add_argument("--auto-apply", action="store_true")
    run_parser.add_argument("--mock-response")
    run_parser.add_argument("--mock-response-file")
    args = parser.parse_args()
    if args.command == "run":
        return run(args)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
