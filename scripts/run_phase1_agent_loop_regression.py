#!/usr/bin/env python3
"""Run the deterministic Phase 1 agent-loop regression pack.

This pack intentionally stays local and provider-free. It covers the
active-turn, pending approval, shell hard-deny, and dirty stream-attempt
contracts hardened during the Phase 1 remediation.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


COMMANDS: list[list[str]] = [
    ["cargo", "test", "-p", "researchcode-runtime", "active_turn_"],
    ["cargo", "test", "-p", "researchcode-runtime", "permission"],
    [
        "cargo",
        "test",
        "-p",
        "researchcode-runtime",
        "failed_stream_attempt_does_not_call_structural_event_handler_before_fallback",
    ],
    [
        "cargo",
        "test",
        "-p",
        "researchcode-runtime",
        "continuation_stream_fallback_discards_failed_attempt_structural_events",
    ],
    [
        "cargo",
        "test",
        "-p",
        "researchcode-runtime",
        "native_agent_loop_v2_visible_only_transition_after_tools_is_telemetry_only",
    ],
    [
        "cargo",
        "test",
        "-p",
        "researchcode-runtime",
        "native_agent_loop_v2_incomplete_streamed_tool_call_becomes_model_readable_error",
    ],
    ["cargo", "test", "-p", "researchcode-runtime", "compatible_provider"],
    ["cargo", "test", "-p", "researchcode-runtime", "live_model_request"],
    ["cargo", "test", "-p", "researchcode-runtime", "stream_observer"],
    ["cargo", "check", "--manifest-path", "desktop/src-tauri/Cargo.toml"],
]


def main() -> int:
    for command in COMMANDS:
        print(f"\n==> {' '.join(command)}", flush=True)
        completed = subprocess.run(command, cwd=ROOT)
        if completed.returncode != 0:
            print(
                f"Phase 1 regression pack failed: {' '.join(command)}",
                file=sys.stderr,
            )
            return completed.returncode
    print("\nPhase 1 agent-loop regression pack passed.", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
