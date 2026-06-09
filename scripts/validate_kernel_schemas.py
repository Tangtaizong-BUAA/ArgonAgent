#!/usr/bin/env python3
"""Lightweight Phase 0 schema/example checks without external dependencies."""

from __future__ import annotations

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCHEMA_DIRS = [
    ROOT / "docs" / "schemas" / "kernel",
    ROOT / "docs" / "schemas" / "provider",
    ROOT / "docs" / "schemas" / "task_contract",
]
EXAMPLE_DIR = ROOT / "docs" / "schemas" / "examples"


def load_json(path: Path) -> object:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def check_schema(path: Path) -> list[str]:
    errors: list[str] = []
    data = load_json(path)
    if not isinstance(data, dict):
        return [f"{path}: schema root is not object"]
    for key in ["$schema", "title", "type", "properties"]:
        if key not in data:
            errors.append(f"{path}: missing {key}")
    required = data.get("required", [])
    if "schema_version" not in required:
        errors.append(f"{path}: schema_version must be required")
    if path.name == "permission_request.schema.json":
        enum = data["properties"]["request_type"]["enum"]
        if "plan" in enum:
            errors.append(f"{path}: PermissionRequest must not include request_type=plan")
    if path.name == "compatible_provider_config.schema.json":
        enum = data["properties"]["optimization_level"]["enum"]
        if "native" in enum:
            errors.append(f"{path}: compatible provider must not allow native optimization")
    return errors


def check_example(path: Path) -> list[str]:
    errors: list[str] = []
    data = load_json(path)
    if not isinstance(data, dict):
        return [f"{path}: example root is not object"]
    if data.get("schema_version") != "v0":
        errors.append(f"{path}: expected schema_version v0")
    if path.name.startswith("permission_request") and data.get("request_type") == "plan":
        errors.append(f"{path}: plan approval must not be represented as PermissionRequest")
    if path.name.startswith("compatible_provider") and data.get("optimization_level") == "native":
        errors.append(f"{path}: compatible provider example cannot be native")
    return errors


def main() -> int:
    errors: list[str] = []
    for schema_dir in SCHEMA_DIRS:
        if not schema_dir.exists():
            errors.append(f"missing schema dir: {schema_dir}")
            continue
        for path in sorted(schema_dir.glob("*.schema.json")):
            errors.extend(check_schema(path))
    for path in sorted(EXAMPLE_DIR.glob("*.json")):
        errors.extend(check_example(path))
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("schema checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

