#!/usr/bin/env python3
"""Native profile promotion gate for DeepSeek/Qwen.

DeepSeek/Qwen native profile changes are only promotable when parser, stream,
and scaffold/context-budget eval suites pass. Compatible providers never enter
this gate.
"""

from __future__ import annotations

import json
import subprocess
import sys


GATE_COMMANDS = [
    ("parser", [sys.executable, "scripts/run_parser_eval.py"]),
    ("stream", [sys.executable, "scripts/run_stream_eval.py"]),
    ("scaffold", [sys.executable, "scripts/run_scaffold_eval.py"]),
    ("scaffold_comparison", [sys.executable, "scripts/run_scaffold_comparison_eval.py"]),
]


def main() -> int:
    results = []
    for gate_name, command in GATE_COMMANDS:
        completed = subprocess.run(command, check=False, capture_output=True, text=True)
        results.append(
            {
                "gate": gate_name,
                "passed": completed.returncode == 0,
                "stdout": completed.stdout.strip(),
                "stderr": completed.stderr.strip()[:500],
            }
        )
    ok = all(result["passed"] for result in results)
    print(
        json.dumps(
            {
                "gate": "native_profile_promotion_v0",
                "native_profiles": ["deepseek-v4-native", "qwen3-6-27b-native"],
                "compatible_providers_included": False,
                "promotable": ok,
                "results": results,
            },
            sort_keys=True,
        )
    )
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
