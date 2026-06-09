#!/usr/bin/env python3
"""Live-native boundary eval.

This eval is safe by default: without RESEARCHCODE_ENABLE_LIVE_PROVIDER=1,
RESEARCHCODE_ALLOW_NETWORK=1, and provider API-key environment variables, it
still exercises provider health checks, native loop preflight, event-log export,
and secret-leak guards. With live access enabled, the same contract becomes the
promotion smoke for DeepSeek/Qwen native transports. Local Ollama Qwen can be
selected with --families qwen --qwen-ollama --require-live.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def run(command: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )


def assert_clean_output(label: str, text: str) -> None:
    forbidden = ["sk-", ".env", "BEGIN PRIVATE KEY", "id_rsa"]
    leaked = [item for item in forbidden if item in text]
    if leaked:
        raise AssertionError(f"{label} leaked forbidden tokens: {leaked}")


def event_log_summary(path: Path) -> dict[str, object]:
    events = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            events.append(json.loads(line))
    assert_clean_output(str(path), path.read_text(encoding="utf-8"))
    event_types = [event.get("event_type") for event in events]
    return {
        "event_count": len(events),
        "has_model_started": "model.call_started" in event_types,
        "has_model_completed": "model.call_completed" in event_types,
        "has_model_blocked": "model.call_blocked" in event_types,
        "has_model_blocked_or_completed": (
            "model.call_blocked" in event_types or "model.call_completed" in event_types
        ),
        "has_tool_pending_or_completed": (
            "agent.tool.pending" in event_types
            or "agent.tool.completed" in event_types
            or "agent.tool.streaming_completed" in event_types
        ),
    }


def parse_provider_health(stdout: str) -> dict[str, str]:
    statuses: dict[str, str] = {}
    for line in stdout.splitlines():
        if not line.startswith("provider health "):
            continue
        label = None
        status = None
        for part in line.split():
            if part.startswith("label="):
                label = part.removeprefix("label=")
            elif part.startswith("status="):
                status = part.removeprefix("status=")
        if label and status:
            statuses[label] = status
    return statuses


def selected_families(raw: str) -> list[str]:
    families = [item.strip() for item in raw.split(",") if item.strip()]
    if not families:
        raise ValueError("--families cannot be empty")
    invalid = [item for item in families if item not in {"deepseek", "qwen"}]
    if invalid:
        raise ValueError(f"unsupported families: {', '.join(invalid)}")
    return families


def main() -> int:
    parser = argparse.ArgumentParser(description="Run DeepSeek/Qwen live-native eval")
    parser.add_argument(
        "--require-live",
        action="store_true",
        help="fail unless real provider calls complete for every native family",
    )
    parser.add_argument(
        "--families",
        default="deepseek,qwen",
        help="comma-separated native families to evaluate: deepseek,qwen",
    )
    parser.add_argument(
        "--qwen-ollama",
        action="store_true",
        help="use local Ollama OpenAI-compatible Qwen endpoint defaults",
    )
    args = parser.parse_args()
    require_live = args.require_live or os.environ.get("RESEARCHCODE_REQUIRE_LIVE_PROVIDER") == "1"
    try:
        families = selected_families(args.families)
    except ValueError as error:
        print(str(error), flush=True)
        return 2

    if args.qwen_ollama:
        os.environ.setdefault("QWEN_BASE_URL", "http://127.0.0.1:11434/v1/chat/completions")
        os.environ.setdefault("QWEN_API_KEY", "local-qwen-ollama")
        os.environ.setdefault("RESEARCHCODE_ENABLE_LIVE_PROVIDER", "1")
        os.environ.setdefault("RESEARCHCODE_ALLOW_NETWORK", "1")

    health = run(["cargo", "run", "-q", "-p", "researchcode-cli", "--", "provider-health-smoke"])
    assert_clean_output("provider-health-smoke", health.stdout + health.stderr)
    if health.returncode != 0:
        print(health.stdout, end="")
        print(health.stderr, end="")
        return health.returncode
    if require_live:
        missing = []
        required_env = ["RESEARCHCODE_ENABLE_LIVE_PROVIDER", "RESEARCHCODE_ALLOW_NETWORK"]
        if "deepseek" in families:
            required_env.append("DEEPSEEK_API_KEY")
        if "qwen" in families:
            required_env.extend(["QWEN_API_KEY", "QWEN_BASE_URL"])
        for name in required_env:
            value = os.environ.get(name, "")
            if not value.strip() or (name.startswith("RESEARCHCODE_") and value != "1"):
                missing.append(name)
        if missing:
            print(
                "live-native eval requires live provider environment: "
                + ", ".join(missing),
                flush=True,
            )
            return 2
        health_status = parse_provider_health(health.stdout)
        unhealthy = {
            family: health_status.get(family, "missing")
            for family in families
            if health_status.get(family) != "healthy"
        }
        if unhealthy:
            print(f"live-native eval requires healthy provider health checks: {unhealthy}", flush=True)
            print(health.stdout, end="")
            return 2

    reports = {"provider_health": health.stdout.strip(), "families": {}, "selected_families": families}
    with tempfile.TemporaryDirectory(prefix="researchcode-live-native-eval-") as tmp:
        root = Path(tmp)
        for family in families:
            event_path = root / f"{family}.jsonl"
            loop = run(
                [
                    "cargo",
                    "run",
                    "-q",
                    "-p",
                    "researchcode-cli",
                    "--",
                    "native-agent-loop-sidecar-live-eventlog",
                    family,
                    str(event_path),
                ]
            )
            assert_clean_output(f"{family}-native-loop", loop.stdout + loop.stderr)
            if loop.returncode != 0:
                print(loop.stdout, end="")
                print(loop.stderr, end="")
                return loop.returncode
            validate = run(
                [
                    "cargo",
                    "run",
                    "-q",
                    "-p",
                    "researchcode-cli",
                    "--",
                    "validate-event-log",
                    str(event_path),
                ]
            )
            if validate.returncode != 0:
                print(validate.stdout, end="")
                print(validate.stderr, end="")
                return validate.returncode
            summary = event_log_summary(event_path)
            if require_live:
                if summary["has_model_blocked"] or not summary["has_model_completed"]:
                    print(
                        f"live-native eval requires completed real model call for {family}; summary={summary}",
                        flush=True,
                    )
                    print(loop.stdout, end="")
                    print(loop.stderr, end="")
                    return 2
            reports["families"][family] = {
                "loop": loop.stdout.strip(),
                "event_log": summary,
            }
    print(json.dumps(reports, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
