#!/usr/bin/env python3
"""SQLite persistence adapter v0 using Python standard library.

This is the executable storage contract for Phase 1. It loads the same schema
the future Rust adapter will use, enforces append-only event sequence at the
adapter boundary, and keeps PlanApproval separate from PermissionRequest.
"""

from __future__ import annotations

import json
import sqlite3
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "docs/storage/sqlite_schema_v0.sql"


def stable_hash(text: str) -> str:
    value = 0xCBF29CE484222325
    for byte in text.encode("utf-8"):
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"fnv64_{value:016x}"


@dataclass(frozen=True)
class StoredEvent:
    event_id: str
    sequence: int
    hash: str


class ResearchCodeStore:
    def __init__(self, db_path: str | Path = ":memory:") -> None:
        self.conn = sqlite3.connect(str(db_path))
        self.conn.execute("PRAGMA foreign_keys = ON")
        self.conn.executescript(SCHEMA.read_text(encoding="utf-8"))

    def close(self) -> None:
        self.conn.close()

    def create_project(self, project_id: str, path: str, display_name: str) -> None:
        now = timestamp()
        self.conn.execute(
            """
            INSERT INTO projects(project_id, path, display_name, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(project_id) DO UPDATE SET path=excluded.path, display_name=excluded.display_name, updated_at=excluded.updated_at
            """,
            (project_id, path, display_name, now, now),
        )
        self.conn.commit()

    def create_session(self, session_id: str, project_id: str, model_profile_id: str, state: str = "Created") -> None:
        now = timestamp()
        self.conn.execute(
            """
            INSERT INTO sessions(session_id, project_id, model_profile_id, state, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            """,
            (session_id, project_id, model_profile_id, state, now, now),
        )
        self.conn.commit()

    def append_event(
        self,
        *,
        project_id: str,
        session_id: str | None,
        task_id: str | None,
        event_type: str,
        actor: str,
        payload: dict[str, Any],
    ) -> StoredEvent:
        last = self.conn.execute(
            """
            SELECT sequence, hash FROM events
            WHERE project_id = ? AND COALESCE(session_id, '') = COALESCE(?, '')
            ORDER BY sequence DESC LIMIT 1
            """,
            (project_id, session_id),
        ).fetchone()
        sequence = int(last[0]) + 1 if last else 1
        prev_hash = str(last[1]) if last else None
        payload_json = json.dumps(payload, ensure_ascii=False, sort_keys=True)
        event_id = f"evt_{session_id or project_id}_{sequence:04d}"
        event_hash = stable_hash(f"{project_id}:{session_id}:{sequence}:{event_type}:{payload_json}:{prev_hash}")
        self.conn.execute(
            """
            INSERT INTO events(
              event_id, schema_version, project_id, session_id, task_id, sequence,
              event_type, actor, created_at, payload_json, prev_hash, hash
            ) VALUES (?, 'v0', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                event_id,
                project_id,
                session_id,
                task_id,
                sequence,
                event_type,
                actor,
                timestamp(),
                payload_json,
                prev_hash,
                event_hash,
            ),
        )
        self.conn.commit()
        return StoredEvent(event_id=event_id, sequence=sequence, hash=event_hash)

    def import_event(self, event: dict[str, Any]) -> StoredEvent:
        last = self.conn.execute(
            """
            SELECT sequence, hash FROM events
            WHERE project_id = ? AND COALESCE(session_id, '') = COALESCE(?, '')
            ORDER BY sequence DESC LIMIT 1
            """,
            (event["project_id"], event.get("session_id")),
        ).fetchone()
        expected_sequence = int(last[0]) + 1 if last else 1
        expected_prev_hash = str(last[1]) if last else None
        if int(event["sequence"]) != expected_sequence:
            raise ValueError(f"non-monotonic event sequence: expected {expected_sequence}, got {event['sequence']}")
        if event.get("prev_hash") != expected_prev_hash:
            raise ValueError(f"prev_hash mismatch for {event['event_id']}")
        payload_json = json.dumps(event.get("payload", {}), ensure_ascii=False, sort_keys=True)
        self.conn.execute(
            """
            INSERT INTO events(
              event_id, schema_version, project_id, session_id, task_id, sequence,
              event_type, actor, created_at, payload_json, prev_hash, hash
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                event["event_id"],
                event.get("schema_version", "v0"),
                event["project_id"],
                event.get("session_id"),
                event.get("task_id"),
                int(event["sequence"]),
                event["event_type"],
                event["actor"],
                event.get("created_at", timestamp()),
                payload_json,
                event.get("prev_hash"),
                event["hash"],
            ),
        )
        self._index_imported_event(event)
        self.conn.commit()
        return StoredEvent(event_id=event["event_id"], sequence=int(event["sequence"]), hash=event["hash"])

    def import_event_jsonl(self, path: str | Path) -> list[StoredEvent]:
        imported: list[StoredEvent] = []
        for line in Path(path).read_text(encoding="utf-8").splitlines():
            if line.strip():
                imported.append(self.import_event(json.loads(line)))
        return imported

    def _index_imported_event(self, event: dict[str, Any]) -> None:
        payload = event.get("payload", {})
        event_type = event.get("event_type")
        if event_type == "tool.call_requested":
            self.conn.execute(
                """
                INSERT INTO tool_calls(tool_call_id, session_id, tool_id, request_event_id, status)
                VALUES (?, ?, ?, ?, 'requested')
                """,
                (
                    payload["tool_call_id"],
                    event["session_id"],
                    payload["tool_id"],
                    event["event_id"],
                ),
            )
        elif event_type == "tool.call_completed":
            self.conn.execute(
                """
                UPDATE tool_calls
                SET result_event_id = ?, status = ?
                WHERE tool_call_id = ?
                """,
                (
                    event["event_id"],
                    "completed" if payload.get("ok") else "failed",
                    payload["tool_call_id"],
                ),
            )
        elif event_type == "tool.result_recorded":
            artifact_id = payload["artifact_id"]
            self.conn.execute(
                """
                INSERT OR IGNORE INTO artifacts(
                  artifact_id, project_id, session_id, kind, sha256, size_bytes,
                  logical_name, source_event_id, privacy_class, retention_policy
                ) VALUES (?, ?, ?, 'tool_result', ?, 0, ?, ?, 'internal', 'project')
                """,
                (
                    artifact_id,
                    event["project_id"],
                    event.get("session_id"),
                    payload["content_hash"],
                    artifact_id,
                    event["event_id"],
                ),
            )
        elif event_type == "model.stream_completed":
            artifact_id = payload["artifact_id"]
            self.conn.execute(
                """
                INSERT OR IGNORE INTO artifacts(
                  artifact_id, project_id, session_id, kind, sha256, size_bytes,
                  logical_name, source_event_id, privacy_class, retention_policy
                ) VALUES (?, ?, ?, 'model_transcript', ?, 0, ?, ?, 'internal', 'project')
                """,
                (
                    artifact_id,
                    event["project_id"],
                    event.get("session_id"),
                    payload["content_hash"],
                    artifact_id,
                    event["event_id"],
                ),
            )
            self.conn.execute(
                """
                UPDATE model_calls
                SET token_input = COALESCE(token_input, ?),
                    token_output = COALESCE(token_output, ?),
                    token_reasoning = COALESCE(token_reasoning, ?)
                WHERE model_call_id = (
                  SELECT model_call_id FROM model_calls
                  WHERE session_id = ?
                    AND provider_id = ?
                    AND completed_event_id IS NULL
                  ORDER BY started_event_id DESC
                  LIMIT 1
                )
                """,
                (
                    int(payload.get("prompt_tokens", 0)),
                    int(payload.get("completion_tokens", 0)),
                    int(payload.get("reasoning_tokens", 0)),
                    event.get("session_id"),
                    payload.get("provider"),
                ),
            )
        elif event_type == "model.call_started":
            self.conn.execute(
                """
                INSERT OR IGNORE INTO model_calls(
                  model_call_id, session_id, model_profile_id, provider_id, started_event_id,
                  completed_event_id, prompt_template_version, parser_version,
                  token_input, token_output, token_reasoning
                ) VALUES (?, ?, ?, ?, ?, NULL, NULL, NULL, NULL, NULL, NULL)
                """,
                (
                    payload["call_id"],
                    event["session_id"],
                    payload.get("adapter_id", "unknown"),
                    payload.get("provider"),
                    event["event_id"],
                ),
            )
        elif event_type == "model.call_completed":
            artifact_id = payload.get("artifact_id")
            content_hash = payload.get("content_hash")
            self.conn.execute(
                """
                UPDATE model_calls
                SET completed_event_id = ?
                WHERE model_call_id = ?
                """,
                (event["event_id"], payload["call_id"]),
            )
            if artifact_id and content_hash:
                self.conn.execute(
                    """
                    INSERT OR IGNORE INTO artifacts(
                      artifact_id, project_id, session_id, kind, sha256, size_bytes,
                      logical_name, source_event_id, privacy_class, retention_policy
                    ) VALUES (?, ?, ?, 'model_transcript', ?, 0, ?, ?, 'internal', 'project')
                    """,
                    (
                        artifact_id,
                        event["project_id"],
                        event.get("session_id"),
                        content_hash,
                        artifact_id,
                        event["event_id"],
                    ),
                )

    def create_permission(
        self,
        permission_id: str,
        session_id: str,
        request_type: str,
        request_event_id: str,
        request_hash: str,
    ) -> None:
        self.conn.execute(
            """
            INSERT INTO permissions(permission_id, session_id, request_type, request_event_id, status, request_hash)
            VALUES (?, ?, ?, ?, 'requested', ?)
            """,
            (permission_id, session_id, request_type, request_event_id, request_hash),
        )
        self.conn.commit()

    def create_plan_approval(
        self,
        plan_approval_id: str,
        session_id: str,
        plan_id: str,
        request_event_id: str,
        request_hash: str,
    ) -> None:
        self.conn.execute(
            """
            INSERT INTO plan_approvals(plan_approval_id, session_id, plan_id, request_event_id, status, request_hash)
            VALUES (?, ?, ?, ?, 'requested', ?)
            """,
            (plan_approval_id, session_id, plan_id, request_event_id, request_hash),
        )
        self.conn.commit()

    def insert_artifact(
        self,
        *,
        artifact_id: str,
        project_id: str,
        session_id: str | None,
        kind: str,
        sha256: str,
        size_bytes: int,
        logical_name: str,
        source_event_id: str,
        privacy_class: str = "internal",
        retention_policy: str = "project",
    ) -> None:
        self.conn.execute(
            """
            INSERT INTO artifacts(
              artifact_id, project_id, session_id, kind, sha256, size_bytes,
              logical_name, source_event_id, privacy_class, retention_policy
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                artifact_id,
                project_id,
                session_id,
                kind,
                sha256,
                size_bytes,
                logical_name,
                source_event_id,
                privacy_class,
                retention_policy,
            ),
        )
        self.conn.commit()

    def create_research_job(
        self,
        research_job_id: str,
        project_id: str,
        session_id: str | None,
        state: str,
        manifest_artifact_id: str | None,
    ) -> None:
        now = timestamp()
        self.conn.execute(
            """
            INSERT INTO research_jobs(
              research_job_id, project_id, session_id, state, manifest_artifact_id, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            """,
            (research_job_id, project_id, session_id, state, manifest_artifact_id, now, now),
        )
        self.conn.commit()

    def insert_eval_result(
        self,
        *,
        eval_result_id: str,
        eval_run_id: str,
        eval_case_id: str,
        fixture_hash: str,
        model_profile_id: str,
        metric_name: str,
        metric_value: str,
        verdict: str,
        event_id: str | None = None,
    ) -> None:
        self.conn.execute(
            """
            INSERT INTO eval_results(
              eval_result_id, eval_run_id, eval_case_id, fixture_hash, model_profile_id,
              metric_name, metric_value, verdict, event_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                eval_result_id,
                eval_run_id,
                eval_case_id,
                fixture_hash,
                model_profile_id,
                metric_name,
                metric_value,
                verdict,
                event_id,
            ),
        )
        self.conn.commit()

    def session_summary(self, session_id: str) -> dict[str, Any]:
        session = self.conn.execute(
            "SELECT session_id, project_id, model_profile_id, state FROM sessions WHERE session_id = ?",
            (session_id,),
        ).fetchone()
        if not session:
            raise KeyError(f"unknown session {session_id}")
        event_count = self.conn.execute(
            "SELECT COUNT(*) FROM events WHERE session_id = ?", (session_id,)
        ).fetchone()[0]
        tool_counts = dict(
            self.conn.execute(
                "SELECT status, COUNT(*) FROM tool_calls WHERE session_id = ? GROUP BY status",
                (session_id,),
            ).fetchall()
        )
        artifact_counts = dict(
            self.conn.execute(
                "SELECT kind, COUNT(*) FROM artifacts WHERE session_id = ? GROUP BY kind",
                (session_id,),
            ).fetchall()
        )
        model_counts = dict(
            self.conn.execute(
                """
                SELECT COALESCE(provider_id, 'unknown'), COUNT(*)
                FROM model_calls
                WHERE session_id = ?
                GROUP BY COALESCE(provider_id, 'unknown')
                """,
                (session_id,),
            ).fetchall()
        )
        latest_event = self.conn.execute(
            """
            SELECT event_type, sequence FROM events
            WHERE session_id = ?
            ORDER BY sequence DESC LIMIT 1
            """,
            (session_id,),
        ).fetchone()
        return {
            "session_id": session[0],
            "project_id": session[1],
            "model_profile_id": session[2],
            "state": session[3],
            "event_count": event_count,
            "tool_counts": tool_counts,
            "model_counts": model_counts,
            "artifact_counts": artifact_counts,
            "latest_event": {
                "event_type": latest_event[0],
                "sequence": latest_event[1],
            }
            if latest_event
            else None,
        }

    def export_gui_summary(self, session_id: str) -> dict[str, Any]:
        summary = self.session_summary(session_id)
        recent_events = [
            {
                "event_id": row[0],
                "sequence": row[1],
                "event_type": row[2],
                "actor": row[3],
                "payload": json.loads(row[4]),
            }
            for row in self.conn.execute(
                """
                SELECT event_id, sequence, event_type, actor, payload_json
                FROM events
                WHERE session_id = ?
                ORDER BY sequence DESC LIMIT 20
                """,
                (session_id,),
            ).fetchall()
        ]
        summary["recent_events"] = list(reversed(recent_events))
        return summary


def timestamp() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def main() -> int:
    store = ResearchCodeStore()
    store.create_project("proj", ".", "Project")
    store.create_session("sess", "proj", "qwen3-6-27b-native")
    event = store.append_event(
        project_id="proj",
        session_id="sess",
        task_id="task",
        event_type="session.created",
        actor="runtime",
        payload={},
    )
    print(json.dumps({"ok": True, "event_id": event.event_id, "sequence": event.sequence}, sort_keys=True))
    store.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
