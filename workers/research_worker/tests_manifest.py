#!/usr/bin/env python3
"""Standard-library Research Worker manifest checks."""

from __future__ import annotations

import sys
import tempfile
import json
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from research_worker.artifacts import (
    write_analysis_script,
    write_markdown_report,
    write_notebook,
)
from research_worker.__main__ import write_job_manifest
from research_worker.csv_profile import profile_csv, write_privacy_report, write_profile
from research_worker.manifest import ResearchJobManifest, ResearchWorkerLimits


class ResearchJobManifestTests(unittest.TestCase):
    def test_network_disabled_by_default(self) -> None:
        manifest = ResearchJobManifest(
            job_id="job_1",
            input_paths=("input.csv",),
            output_dir="out",
        )
        manifest.validate()

    def test_network_enabled_fails(self) -> None:
        manifest = ResearchJobManifest(
            job_id="job_1",
            input_paths=("input.csv",),
            output_dir="out",
            network_enabled=True,
        )
        with self.assertRaises(ValueError):
            manifest.validate()

    def test_package_install_enabled_fails(self) -> None:
        manifest = ResearchJobManifest(
            job_id="job_1",
            input_paths=("input.csv",),
            output_dir="out",
            resource_limits=ResearchWorkerLimits(package_install_enabled=True),
        )
        with self.assertRaises(ValueError):
            manifest.validate()

    def test_nonpositive_limits_fail(self) -> None:
        manifest = ResearchJobManifest(
            job_id="job_1",
            input_paths=("input.csv",),
            output_dir="out",
            resource_limits=ResearchWorkerLimits(max_input_bytes=0),
        )
        with self.assertRaises(ValueError):
            manifest.validate()

    def test_input_required(self) -> None:
        manifest = ResearchJobManifest(
            job_id="job_1",
            input_paths=(),
            output_dir="out",
        )
        with self.assertRaises(ValueError):
            manifest.validate()

    def test_csv_profile_and_manifest_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            input_path = root / "input.csv"
            input_path.write_text(
                "subject_email,value\nuser@example.com,1\nuser@example.com,1000\n,1\n",
                encoding="utf-8",
            )
            profile = profile_csv(input_path)
            self.assertEqual(profile["row_count"], 3)
            self.assertEqual(profile["column_count"], 2)
            self.assertEqual(profile["columns"][0]["privacy_class"], "sensitive_personal")
            profile_path = write_profile(input_path, root / "out")
            privacy_report_path = write_privacy_report(profile_path, root / "out")
            analysis_script_path = write_analysis_script(profile_path, root / "out")
            report_path = write_markdown_report(profile_path, privacy_report_path, root / "out")
            notebook_path = write_notebook(profile_path, report_path, root / "out")
            manifest_path = write_job_manifest(
                "job_1",
                input_path,
                [
                    ("data_profile", profile_path),
                    ("privacy_report", privacy_report_path),
                    ("analysis_script", analysis_script_path),
                    ("markdown_report", report_path),
                    ("notebook", notebook_path),
                ],
                root / "out",
            )
            privacy_report = json.loads(privacy_report_path.read_text(encoding="utf-8"))
            self.assertTrue(privacy_report["cloud_model_requires_approval"])
            manifest_text = manifest_path.read_text(encoding="utf-8")
            manifest_payload = json.loads(manifest_text)
            self.assertIn("research_job_manifest.v0", manifest_text)
            self.assertIn("data_profile", manifest_text)
            self.assertIn("privacy_report", manifest_text)
            self.assertIn("analysis_script", manifest_text)
            self.assertIn("markdown_report", manifest_text)
            self.assertIn("notebook", manifest_text)
            self.assertIn("resource_limits", manifest_text)
            self.assertIn("package_install_enabled", manifest_text)
            self.assertIn("max_memory_mb", manifest_text)
            self.assertIn("os_sandbox", manifest_text)
            self.assertEqual(manifest_payload["data_lineage"]["artifact_count"], 5)
            self.assertEqual(len(manifest_payload["data_lineage"]["edges"]), 5)
            for artifact in manifest_payload["artifacts"]:
                self.assertEqual(
                    artifact["source_input_hash"],
                    manifest_payload["inputs"][0]["content_hash"],
                )
            report_text = report_path.read_text(encoding="utf-8")
            self.assertIn("Data Profile Report", report_text)
            notebook = json.loads(notebook_path.read_text(encoding="utf-8"))
            self.assertEqual(notebook["nbformat"], 4)
            self.assertEqual(notebook["metadata"]["researchcode"]["artifact_kind"], "notebook")


if __name__ == "__main__":
    unittest.main()
