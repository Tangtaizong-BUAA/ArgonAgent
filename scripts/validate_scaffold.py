#!/usr/bin/env python3
"""Validate the minimal scaffold without installing dependencies."""

from __future__ import annotations

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_PATHS = [
    "AGENTS.md",
    "README.md",
    "Cargo.toml",
    "crates/kernel/Cargo.toml",
    "crates/kernel/src/lib.rs",
    "crates/runtime/Cargo.toml",
    "crates/cli/Cargo.toml",
    "desktop/package.json",
    "desktop/static_mock.html",
    "desktop/mock_data/events.jsonl",
    "desktop/mock_data/artifact_manifest.json",
    "desktop/mock_data/privacy_report.json",
    "desktop/src/App.tsx",
    "desktop/src/components/AppShell.tsx",
    "desktop/src/runtime/localRuntimeClient.ts",
    "desktop/src-tauri/Cargo.toml",
    "desktop/src-tauri/src/main.rs",
    "workers/research_worker/research_worker/manifest.py",
    "docs/storage/sqlite_schema_v0.sql",
    "docs/schemas/provider/compatible_provider_config.schema.json",
    "eval/fixtures/deepseek/parser_golden.json",
    "eval/fixtures/qwen/parser_golden.json",
]


def main() -> int:
    missing = [path for path in REQUIRED_PATHS if not (ROOT / path).exists()]
    if missing:
        for path in missing:
            print(f"missing: {path}", file=sys.stderr)
        return 1
    provider_schema = json.loads((ROOT / "docs/schemas/provider/compatible_provider_config.schema.json").read_text())
    levels = provider_schema["properties"]["optimization_level"]["enum"]
    if "native" in levels:
        print("compatible provider schema allows native", file=sys.stderr)
        return 1
    print("scaffold validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
