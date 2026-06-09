#!/usr/bin/env python3
"""Generate a local fixture bundle for the static GUI/API.

This is a no-network developer entrypoint. It runs deterministic Rust runtime
fixtures and writes event logs plus a manifest under `runs/`, so the current
product shell can be inspected without hand-assembling paths.
"""

from __future__ import annotations

import json
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def run(command: list[str]) -> None:
    completed = subprocess.run(command, cwd=ROOT, check=False)
    if completed.returncode != 0:
        raise SystemExit(completed.returncode)


def main() -> int:
    run_id = time.strftime("%Y%m%d_%H%M%S")
    run_dir = ROOT / "runs" / f"dev_fixture_bundle_{run_id}"
    run_dir.mkdir(parents=True, exist_ok=False)
    coding_events = run_dir / "coding_events.jsonl"
    model_events = run_dir / "model_events.jsonl"
    agent_loop_events = run_dir / "agent_loop_events.jsonl"
    live_transport_agent_loop_events = run_dir / "live_transport_agent_loop_events.jsonl"
    blocked_permission_events = run_dir / "blocked_permission_events.jsonl"
    research_events = run_dir / "research_events.jsonl"

    run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "researchcode-cli",
            "--",
            "coding-fixture-eventlog",
            str(coding_events),
        ]
    )
    run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "researchcode-cli",
            "--",
            "live-model-response-record-eventlog",
            str(model_events),
        ]
    )
    run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "researchcode-cli",
            "--",
            "recorded-agent-loop-eventlog",
            str(agent_loop_events),
        ]
    )
    run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "researchcode-cli",
            "--",
            "live-transport-agent-loop-eventlog",
            str(live_transport_agent_loop_events),
        ]
    )
    run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "researchcode-cli",
            "--",
            "native-agent-loop-blocked-eventlog",
            str(blocked_permission_events),
        ]
    )
    run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "researchcode-cli",
            "--",
            "recorded-research-loop-eventlog",
            str(research_events),
        ]
    )
    run(["cargo", "run", "-q", "-p", "researchcode-cli", "--", "validate-event-log", str(coding_events)])
    run(["cargo", "run", "-q", "-p", "researchcode-cli", "--", "validate-event-log", str(model_events)])
    run(["cargo", "run", "-q", "-p", "researchcode-cli", "--", "validate-event-log", str(agent_loop_events)])
    run(["cargo", "run", "-q", "-p", "researchcode-cli", "--", "validate-event-log", str(live_transport_agent_loop_events)])
    run(["cargo", "run", "-q", "-p", "researchcode-cli", "--", "validate-event-log", str(blocked_permission_events)])
    run(["cargo", "run", "-q", "-p", "researchcode-cli", "--", "validate-event-log", str(research_events)])
    run(["python3", "scripts/validate_runtime_event_contract.py", str(coding_events)])
    run(["python3", "scripts/validate_runtime_event_contract.py", str(agent_loop_events)])
    run(["python3", "scripts/validate_runtime_event_contract.py", str(live_transport_agent_loop_events)])
    run(["python3", "scripts/validate_blocked_permission_event_contract.py", str(blocked_permission_events)])
    run(["python3", "scripts/validate_model_event_contract.py", str(model_events)])
    run(["python3", "scripts/validate_model_event_contract.py", str(agent_loop_events)])
    run(["python3", "scripts/validate_model_event_contract.py", str(live_transport_agent_loop_events)])
    run(["python3", "scripts/validate_research_event_contract.py", str(research_events)])

    manifest = {
        "run_id": run_id,
        "run_dir": str(run_dir),
        "agent_loop_events": str(agent_loop_events),
        "blocked_permission_events": str(blocked_permission_events),
        "coding_events": str(coding_events),
        "live_transport_agent_loop_events": str(live_transport_agent_loop_events),
        "model_events": str(model_events),
        "research_events": str(research_events),
        "local_api": {
            "start": "cargo run -p researchcode-cli -- local-api-server 8765",
            "model_timeline": f"http://127.0.0.1:8765/model-timeline?path={live_transport_agent_loop_events.relative_to(ROOT)}",
            "session_snapshot": f"http://127.0.0.1:8765/session-snapshot?path={live_transport_agent_loop_events.relative_to(ROOT)}",
            "blocked_permission_snapshot": f"http://127.0.0.1:8765/session-snapshot?path={blocked_permission_events.relative_to(ROOT)}",
            "coding_summary": f"http://127.0.0.1:8765/summary?path={live_transport_agent_loop_events.relative_to(ROOT)}",
            "blocked_permission_summary": f"http://127.0.0.1:8765/summary?path={blocked_permission_events.relative_to(ROOT)}",
            "research_summary": f"http://127.0.0.1:8765/summary?path={research_events.relative_to(ROOT)}",
        },
        "static_gui": {
            "open": (
                "desktop/static_mock.html?"
                "api=http://127.0.0.1:8765"
                f"&events={live_transport_agent_loop_events.relative_to(ROOT)}"
            )
        },
    }
    manifest_path = run_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    latest_path = ROOT / "runs" / "latest_dev_fixture_bundle.json"
    latest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "manifest": str(manifest_path)}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
