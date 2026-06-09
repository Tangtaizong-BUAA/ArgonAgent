#!/usr/bin/env python3
"""Tests for SQLite persistence adapter v0."""

from __future__ import annotations

import sqlite3
import sys
from pathlib import Path
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from sqlite_store import ResearchCodeStore


class SQLiteStoreTests(unittest.TestCase):
    def setUp(self) -> None:
        self.store = ResearchCodeStore()
        self.store.create_project("proj", ".", "Project")
        self.store.create_session("sess", "proj", "qwen3-6-27b-native")

    def tearDown(self) -> None:
        self.store.close()

    def test_event_append_is_monotonic_and_chained(self) -> None:
        first = self.store.append_event(
            project_id="proj",
            session_id="sess",
            task_id="task",
            event_type="session.created",
            actor="runtime",
            payload={},
        )
        second = self.store.append_event(
            project_id="proj",
            session_id="sess",
            task_id="task",
            event_type="session.state_changed",
            actor="runtime",
            payload={"to_state": "Planning"},
        )
        self.assertEqual(first.sequence, 1)
        self.assertEqual(second.sequence, 2)
        row = self.store.conn.execute("SELECT prev_hash FROM events WHERE event_id = ?", (second.event_id,)).fetchone()
        self.assertEqual(row[0], first.hash)

    def test_plan_approval_is_not_permission(self) -> None:
        event = self.store.append_event(
            project_id="proj",
            session_id="sess",
            task_id="task",
            event_type="plan.approval_requested",
            actor="runtime",
            payload={"plan_approval_id": "pa_1"},
        )
        self.store.create_plan_approval("pa_1", "sess", "plan_1", event.event_id, "hash")
        with self.assertRaises(sqlite3.IntegrityError):
            self.store.create_permission("perm_bad", "sess", "plan", event.event_id, "hash")

    def test_artifact_requires_source_event(self) -> None:
        with self.assertRaises(sqlite3.IntegrityError):
            self.store.insert_artifact(
                artifact_id="artifact_1",
                project_id="proj",
                session_id="sess",
                kind="report",
                sha256="hash",
                size_bytes=1,
                logical_name="report.md",
                source_event_id="missing_event",
            )

    def test_research_job_links_manifest_artifact(self) -> None:
        event = self.store.append_event(
            project_id="proj",
            session_id="sess",
            task_id="task",
            event_type="research.job_completed",
            actor="research_worker",
            payload={"research_job_id": "rj_1"},
        )
        for artifact_id, kind, privacy_class in [
            ("artifact_profile", "data_profile", "internal"),
            ("artifact_privacy", "privacy_report", "sensitive_personal"),
            ("artifact_manifest", "manifest", "internal"),
        ]:
            self.store.insert_artifact(
                artifact_id=artifact_id,
                project_id="proj",
                session_id="sess",
                kind=kind,
                sha256=f"hash_{artifact_id}",
                size_bytes=10,
                logical_name=f"{kind}.json",
                source_event_id=event.event_id,
                privacy_class=privacy_class,
            )
        self.store.create_research_job("rj_1", "proj", "sess", "completed", "artifact_manifest")
        row = self.store.conn.execute(
            """
            SELECT r.state, a.kind
            FROM research_jobs r
            JOIN artifacts a ON a.artifact_id = r.manifest_artifact_id
            WHERE r.research_job_id = 'rj_1'
            """
        ).fetchone()
        self.assertEqual(row, ("completed", "manifest"))

    def test_eval_result_persistence(self) -> None:
        event = self.store.append_event(
            project_id="proj",
            session_id="sess",
            task_id="task",
            event_type="eval.case_completed",
            actor="runtime",
            payload={"eval_case_id": "DS-PARSE-01"},
        )
        self.store.insert_eval_result(
            eval_result_id="eval_result_1",
            eval_run_id="eval_run_1",
            eval_case_id="DS-PARSE-01",
            fixture_hash="fixture_hash",
            model_profile_id="deepseek-v4-native",
            metric_name="parser_exact_match",
            metric_value="1",
            verdict="pass",
            event_id=event.event_id,
        )
        row = self.store.conn.execute(
            "SELECT verdict, metric_value FROM eval_results WHERE eval_result_id = 'eval_result_1'"
        ).fetchone()
        self.assertEqual(row, ("pass", "1"))

    def test_import_runtime_event_jsonl_indexes_tool_results(self) -> None:
        events = [
            {
                "event_id": "evt_0001",
                "schema_version": "v0",
                "project_id": "proj",
                "session_id": "sess",
                "task_id": "task",
                "sequence": 1,
                "event_type": "tool.call_requested",
                "actor": "agent",
                "created_at": "now",
                "payload": {"tool_call_id": "tool_1", "tool_id": "file.read"},
                "prev_hash": None,
                "hash": "h1",
            },
            {
                "event_id": "evt_0002",
                "schema_version": "v0",
                "project_id": "proj",
                "session_id": "sess",
                "task_id": "task",
                "sequence": 2,
                "event_type": "tool.call_completed",
                "actor": "tool",
                "created_at": "now",
                "payload": {"tool_call_id": "tool_1", "tool_id": "file.read", "ok": True},
                "prev_hash": "h1",
                "hash": "h2",
            },
            {
                "event_id": "evt_0003",
                "schema_version": "v0",
                "project_id": "proj",
                "session_id": "sess",
                "task_id": "task",
                "sequence": 3,
                "event_type": "tool.result_recorded",
                "actor": "runtime",
                "created_at": "now",
                "payload": {
                    "tool_call_id": "tool_1",
                    "tool_id": "file.read",
                    "artifact_id": "artifact_tool_1",
                    "content_hash": "fnv64_hash",
                    "preview": "preview",
                },
                "prev_hash": "h2",
                "hash": "h3",
            },
            {
                "event_id": "evt_0004",
                "schema_version": "v0",
                "project_id": "proj",
                "session_id": "sess",
                "task_id": "task",
                "sequence": 4,
                "event_type": "model.call_started",
                "actor": "runtime",
                "created_at": "now",
                "payload": {
                    "call_id": "call_1",
                    "provider": "deepseek",
                    "adapter_id": "deepseek-v4-native",
                    "actual_model_name": "deepseek-v4-flash",
                    "role": "planner",
                    "live": False,
                },
                "prev_hash": "h3",
                "hash": "h4",
            },
            {
                "event_id": "evt_0005",
                "schema_version": "v0",
                "project_id": "proj",
                "session_id": "sess",
                "task_id": "task",
                "sequence": 5,
                "event_type": "model.stream_completed",
                "actor": "runtime",
                "created_at": "now",
                "payload": {
                    "stream_id": "stream_1",
                    "provider": "deepseek",
                    "artifact_id": "artifact_model_1",
                    "content_hash": "fnv64_model_hash",
                    "prompt_tokens": 100,
                    "completion_tokens": 20,
                    "reasoning_tokens": 15,
                    "prompt_cache_hit_tokens": 80,
                    "prompt_cache_miss_tokens": 20,
                },
                "prev_hash": "h4",
                "hash": "h5",
            },
            {
                "event_id": "evt_0006",
                "schema_version": "v0",
                "project_id": "proj",
                "session_id": "sess",
                "task_id": "task",
                "sequence": 6,
                "event_type": "model.call_completed",
                "actor": "runtime",
                "created_at": "now",
                "payload": {
                    "call_id": "call_1",
                    "provider": "deepseek",
                    "ok": True,
                    "artifact_id": "artifact_model_1",
                    "content_hash": "fnv64_model_hash",
                },
                "prev_hash": "h5",
                "hash": "h6",
            },
        ]
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "events.jsonl"
            path.write_text("\n".join(__import__("json").dumps(event) for event in events), encoding="utf-8")
            imported = self.store.import_event_jsonl(path)
        self.assertEqual(len(imported), 6)
        tool_row = self.store.conn.execute(
            "SELECT status, result_event_id FROM tool_calls WHERE tool_call_id = 'tool_1'"
        ).fetchone()
        self.assertEqual(tool_row, ("completed", "evt_0002"))
        artifact_row = self.store.conn.execute(
            "SELECT kind, sha256, source_event_id FROM artifacts WHERE artifact_id = 'artifact_tool_1'"
        ).fetchone()
        self.assertEqual(artifact_row, ("tool_result", "fnv64_hash", "evt_0003"))
        model_artifact_row = self.store.conn.execute(
            "SELECT kind, sha256, source_event_id FROM artifacts WHERE artifact_id = 'artifact_model_1'"
        ).fetchone()
        self.assertEqual(model_artifact_row, ("model_transcript", "fnv64_model_hash", "evt_0005"))
        model_call_row = self.store.conn.execute(
            """
            SELECT provider_id, model_profile_id, started_event_id, completed_event_id,
                   token_input, token_output, token_reasoning
            FROM model_calls WHERE model_call_id = 'call_1'
            """
        ).fetchone()
        self.assertEqual(
            model_call_row,
            ("deepseek", "deepseek-v4-native", "evt_0004", "evt_0006", 100, 20, 15),
        )

        summary = self.store.export_gui_summary("sess")
        self.assertEqual(summary["event_count"], 6)
        self.assertEqual(summary["tool_counts"], {"completed": 1})
        self.assertEqual(summary["model_counts"], {"deepseek": 1})
        self.assertEqual(summary["artifact_counts"], {"model_transcript": 1, "tool_result": 1})
        self.assertEqual(summary["latest_event"]["event_type"], "model.call_completed")
        self.assertEqual(len(summary["recent_events"]), 6)


if __name__ == "__main__":
    unittest.main()
