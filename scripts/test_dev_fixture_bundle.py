#!/usr/bin/env python3
"""Contract test for the no-network dev fixture bundle."""

from __future__ import annotations

import json
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class DevFixtureBundleTests(unittest.TestCase):
    def test_bundle_generates_valid_manifest_and_event_logs(self) -> None:
        completed = subprocess.run(
            ["python3", "scripts/run_dev_fixture_bundle.py"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        payload = json.loads(completed.stdout.strip().splitlines()[-1])
        manifest_path = Path(payload["manifest"])
        self.assertTrue(manifest_path.exists())
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        coding_events = Path(manifest["coding_events"])
        model_events = Path(manifest["model_events"])
        agent_loop_events = Path(manifest["agent_loop_events"])
        live_transport_agent_loop_events = Path(manifest["live_transport_agent_loop_events"])
        blocked_permission_events = Path(manifest["blocked_permission_events"])
        research_events = Path(manifest["research_events"])
        self.assertTrue(coding_events.exists())
        self.assertTrue(model_events.exists())
        self.assertTrue(agent_loop_events.exists())
        self.assertTrue(live_transport_agent_loop_events.exists())
        self.assertTrue(blocked_permission_events.exists())
        self.assertTrue(research_events.exists())
        self.assertIn("static_mock.html", manifest["static_gui"]["open"])
        self.assertIn("model-timeline", manifest["local_api"]["model_timeline"])
        self.assertIn("session-snapshot", manifest["local_api"]["session_snapshot"])
        self.assertIn("session-snapshot", manifest["local_api"]["blocked_permission_snapshot"])
        self.assertIn("research_summary", manifest["local_api"])
        self.assertIn("blocked_permission_summary", manifest["local_api"])
        self.assertIn("live_transport_agent_loop_events.jsonl", manifest["static_gui"]["open"])


if __name__ == "__main__":
    unittest.main()
