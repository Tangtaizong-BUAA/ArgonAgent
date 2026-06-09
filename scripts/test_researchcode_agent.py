#!/usr/bin/env python3
"""Tests for the standard-library local agent runner."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from researchcode_agent import deepseek_messages_url, endpoint_config  # noqa: E402


class ResearchCodeAgentRunnerTests(unittest.TestCase):
    def test_deepseek_defaults_to_anthropic_v4_flash_without_raw_key_arg(self) -> None:
        class Args:
            base_url = None
            model = None

        old_key = os.environ.pop("DEEPSEEK_API_KEY", None)
        try:
            url, model, key = endpoint_config("deepseek", Args())
            self.assertEqual(url, "https://api.deepseek.com/anthropic")
            self.assertEqual(model, "deepseek-v4-flash")
            self.assertIsNone(key)
        finally:
            if old_key is not None:
                os.environ["DEEPSEEK_API_KEY"] = old_key
        self.assertEqual(
            deepseek_messages_url("https://api.deepseek.com/anthropic"),
            "https://api.deepseek.com/anthropic/v1/messages",
        )

    def test_mock_provider_finishes_and_writes_run_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cwd = Path(tmp)
            response = cwd / "response.json"
            response.write_text(
                json.dumps(
                    {
                        "tool_calls": [
                            {
                                "name": "finish",
                                "arguments": {"message": "mock done"},
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "scripts/researchcode_agent.py"),
                    "run",
                    "--provider",
                    "mock",
                    "--task",
                    "finish",
                    "--cwd",
                    str(cwd),
                    "--mock-response-file",
                    str(response),
                ],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            summary = json.loads(completed.stdout)
            self.assertEqual(summary["final"], "mock done")
            self.assertTrue(Path(summary["events"]).exists())
            self.assertTrue((Path(summary["run_dir"]) / "summary.json").exists())
            events = Path(summary["events"]).read_text(encoding="utf-8")
            self.assertIn("model.call_started", events)
            self.assertIn("model.call_completed", events)
            self.assertIn("model_response_", events)

    def test_mock_provider_sensitive_file_read_is_denied(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cwd = Path(tmp)
            response = cwd / "response.json"
            response.write_text(
                json.dumps(
                    {
                        "tool_calls": [
                            {
                                "name": "file.read",
                                "arguments": {"path": ".env"},
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )
            completed = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "scripts/researchcode_agent.py"),
                    "run",
                    "--provider",
                    "mock",
                    "--task",
                    "read env",
                    "--cwd",
                    str(cwd),
                    "--mock-response-file",
                    str(response),
                    "--max-turns",
                    "1",
                ],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            summary = json.loads(completed.stdout)
            events = Path(summary["events"]).read_text(encoding="utf-8")
            self.assertIn("sensitive path denied", events)


if __name__ == "__main__":
    unittest.main()
