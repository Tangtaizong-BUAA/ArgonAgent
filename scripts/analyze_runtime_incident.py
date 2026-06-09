#!/usr/bin/env python3
"""Analyze ResearchCode runtime JSONL logs for long-session loop regressions.

This script intentionally stays dependency-free so it can run in local incident
triage and CI smoke checks. It detects the failure class captured by
runtime_session_1779064870095449000: repeated plan reads, empty line ranges,
too many stream deltas, and multiple native turns for one user instruction.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path


LINE_RANGE_RE = re.compile(r"lines=(\d+)\.\.(\d+)")


def load_events(path: Path) -> list[dict]:
    events: list[dict] = []
    with path.open("r", encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            line = line.strip()
            if not line:
                continue
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError as error:
                raise SystemExit(f"{path}:{line_number}: invalid JSONL: {error}") from error
    return events


def payload(event: dict) -> dict:
    value = event.get("payload")
    return value if isinstance(value, dict) else {}


def is_plan_preview(text: str) -> bool:
    lower = text.lower()
    return "plan/" in lower or "计划" in text or "实施计划" in text


def analyze(events: list[dict]) -> dict:
    event_types = Counter(event.get("event_type", "") for event in events)
    tool_calls = Counter()
    plan_read_previews: list[str] = []
    empty_ranges: list[str] = []
    user_inputs = 0
    native_turns = 0

    for event in events:
        event_type = event.get("event_type", "")
        data = payload(event)
        if event_type == "tool.call_requested":
            tool_id = data.get("tool_id")
            if isinstance(tool_id, str):
                tool_calls[tool_id] += 1
        elif event_type == "tool.result_recorded":
            tool_id = data.get("tool_id")
            preview = data.get("preview", "")
            if isinstance(tool_id, str):
                tool_calls[f"{tool_id}:result"] += 1
            if isinstance(preview, str) and is_plan_preview(preview):
                plan_read_previews.append(preview)
            if isinstance(preview, str):
                match = LINE_RANGE_RE.search(preview)
                if match and int(match.group(1)) > int(match.group(2)):
                    empty_ranges.append(preview)
        elif event_type == "model.stream_delta":
            if data.get("provider") == "user":
                user_inputs += 1
        elif event_type == "agent.turn.started":
            native_turns += 1

    repeated_plan_read_risk = len(plan_read_previews) >= 8
    stream_pressure_risk = event_types["model.stream_delta"] >= 5000
    turn_restart_risk = user_inputs <= 1 and native_turns >= 3
    empty_range_risk = bool(empty_ranges)

    return {
        "event_count": len(events),
        "event_types": dict(event_types.most_common(16)),
        "tool_calls": dict(tool_calls.most_common(16)),
        "user_inputs": user_inputs,
        "native_turns": native_turns,
        "plan_read_count": len(plan_read_previews),
        "empty_range_count": len(empty_ranges),
        "empty_range_examples": empty_ranges[:5],
        "stream_delta_count": event_types["model.stream_delta"],
        "risks": {
            "repeated_plan_reads": repeated_plan_read_risk,
            "empty_file_read_ranges": empty_range_risk,
            "stream_delta_pressure": stream_pressure_risk,
            "turn_restart_after_single_user_input": turn_restart_risk,
        },
        "incident_detected": any(
            [
                repeated_plan_read_risk,
                empty_range_risk,
                stream_pressure_risk,
                turn_restart_risk,
            ]
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("jsonl", type=Path)
    parser.add_argument(
        "--fail-on-incident",
        action="store_true",
        help="exit non-zero when the incident signature is detected",
    )
    args = parser.parse_args()

    report = analyze(load_events(args.jsonl))
    print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    if args.fail_on_incident and report["incident_detected"]:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
