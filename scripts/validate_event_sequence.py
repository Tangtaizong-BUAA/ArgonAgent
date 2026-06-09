#!/usr/bin/env python3
"""Validate a Phase 0 kernel event JSONL replay file."""

from __future__ import annotations

import json
import sys
from pathlib import Path


REQUIRED_EVENTS = {
    "session.created",
    "message.user_created",
    "plan.proposed",
    "plan.approval_requested",
    "plan.approval_decided",
    "permission.requested",
    "permission.decided",
    "patch.proposed",
    "patch.applied",
    "artifact.created",
    "eval.event",
    "session.completed",
}


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: validate_event_sequence.py <events.jsonl>", file=sys.stderr)
        return 2
    path = Path(sys.argv[1])
    events = []
    with path.open("r", encoding="utf-8") as handle:
        for idx, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError as exc:
                print(f"{path}:{idx}: invalid JSON: {exc}", file=sys.stderr)
                return 1
    errors: list[str] = []
    seen_types = {event.get("event_type") for event in events}
    missing = sorted(REQUIRED_EVENTS - seen_types)
    if missing:
        errors.append(f"missing required event types: {missing}")
    last_sequence = 0
    last_hash = None
    for event in events:
        sequence = event.get("sequence")
        if not isinstance(sequence, int) or sequence <= last_sequence:
            errors.append(f"bad sequence at {event.get('event_id')}: {sequence}")
        last_sequence = sequence if isinstance(sequence, int) else last_sequence
        if last_hash and event.get("prev_hash") != last_hash:
            errors.append(f"bad prev_hash at {event.get('event_id')}")
        last_hash = event.get("hash")
        if event.get("event_type") == "permission.requested":
            payload = event.get("payload", {})
            if payload.get("request_type") == "plan":
                errors.append("plan approval must not be represented as PermissionRequest")
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(f"event sequence valid: {len(events)} events")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

