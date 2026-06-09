#!/usr/bin/env python3
"""Eval harness v0 with SQLite result persistence."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from run_parser_eval import compare, load_cases  # noqa: E402
from sqlite_store import ResearchCodeStore, stable_hash  # noqa: E402


def run_parser_suite(store: ResearchCodeStore, eval_run_id: str) -> int:
    store.create_project("eval_project", ".", "Eval Project")
    store.create_session("eval_session", "eval_project", "native-parser-eval")
    total = 0
    failures = 0
    for suite, model_profile in [("deepseek", "deepseek-v4-native"), ("qwen", "qwen3-6-27b-native")]:
        cases = load_cases(suite)
        errors = compare(suite, cases)
        failed_ids = {error.split(" ", 1)[0] for error in errors}
        for case in cases:
            total += 1
            passed = case["id"] not in failed_ids
            failures += 0 if passed else 1
            event = store.append_event(
                project_id="eval_project",
                session_id="eval_session",
                task_id=case["id"],
                event_type="eval.case_completed",
                actor="runtime",
                payload={"suite": suite, "case_id": case["id"], "passed": passed},
            )
            fixture_hash = stable_hash(json.dumps(case, ensure_ascii=False, sort_keys=True))
            store.insert_eval_result(
                eval_result_id=f"{eval_run_id}_{case['id']}",
                eval_run_id=eval_run_id,
                eval_case_id=case["id"],
                fixture_hash=fixture_hash,
                model_profile_id=model_profile,
                metric_name="parser_exact_match",
                metric_value="1" if passed else "0",
                verdict="pass" if passed else "fail",
                event_id=event.event_id,
            )
    print(json.dumps({"suite": "parser", "total": total, "failures": failures}, sort_keys=True))
    return 0 if failures == 0 else 1


def run_scaffold_suite(store: ResearchCodeStore, eval_run_id: str) -> int:
    store.create_project("eval_project", ".", "Eval Project")
    store.create_session("eval_session", "eval_project", "native-scaffold-eval")
    completed = subprocess.run(
        [sys.executable, "scripts/run_scaffold_eval.py"],
        check=False,
        capture_output=True,
        text=True,
    )
    cases = json.loads(
        (Path("eval") / "fixtures" / "scaffold" / "scaffold_cases.json").read_text(encoding="utf-8")
    )
    passed = completed.returncode == 0
    for case in cases:
        event = store.append_event(
            project_id="eval_project",
            session_id="eval_session",
            task_id=case["id"],
            event_type="eval.case_completed",
            actor="runtime",
            payload={"suite": "scaffold", "case_id": case["id"], "passed": passed},
        )
        fixture_hash = stable_hash(json.dumps(case, ensure_ascii=False, sort_keys=True))
        store.insert_eval_result(
            eval_result_id=f"{eval_run_id}_{case['id']}",
            eval_run_id=eval_run_id,
            eval_case_id=case["id"],
            fixture_hash=fixture_hash,
            model_profile_id=case["family"],
            metric_name="scaffold_budget_gate",
            metric_value="1" if passed else "0",
            verdict="pass" if passed else "fail",
            event_id=event.event_id,
        )
    print(
        json.dumps(
            {"suite": "scaffold", "total": len(cases), "failures": 0 if passed else len(cases)},
            sort_keys=True,
        )
    )
    if not passed:
        print(completed.stderr.strip() or completed.stdout.strip(), file=sys.stderr)
    return completed.returncode


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--suite", choices=["parser", "scaffold", "scaffold_comparison"], default="parser")
    parser.add_argument("--db", default=":memory:")
    parser.add_argument("--eval-run-id", default="eval_run_v0")
    args = parser.parse_args()
    store = ResearchCodeStore(args.db)
    try:
        if args.suite == "parser":
            return run_parser_suite(store, args.eval_run_id)
        if args.suite == "scaffold":
            return run_scaffold_suite(store, args.eval_run_id)
        if args.suite == "scaffold_comparison":
            completed = subprocess.run(
                [sys.executable, "scripts/run_scaffold_comparison_eval.py"],
                check=False,
                capture_output=True,
                text=True,
            )
            print(completed.stdout.strip())
            if completed.returncode != 0:
                print(completed.stderr.strip(), file=sys.stderr)
            return completed.returncode
    finally:
        store.close()
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
