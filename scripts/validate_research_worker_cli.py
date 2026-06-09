#!/usr/bin/env python3
"""Validate the Research Worker CLI without external dependencies."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path


def main() -> int:
    fixture = Path("eval/fixtures/research/csv-quality-small/input.csv").resolve()
    with tempfile.TemporaryDirectory() as tmp:
        output_dir = Path(tmp) / "out"
        completed = subprocess.run(
            [
                sys.executable,
                "-m",
                "research_worker",
                "profile-csv",
                "job_fixture",
                str(fixture),
                str(output_dir),
            ],
            cwd=Path("workers/research_worker"),
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            print(completed.stderr, file=sys.stderr)
            return completed.returncode
        result = json.loads(completed.stdout)
        profile_path = Path(result["profile"])
        privacy_report_path = Path(result["privacy_report"])
        analysis_script_path = Path(result["analysis_script"])
        report_path = Path(result["report"])
        notebook_path = Path(result["notebook"])
        manifest_path = Path(result["manifest"])
        profile = json.loads(profile_path.read_text(encoding="utf-8"))
        privacy_report = json.loads(privacy_report_path.read_text(encoding="utf-8"))
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        assert profile["artifact_kind"] == "data_profile"
        assert profile["row_count"] > 0
        assert privacy_report["artifact_kind"] == "privacy_report"
        assert "cloud_model_requires_approval" in privacy_report
        assert analysis_script_path.exists()
        assert "csv.DictReader" in analysis_script_path.read_text(encoding="utf-8")
        assert "Data Profile Report" in report_path.read_text(encoding="utf-8")
        notebook = json.loads(notebook_path.read_text(encoding="utf-8"))
        assert notebook["nbformat"] == 4
        assert manifest["schema_version"] == "research_job_manifest.v0"
        assert manifest["network_enabled"] is False
        assert any(artifact["artifact_kind"] == "privacy_report" for artifact in manifest["artifacts"])
        assert any(artifact["artifact_kind"] == "analysis_script" for artifact in manifest["artifacts"])
        assert any(artifact["artifact_kind"] == "markdown_report" for artifact in manifest["artifacts"])
        assert any(artifact["artifact_kind"] == "notebook" for artifact in manifest["artifacts"])
    print("research worker cli fixture passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
