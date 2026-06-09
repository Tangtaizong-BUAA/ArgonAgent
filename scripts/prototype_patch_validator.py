#!/usr/bin/env python3
"""Prototype patch invariant validator.

This script does not apply patches. It validates expected pass/fail behavior
for Phase 0 fixtures.
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path


PROTECTED_PARTS = {".ssh", ".env", "id_rsa", "id_ed25519"}


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def is_protected(path: str) -> bool:
    parts = Path(path).parts
    return path.startswith("..") or any(part in PROTECTED_PARTS for part in parts)


def validate_case(root: Path, case: dict) -> str:
    rel_path = case["path"]
    if is_protected(rel_path):
        return "fail_protected"
    fixture_dir = root / case["fixture_dir"]
    target = fixture_dir / rel_path
    old = case["old_string"]
    base_hash = case.get("base_hash", "")
    if old == "":
        if target.exists():
            return "fail_create_exists"
        return "pass_create"
    if not target.exists():
        return "fail_missing"
    text = target.read_text(encoding="utf-8")
    actual_hash = sha256_text(text)
    if base_hash not in ("__compute__", actual_hash):
        return "fail_stale"
    matches = text.count(old)
    if matches == 0:
        return "fail_missing_old_string"
    if matches > 1:
        return "fail_ambiguous"
    return "pass"


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prototype_patch_validator.py <fixture-root>", file=sys.stderr)
        return 2
    root = Path(sys.argv[1])
    cases = json.loads((root / "patch_cases.json").read_text(encoding="utf-8"))
    failures: list[str] = []
    for case in cases:
        actual = validate_case(root, case)
        expected = case["expected"]
        if actual != expected:
            failures.append(f"{case['id']} expected {expected}, got {actual}")
    if failures:
        for failure in failures:
            print(failure, file=sys.stderr)
        return 1
    print(f"patch validator fixtures passed: {len(cases)} cases")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

