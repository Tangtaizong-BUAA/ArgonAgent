#!/usr/bin/env python3
"""Validate the CSV profiler against the small research fixture."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "eval" / "fixtures" / "research" / "csv-quality-small"


def main() -> int:
    completed = subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "prototype_csv_profiler.py"), str(FIXTURE / "input.csv")],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        print(completed.stderr, file=sys.stderr)
        return completed.returncode
    actual = json.loads(completed.stdout)
    expected = json.loads((FIXTURE / "expected_issues.json").read_text(encoding="utf-8"))
    errors: list[str] = []
    if actual["row_count"] != expected["row_count"]:
        errors.append("row_count mismatch")
    if actual["column_count"] != expected["column_count"]:
        errors.append("column_count mismatch")
    if actual["duplicate_rows"] != expected["expected_duplicate_rows"]:
        errors.append("duplicate row mismatch")
    by_name = {column["column_name"]: column for column in actual["columns"]}
    missing_columns = sorted(name for name, column in by_name.items() if column["missing_count"] > 0)
    if missing_columns != sorted(expected["expected_missing_columns"]):
        errors.append(f"missing columns mismatch: {missing_columns}")
    sensitive_columns = sorted(name for name, column in by_name.items() if column["privacy_class"] == "sensitive_personal")
    if sensitive_columns != sorted(expected["expected_sensitive_columns"]):
        errors.append(f"sensitive columns mismatch: {sensitive_columns}")
    outlier_columns = sorted(name for name, column in by_name.items() if column["outlier"]["outlier_count"] > 0)
    if outlier_columns != sorted(expected["expected_outlier_columns"]):
        errors.append(f"outlier columns mismatch: {outlier_columns}")
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("csv profile fixture passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

