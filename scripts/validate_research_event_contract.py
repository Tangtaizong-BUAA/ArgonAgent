#!/usr/bin/env python3
"""Validate Research Coworker event logs for GUI/API consumers."""

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
    "tool.call_requested",
    "tool.call_completed",
    "tool.result_recorded",
}


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: validate_research_event_contract.py <events.jsonl>", file=sys.stderr)
        return 2
    events = [
        json.loads(line)
        for line in Path(argv[1]).read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    event_types = {event.get("event_type") for event in events}
    missing = sorted(REQUIRED_EVENT_TYPES - event_types)
    if missing:
        print(f"missing research event types: {missing}", file=sys.stderr)
        return 1
    raw_dump = json.dumps(events, ensure_ascii=False, sort_keys=True)
    for forbidden in ["sk-testsecret", ".env", "api_key"]:
        if forbidden in raw_dump:
            print(f"research event log leaked forbidden text: {forbidden}", file=sys.stderr)
            return 1
    tool_ids = {
        event.get("payload", {}).get("tool_id")
        for event in events
        if event.get("event_type") == "tool.call_completed"
    }
    if "research.csv_profile" not in tool_ids:
        print(f"missing research.csv_profile completion: {tool_ids}", file=sys.stderr)
        return 1
    stream_completed = [
        event for event in events if event.get("event_type") == "model.stream_completed"
    ]
    if not stream_completed:
        print("missing model stream completion", file=sys.stderr)
        return 1
    providers = {event.get("payload", {}).get("provider") for event in stream_completed}
    if providers - {"deepseek", "qwen"}:
        print(f"unexpected provider set: {providers}", file=sys.stderr)
        return 1
    for event in stream_completed:
        payload = event.get("payload", {})
        if not str(payload.get("content_hash", "")).startswith("fnv64_"):
            print(f"invalid model transcript hash: {payload}", file=sys.stderr)
            return 1
    tool_results = [
        event for event in events if event.get("event_type") == "tool.result_recorded"
    ]
    for event in tool_results:
        payload = event.get("payload", {})
        for key in ["tool_call_id", "tool_id", "artifact_id", "content_hash", "preview"]:
            if key not in payload:
                print(f"tool.result_recorded missing {key}: {event}", file=sys.stderr)
                return 1
        if not str(payload.get("content_hash", "")).startswith("fnv64_"):
            print(f"invalid tool result hash: {payload}", file=sys.stderr)
            return 1
    print(
        f"research event contract passed: events={len(events)} tool_results={len(tool_results)} streams={len(stream_completed)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
