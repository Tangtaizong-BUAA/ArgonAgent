#!/usr/bin/env python3
"""Validate model-call event logs for GUI/API consumers."""

from __future__ import annotations

import json
import sys
from pathlib import Path


REQUIRED_EVENT_TYPES = {
    "session.created",
    "session.state_changed",
    "model.call_started",
    "model.stream_delta",
    "model.stream_completed",
    "model.call_completed",
}


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: validate_model_event_contract.py <events.jsonl>", file=sys.stderr)
        return 2
    events = [
        json.loads(line)
        for line in Path(argv[1]).read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    event_types = {event.get("event_type") for event in events}
    missing = sorted(REQUIRED_EVENT_TYPES - event_types)
    if missing:
        print(f"missing model event types: {missing}", file=sys.stderr)
        return 1
    raw_dump = json.dumps(events, ensure_ascii=False, sort_keys=True)
    for forbidden in ["sk-testsecret", ".env", "api_key"]:
        if forbidden in raw_dump:
            print(f"model event log leaked forbidden text: {forbidden}", file=sys.stderr)
            return 1
    completed = [event for event in events if event.get("event_type") == "model.stream_completed"]
    started = [event for event in events if event.get("event_type") == "model.call_started"]
    providers = {event.get("payload", {}).get("provider") for event in completed}
    if not {"deepseek", "qwen"}.issubset(providers):
        print(f"expected deepseek and qwen stream completions, got {providers}", file=sys.stderr)
        return 1
    for event in completed:
        payload = event.get("payload", {})
        if not str(payload.get("content_hash", "")).startswith("fnv64_"):
            print(f"invalid model transcript hash: {payload}", file=sys.stderr)
            return 1
    for event in started:
        payload = event.get("payload", {})
        for key in [
            "scaffold_level",
            "prompt_tokens_estimate",
            "prompt_hash",
            "tool_catalog_hash",
            "max_context_tokens",
            "prompt_scaffold_budget",
            "dynamic_context_budget",
            "protected_reserve_tokens",
        ]:
            if key not in payload:
                print(f"missing model.call_started telemetry {key}: {payload}", file=sys.stderr)
                return 1
        if payload.get("provider") in {"deepseek", "qwen"} and payload.get("scaffold_level") == "unknown":
            print(f"native model.call_started lacks scaffold level: {payload}", file=sys.stderr)
            return 1
        if payload.get("provider") in {"deepseek", "qwen"} and int(payload.get("protected_reserve_tokens", 0)) <= 0:
            print(f"native model.call_started lacks protected reserve: {payload}", file=sys.stderr)
            return 1
    print(f"model event contract passed: events={len(events)} streams={len(completed)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
