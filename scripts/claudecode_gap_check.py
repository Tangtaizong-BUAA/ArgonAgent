#!/usr/bin/env python3
"""Check ClaudeCode/OpenCode parity coverage at the current ResearchCode layer.

This is not a claim of full parity. It is a regression guard that keeps the
known gap table executable so future work cannot accidentally erase a production
kernel capability or silently mark gated features as implemented.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class GapCheck:
    area: str
    expected_status: str
    paths: tuple[str, ...]
    markers: tuple[str, ...]
    notes: str


CHECKS = [
    GapCheck(
        "RuntimeFacade boundary",
        "implemented",
        (
            "crates/runtime/src/runtime_facade.rs",
            "crates/runtime/src/runtime_facade_impl.rs",
        ),
        ("start_session", "submit_user_message", "stream_agent_events", "resume_session_from_eventlog"),
        "TUI/GUI must share this boundary.",
    ),
    GapCheck(
        "Agent loop v2/v3 continuation",
        "implemented",
        (
            "crates/runtime/src/native_agent_loop.rs",
            "crates/runtime/src/native_agent_loop_tools.rs",
        ),
        (
            "run_native_agent_loop_v2_deepseek",
            "build_deepseek_anthropic_multi_tool_result_request_with_thinking",
            "agent.loop_recovery",
            "agent.tool.streaming_completed",
        ),
        "DeepSeek tool_result continuation cannot stop after tool execution; streaming tool-use inputs must enter the same ledger.",
    ),
    GapCheck(
        "ToolSpec v2",
        "implemented",
        "crates/kernel/src/tool.rs",
        ("provider_aliases", "input_schema_json", "renderer", "tool_catalog_hash", "ToolCapabilityStatus"),
        "Tool catalog must be stable, GUI-renderable, and explicit about production/governance/preview/gated capability status.",
    ),
    GapCheck(
        "File edit/write/multi-edit",
        "implemented",
        "crates/runtime/src/tool_execution.rs",
        ("execute_file_edit", "execute_file_write", "execute_file_multi_edit", "write_rollback_artifact"),
        "Read-before-write and rollback are required before broad agent edits.",
    ),
    GapCheck(
        "Shell classifier audit",
        "implemented",
        (
            "crates/runtime/src/agent_kernel/permission_gate.rs",
            "crates/runtime/src/command.rs",
        ),
        ("classify_command_with_reasons", "CommandClassification", "PACKAGE_INSTALL_PREFIXES"),
        "Permission cards need explainable command risk.",
    ),
    GapCheck(
        "Permission policy persistence",
        "implemented",
        "crates/runtime/src/permission_policy.rs",
        ("PermissionPolicyStore", "PermissionRuleScope", "AllowProjectRule", "from_tsv"),
        "Allow-session/project decisions must be auditable and resumable.",
    ),
    GapCheck(
        "TUI slash/cards",
        "implemented",
        "crates/cli/src/main.rs",
        ("render_slash_command_palette", "render_agent_event_cards", "ToolCallCard", "TodoPanel"),
        "TUI remains a client over runtime events.",
    ),
    GapCheck(
        "Session/event replay",
        "implemented",
        "crates/runtime/src/event_log.rs",
        ("append", "read_jsonl", "NonMonotonicSequence"),
        "GUI must be able to replay append-only events.",
    ),
    GapCheck(
        "Context and compaction",
        "implemented",
        (
            "crates/runtime/src/native_turn_controller.rs",
            "crates/runtime/src/native_agent_loop.rs",
            "crates/runtime/src/context_budget.rs",
        ),
        (
            "evaluate_native_context_guard",
            "context.compaction.started",
            "context.compaction.completed",
            "context.compaction.blocked",
            "DEEPSEEK_TARGET_MODEL_CALL_TOKENS",
        ),
        "DeepSeek native loop preflights every prepared request and blocks over-budget calls before provider I/O.",
    ),
    GapCheck(
        "HookDispatch integration",
        "implemented",
        "crates/runtime/src/hook_dispatcher.rs",
        ("dispatch", "PreToolUse", "HookDecision", "HookDispatcher", "register", "is_empty"),
        "PreToolUse/PostToolUse hooks must dispatch through the V2 tool execution pipeline. Events defined in kernel hooks.rs.",
    ),
    GapCheck(
        "DeepSeek cache planner",
        "implemented",
        ("crates/runtime/src/native_profile/deepseek/cache_prefix.rs",),
        ("plan_cache_breakpoints", "apply_cache_control_blocks", "apply_cache_breakpoints_to_model_messages"),
        "Anthropic-format DeepSeek requests must include cache breakpoints at system/metadata/conversation boundaries.",
    ),
    GapCheck(
        "Tool orchestration concurrent execution",
        "implemented",
        (
            "crates/runtime/src/tool_orchestration.rs",
            "crates/runtime/src/tool_execution.rs",
        ),
        ("partition_tool_calls", "execute_tool_batch_concurrent", "SiblingAbortController", "MAX_TOOL_CONCURRENCY"),
        "Concurrent-safe tools must be partitionable into parallel-execution batches with sibling abort.",
    ),
    GapCheck(
        "Research coworker artifacts",
        "partial",
        "workers/research_worker/research_worker/manifest.py",
        ("ResearchJobManifest", "resource_limits", "privacy_class"),
        "CSV artifacts exist; notebook/report lifecycle needs deeper GUI flow.",
    ),
    GapCheck(
        "MCP capability surface",
        "gated",
        "crates/kernel/src/tool.rs",
        ("mcp.tool", "mcp.resource", "ToolCapabilityStatus::Gated"),
        "MCP is cataloged but disabled by default until runtime execution support is implemented.",
    ),
    GapCheck(
        "Worktree capability surface",
        "gated",
        "crates/kernel/src/tool.rs",
        ("worktree.create", "worktree.rollback", "ToolCapabilityStatus::Gated"),
        "Worktree tools are cataloged but disabled by default until runtime guardrails are wired.",
    ),
    GapCheck(
        "Subagent capability surface",
        "gated",
        "crates/kernel/src/tool.rs",
        (
            "agent.explorer",
            "agent.reviewer",
            "agent.worker",
            "ToolCapabilityStatus::Gated",
        ),
        "Subagents are cataloged but disabled by default until isolated runtime policy checks are complete.",
    ),
]


def check_one(check: GapCheck) -> tuple[bool, str]:
    paths = (check.paths,) if isinstance(check.paths, str) else check.paths
    existing_paths = [ROOT / path for path in paths if (ROOT / path).exists()]
    if not existing_paths:
        return False, "missing file(s): " + ", ".join(paths)
    text = "\n".join(path.read_text(encoding="utf-8") for path in existing_paths)
    missing = [marker for marker in check.markers if marker not in text]
    if missing:
        return False, "missing markers: " + ", ".join(missing)
    return True, "ok"


def main() -> int:
    failures: list[str] = []
    print("| area | expected_status | check | notes |")
    print("| --- | --- | --- | --- |")
    for check in CHECKS:
        ok, detail = check_one(check)
        print(f"| {check.area} | {check.expected_status} | {detail} | {check.notes} |")
        if check.expected_status == "implemented" and not ok:
            failures.append(f"{check.area}: {detail}")
    if failures:
        print("\nRelease-blocking gap check failures:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
