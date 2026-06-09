#!/usr/bin/env python3
"""Persist a runtime fixture event log into SQLite and validate indexed rows."""

from __future__ import annotations

import argparse
import json
import sqlite3
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from sqlite_store import ResearchCodeStore  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("events_jsonl")
    parser.add_argument("--db", default=":memory:")
    args = parser.parse_args()

    path = Path(args.events_jsonl)
    first_event = next(
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    )
    project_id = first_event["project_id"]
    session_id = first_event["session_id"]
    store = ResearchCodeStore(args.db)
    try:
        store.create_project(project_id, ".", "Imported Fixture")
        store.create_session(session_id, project_id, "qwen3-6-27b-native")
        imported = store.import_event_jsonl(path)
        counts = dict(
            store.conn.execute(
                """
                SELECT 'events', COUNT(*) FROM events
                UNION ALL SELECT 'tool_calls', COUNT(*) FROM tool_calls
                UNION ALL SELECT 'tool_result_artifacts', COUNT(*) FROM artifacts WHERE kind = 'tool_result'
                UNION ALL SELECT 'model_transcript_artifacts', COUNT(*) FROM artifacts WHERE kind = 'model_transcript'
                """
            ).fetchall()
        )
        if counts["events"] != len(imported):
            raise AssertionError("event count mismatch after import")
        if counts["tool_calls"] < 1 and counts["model_transcript_artifacts"] < 1:
            raise AssertionError("expected indexed tool calls or model transcript artifacts")
        if counts["tool_calls"] >= 1 and counts["tool_result_artifacts"] < 1:
            raise AssertionError("expected indexed tool result artifacts")
        summary = store.export_gui_summary(session_id)
        print(json.dumps({"ok": True, **counts, "summary": summary}, sort_keys=True))
    except (sqlite3.Error, AssertionError, ValueError) as error:
        print(f"sqlite fixture persistence failed: {error}", file=sys.stderr)
        return 1
    finally:
        store.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
