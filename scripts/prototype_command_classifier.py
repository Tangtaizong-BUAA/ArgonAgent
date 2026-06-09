#!/usr/bin/env python3
"""Conservative Phase 0 command permission classifier prototype.

It classifies fixture commands only. It does not execute commands.
"""

from __future__ import annotations

import json
import shlex
import sys
from pathlib import Path


DENY_SUBSTRINGS = [
    "rm -rf",
    "curl ",
    "wget ",
    ".env",
    "~/.ssh",
    "id_rsa",
    "id_ed25519",
    "git push",
    "--force",
    "sudo ",
    "chmod 777",
]

PACKAGE_INSTALL_PREFIXES = [
    ["npm", "install"],
    ["pnpm", "install"],
    ["yarn", "add"],
    ["pip", "install"],
    ["pip3", "install"],
    ["python", "-m", "pip", "install"],
    ["python3", "-m", "pip", "install"],
    ["cargo", "install"],
]

ALLOW_PREFIXES = [
    ["rg"],
    ["find"],
    ["ls"],
    ["wc"],
    ["python3", "scripts/prototype_patch_validator.py"],
    ["python3", "scripts/validate_event_sequence.py"],
    ["python3", "scripts/validate_kernel_schemas.py"],
    ["npm", "test"],
    ["cargo", "test"],
    ["pytest"],
]


def starts_with(tokens: list[str], prefix: list[str]) -> bool:
    return tokens[: len(prefix)] == prefix


def classify(command: str) -> str:
    lowered = command.lower()
    if any(part in lowered for part in DENY_SUBSTRINGS):
        return "deny"
    if any(meta in command for meta in [";", "&&", "||", "$(", "`", "|", ">", "<"]):
        return "deny"
    try:
        tokens = shlex.split(command)
    except ValueError:
        return "deny"
    if any(starts_with(tokens, prefix) for prefix in PACKAGE_INSTALL_PREFIXES):
        return "ask_package_install"
    if any(starts_with(tokens, prefix) for prefix in ALLOW_PREFIXES):
        return "allow"
    return "ask"


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prototype_command_classifier.py <permission_cases.json>", file=sys.stderr)
        return 2
    cases = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    failures: list[str] = []
    for case in cases:
        actual = classify(case["command"])
        expected = case["expected"]
        if actual != expected:
            failures.append(f"{case['id']} expected {expected}, got {actual}")
    if failures:
        for failure in failures:
            print(failure, file=sys.stderr)
        return 1
    print(f"command permission fixtures passed: {len(cases)} cases")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
