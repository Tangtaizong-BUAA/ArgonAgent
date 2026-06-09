#!/usr/bin/env python3
"""Small Phase 0 CSV profiler prototype using Python standard library."""

from __future__ import annotations

import csv
import json
import re
import statistics
import sys
from collections import Counter
from pathlib import Path


SENSITIVE_NAME_PATTERNS = [
    re.compile(pattern, re.IGNORECASE)
    for pattern in [r"email", r"phone", r"ssn", r"subject", r"patient", r"name", r"token", r"key"]
]
EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")


def infer_sensitive(column: str, values: list[str]) -> tuple[str, list[str]]:
    evidence: list[str] = []
    if any(pattern.search(column) for pattern in SENSITIVE_NAME_PATTERNS):
        evidence.append("sensitive_column_name")
    non_empty = [value for value in values if value]
    if non_empty and sum(bool(EMAIL_RE.match(value)) for value in non_empty) >= max(1, len(non_empty) // 2):
        evidence.append("email_like_values")
    if evidence:
        return "sensitive_personal", evidence
    return "internal", []


def detect_outliers(values: list[str]) -> dict:
    numbers: list[float] = []
    for value in values:
        if not value:
            continue
        try:
            numbers.append(float(value))
        except ValueError:
            return {"numeric": False, "outlier_count": 0}
    if len(numbers) < 3:
        return {"numeric": bool(numbers), "outlier_count": 0}
    median = statistics.median(numbers)
    deviations = [abs(number - median) for number in numbers]
    mad = statistics.median(deviations) or 1.0
    outliers = [number for number in numbers if abs(number - median) / mad > 20]
    return {"numeric": True, "outlier_count": len(outliers)}


def profile_csv(path: Path) -> dict:
    with path.open("r", encoding="utf-8-sig", newline="") as handle:
        reader = csv.DictReader(handle)
        rows = list(reader)
    columns = reader.fieldnames or []
    row_tuples = [tuple(row.get(column, "") for column in columns) for row in rows]
    duplicate_rows = sum(count - 1 for count in Counter(row_tuples).values() if count > 1)
    column_profiles = []
    for column in columns:
        values = [row.get(column, "") for row in rows]
        missing = sum(1 for value in values if value == "")
        privacy_class, evidence = infer_sensitive(column, values)
        outlier = detect_outliers(values)
        column_profiles.append(
            {
                "column_name": column,
                "missing_count": missing,
                "privacy_class": privacy_class,
                "sensitivity_evidence": evidence,
                "outlier": outlier,
            }
        )
    return {
        "artifact_kind": "data_profile",
        "source_path": str(path),
        "row_count": len(rows),
        "column_count": len(columns),
        "duplicate_rows": duplicate_rows,
        "columns": column_profiles,
    }


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prototype_csv_profiler.py <input.csv>", file=sys.stderr)
        return 2
    result = profile_csv(Path(sys.argv[1]))
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
