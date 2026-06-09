#!/usr/bin/env python3
"""Validate runtime event contract for GUI/API consumers."""

from __future__ import annotations

import json
import sys
from pathlib import Path


REQUIRED_EVENT_TYPES = {
    "session.created",
    "session.state_changed",
    "tool.call_requested",
    "tool.call_completed",
    "tool.result_recorded",
    "permission.requested",
    "permission.decided",
    "patch.proposal_created",
    "patch.proposal_validated",
    "patch.applied",
}


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: validate_runtime_event_contract.py <events.jsonl>", file=sys.stderr)
        return 2
    path = Path(argv[1])
    events = [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    event_types = {event.get("event_type") for event in events}
    missing = sorted(REQUIRED_EVENT_TYPES - event_types)
    if missing:
        print(f"missing event types: {missing}", file=sys.stderr)
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
        if not str(payload["content_hash"]).startswith("fnv64_"):
            print(f"unexpected content hash: {payload}", file=sys.stderr)
            return 1
    stream_events = [
        event for event in events if event.get("event_type") == "model.stream_completed"
    ]
    for event in stream_events:
        payload = event.get("payload", {})
        for key in [
            "stream_id",
            "provider",
            "artifact_id",
            "content_hash",
            "prompt_tokens",
            "completion_tokens",
            "reasoning_tokens",
            "prompt_cache_hit_tokens",
            "prompt_cache_miss_tokens",
        ]:
            if key not in payload:
                print(f"model.stream_completed missing {key}: {event}", file=sys.stderr)
                return 1
        if payload["provider"] not in {"deepseek", "qwen"}:
            print(f"unexpected native stream provider: {payload}", file=sys.stderr)
            return 1
        if not str(payload["content_hash"]).startswith("fnv64_"):
            print(f"unexpected stream content hash: {payload}", file=sys.stderr)
            return 1
    model_call_events = [
        event for event in events if event.get("event_type") == "model.call_completed"
    ]
    for event in model_call_events:
        payload = event.get("payload", {})
        for key in ["call_id", "provider", "ok", "artifact_id", "content_hash"]:
            if key not in payload:
                print(f"model.call_completed missing {key}: {event}", file=sys.stderr)
                return 1
        if not str(payload["content_hash"]).startswith("fnv64_"):
            print(f"unexpected model call content hash: {payload}", file=sys.stderr)
            return 1
    print(
        f"runtime event contract passed: events={len(events)} tool_results={len(tool_results)} model_streams={len(stream_events)} model_calls={len(model_call_events)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
