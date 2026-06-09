#!/usr/bin/env python3
"""Validate event logs that intentionally stop at a permission boundary."""

from __future__ import annotations

import json
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: validate_blocked_permission_event_contract.py <events.jsonl>", file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    events = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    event_types = [event.get("event_type") for event in events]

    required = {
        "session.created",
        "session.state_changed",
        "model.call_started",
        "model.stream_delta",
        "model.stream_completed",
        "model.call_completed",
        "tool.call_requested",
        "patch.proposal_created",
        "patch.proposal_validated",
        "permission.requested",
    }
    missing = sorted(required.difference(event_types))
    if missing:
        print(f"missing blocked-permission event types: {missing}", file=sys.stderr)
        return 1

    forbidden = {
        "permission.decided",
        "patch.applied",
        "tool.call_completed",
        "tool.result_recorded",
    }
    present_forbidden = sorted(forbidden.intersection(event_types))
    if present_forbidden:
        print(f"blocked log contains post-approval events: {present_forbidden}", file=sys.stderr)
        return 1

    last = events[-1]
    if last.get("event_type") != "permission.requested":
        print(f"blocked log must end at permission.requested, got {last.get('event_type')}", file=sys.stderr)
        return 1

    state_changes = [
        event
        for event in events
        if event.get("event_type") == "session.state_changed"
        and event.get("payload", {}).get("to_state") == "WaitingForToolApproval"
    ]
    if not state_changes:
        print("blocked log never reached WaitingForToolApproval", file=sys.stderr)
        return 1

    request_type = last.get("payload", {}).get("request_type")
    if request_type not in {"command", "file_write", "network", "cloud_model", "package_install", "protected_path"}:
        print(f"unexpected permission request_type: {request_type}", file=sys.stderr)
        return 1

    print(f"blocked permission event contract passed: events={len(events)} request_type={request_type}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
