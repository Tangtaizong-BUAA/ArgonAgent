import assert from "node:assert/strict";
import {
  buildCommandCenterViewModel,
  buildModelStreamTimeline,
  buildWorkflowPanels,
  buildUrl,
  createPendingPackageFromSession,
  getApprovalQueue,
  getArtifacts,
  getEvents,
  getHealth,
  getLatestRun,
  getModelTimeline,
  getRepoMap,
  getSessionSnapshot,
  getSummary,
  getToolCatalog,
  parseJsonl,
  parseSseEvents,
  previewTool,
} from "./local_api_client.mjs";

function fakeFetch(routes) {
  return async (url) => {
    const parsed = new URL(url);
    const key = `${parsed.pathname}?${parsed.searchParams.toString()}`;
    const payload = routes[key] ?? routes[parsed.pathname];
    return {
      ok: payload !== undefined,
      status: payload === undefined ? 404 : 200,
      async json() {
        return payload;
      },
    };
  };
}

assert.equal(
  buildUrl("http://127.0.0.1:8765", "/events", { path: "apps/desktop/mock_data/events.jsonl" }),
  "http://127.0.0.1:8765/events?path=apps%2Fdesktop%2Fmock_data%2Fevents.jsonl",
);

assert.deepEqual(parseJsonl('{"sequence":1}\n{"sequence":2}\n'), [
  { sequence: 1 },
  { sequence: 2 },
]);

assert.deepEqual(
  parseSseEvents('id: evt_1\nevent: session.created\ndata: {"sequence":1}\n\n'),
  [{ id: "evt_1", event: "session.created", data: { sequence: 1 } }],
);

const fetchImpl = fakeFetch({
  "/health": { ok: true },
  "/latest-run": {
    latest_run: {
      agent_loop_events: "runs/dev_fixture_bundle_latest/agent_loop_events.jsonl",
    },
  },
  "/events?path=apps%2Fdesktop%2Fmock_data%2Fevents.jsonl": {
    events: [
      { event_type: "session.created", sequence: 1 },
      { event_type: "tool.call_completed", sequence: 2, payload: { ok: true } },
    ],
  },
  "/summary?path=apps%2Fdesktop%2Fmock_data%2Fevents.jsonl": {
    summary: {
      event_count: 2,
      latest_event: { event_type: "tool.call_completed", sequence: 2 },
      tool_counts: { completed: 1 },
      model_counts: { deepseek: 1 },
      artifact_counts: { tool_result: 1 },
      recent_events: [{ event_type: "tool.call_completed", sequence: 2 }],
    },
  },
  "/artifacts?dir=apps%2Fdesktop%2Fmock_data": {
    artifacts: [{ path: "artifact_manifest.json" }],
  },
  "/model-timeline?path=apps%2Fdesktop%2Fmock_data%2Fevents.jsonl": {
    model_timeline: {
      call_count: 1,
      stream_count: 1,
      calls: [
        {
          call_id: "call_1",
          provider: "deepseek",
          status: "completed",
          scaffold: {
            level: "DeepSeekFull",
            protected_reserve_tokens: 144000,
          },
        },
      ],
      streams: [{ stream_id: "stream_1", provider: "deepseek", status: "completed" }],
    },
  },
  "/session-snapshot?path=apps%2Fdesktop%2Fmock_data%2Fevents.jsonl": {
    session_snapshot: {
      event_count: 2,
      last_event_type: "permission.requested",
      state: "WaitingForToolApproval",
      health: "blocked_for_permission",
      pending_permissions: [{ permission_id: "perm_1", request_type: "file_write" }],
      pending_plan_approvals: [],
      counts: { "permission.requested": 1 },
      can_resume_without_user: false,
    },
  },
  "/repo-map?root=.&max_files=160&max_depth=4": {
    repo_map: {
      file_count: 2,
      omitted_count: 1,
      tech_stack: ["rust", "typescript/javascript"],
      important_files: ["Cargo.toml"],
      tree: ["Cargo.toml", "src/lib.rs"],
    },
  },
  "/tool-catalog": {
    tool_catalog: {
      version: "kernel-core-tool-spec-v0",
      tool_count: 2,
      permission_required_count: 1,
      concurrency_safe_count: 1,
      categories: { file: 1, shell: 1 },
      tools: [
        { tool_id: "file.read", risk: "read_only", permission_required: false },
        { tool_id: "shell.command", risk: "executes_command", permission_required: true },
      ],
    },
  },
  "/approval-queue": {
    approval_queue: {
      pending_count: 1,
      pending: [{ permission_id: "perm_1", request_type: "file_write", status: "requested" }],
      permissions: [{ permission_id: "perm_1", request_type: "file_write", status: "requested" }],
      patches: [{ patch_id: "patch_1", status: "validated" }],
      tools: [{ tool_call_id: "tool_1", tool_id: "patch.apply", status: "requested" }],
      latest_pending: { permission_id: "perm_1", request_type: "file_write", status: "requested" },
    },
  },
  "/tool/preview": {
    ok: true,
    tool_id: "repo.map",
    result: {
      file_count: 2,
      omitted_count: 1,
      tech_stack: ["rust"],
      important_files: ["Cargo.toml"],
      tree: ["Cargo.toml"],
    },
  },
  "/native-loop/pending-package-from-session": {
    ok: true,
    package_dir: "runs/test_native_pending_from_session",
    session_snapshot: { health: "blocked_for_permission" },
    approval_queue: { pending_count: 1 },
    pending_tool: { permission_id: "perm_1" },
  },
});

const modelEvents = [
  {
    event_type: "model.call_started",
    sequence: 1,
    payload: {
      call_id: "call_1",
      provider: "deepseek",
      adapter_id: "deepseek-v4-native",
      actual_model_name: "deepseek-v4-flash",
      role: "planner",
      live: false,
      scaffold_level: "DeepSeekFull",
      prompt_tokens_estimate: 100,
      prompt_hash: "fnv64_prompt",
      tool_catalog_hash: "fnv64_tools",
      max_context_tokens: 1000000,
      prompt_scaffold_budget: 53000,
      dynamic_context_budget: 700000,
      protected_reserve_tokens: 144000,
      budget_warning_count: 0,
    },
  },
  {
    event_type: "model.stream_delta",
    sequence: 2,
    payload: {
      stream_id: "stream_1",
      provider: "deepseek",
      delta_kind: "reasoning_sanitized",
      preview: "Need [REDACTED_SECRET] from [REDACTED_PATH]",
    },
  },
  {
    event_type: "model.stream_delta",
    sequence: 3,
    payload: {
      stream_id: "stream_1",
      provider: "deepseek",
      delta_kind: "content",
      preview: "Visible answer",
    },
  },
  {
    event_type: "model.stream_completed",
    sequence: 4,
    payload: {
      stream_id: "stream_1",
      provider: "deepseek",
      artifact_id: "transcript_1",
      content_hash: "fnv64_hash",
      prompt_tokens: 100,
      completion_tokens: 20,
      reasoning_tokens: 15,
      prompt_cache_hit_tokens: 80,
      prompt_cache_miss_tokens: 20,
    },
  },
  {
    event_type: "model.call_completed",
    sequence: 5,
    payload: {
      call_id: "call_1",
      provider: "deepseek",
      ok: true,
      artifact_id: "transcript_1",
      content_hash: "fnv64_hash",
    },
  },
];

const modelTimeline = buildModelStreamTimeline(modelEvents);
assert.equal(modelTimeline.callCount, 1);
assert.equal(modelTimeline.streamCount, 1);
assert.equal(modelTimeline.calls[0].status, "completed");
assert.equal(modelTimeline.calls[0].scaffold.level, "DeepSeekFull");
assert.equal(modelTimeline.calls[0].scaffold.protectedReserveTokens, 144000);
assert.equal(modelTimeline.streams[0].tokens.reasoning, 15);
assert.equal(modelTimeline.streams[0].tokens.promptCacheHit, 80);
assert.equal(modelTimeline.streams[0].deltas[0].preview.includes("sk-testsecret"), false);
assert.equal(modelTimeline.streams[0].deltas[0].preview.includes(".env"), false);

const workflowEvents = [
  {
    event_type: "tool.call.assembled",
    sequence: 0,
    payload: {
      tool_call_id: "tool_1",
      tool_id: "patch.apply",
      arguments_replayable: false,
      arguments_summary: { path: "src/parser.ts", redacted_keys: ["old_string", "new_string"] },
    },
  },
  {
    event_type: "tool.permission.evaluated",
    sequence: 0.5,
    payload: { tool_call_id: "tool_1", tool_id: "patch.apply", decision: "prechecked", mode: "runtime_facade" },
  },
  {
    event_type: "tool.call_requested",
    sequence: 1,
    payload: { tool_call_id: "tool_1", tool_id: "patch.apply" },
  },
  {
    event_type: "patch.proposal_created",
    sequence: 2,
    payload: { patch_id: "patch_1", path: "src/parser.ts" },
  },
  {
    event_type: "patch.proposal_validated",
    sequence: 3,
    payload: { patch_id: "patch_1", validation: "pass" },
  },
  {
    event_type: "permission.requested",
    sequence: 4,
    payload: { permission_id: "perm_1", request_type: "file_write" },
  },
  {
    event_type: "permission.decided",
    sequence: 5,
    payload: { permission_id: "perm_1", request_type: "file_write", decision: "allow_once" },
  },
  {
    event_type: "tool.dispatched",
    sequence: 5.5,
    payload: { tool_call_id: "tool_1", tool_id: "patch.apply", dispatcher: "tool_executor" },
  },
  {
    event_type: "patch.applied",
    sequence: 6,
    payload: { patch_id: "patch_1", path: "src/parser.ts" },
  },
  {
    event_type: "tool.call_completed",
    sequence: 7,
    payload: { tool_call_id: "tool_1", tool_id: "patch.apply", ok: true },
  },
  {
    event_type: "tool.result_recorded",
    sequence: 8,
    payload: {
      tool_call_id: "tool_1",
      tool_id: "patch.apply",
      artifact_id: "artifact_1",
      content_hash: "fnv64_hash",
      preview: "patch result",
    },
  },
  {
    event_type: "tool.completed",
    sequence: 8.5,
    payload: { tool_call_id: "tool_1", tool_id: "patch.apply", ok: true },
  },
  {
    event_type: "tool.call_completed",
    sequence: 9,
    payload: { tool_call_id: "tool_2", tool_id: "shell.command", ok: true },
  },
];

const workflowPanels = buildWorkflowPanels(workflowEvents);
assert.equal(workflowPanels.tools[0].status, "completed");
assert.deepEqual(
  workflowPanels.tools[0].lifecycle.map((phase) => phase.phase),
  ["assembled", "permission_evaluated", "requested", "dispatched", "completed", "result_recorded", "completed"],
);
assert.equal(workflowPanels.tools[0].argumentsReplayable, false);
assert.equal(workflowPanels.tools[0].argumentsSummary.path, "src/parser.ts");
assert.equal(workflowPanels.permissions[0].status, "allow_once");
assert.equal(workflowPanels.patches[0].status, "applied");
assert.equal(workflowPanels.commands[0].toolCallId, "tool_2");

const blockedWorkflowPanels = buildWorkflowPanels(workflowEvents.slice(0, 6));
assert.equal(blockedWorkflowPanels.permissions[0].status, "requested");
assert.equal(blockedWorkflowPanels.patches[0].status, "validated");
assert.equal(blockedWorkflowPanels.tools[0].status, "requested");

assert.deepEqual(await getHealth(fetchImpl, "http://127.0.0.1:8765"), { ok: true });
assert.equal(
  (await getLatestRun(fetchImpl, "http://127.0.0.1:8765")).agent_loop_events,
  "runs/dev_fixture_bundle_latest/agent_loop_events.jsonl",
);
assert.equal(
  (await getEvents(fetchImpl, "http://127.0.0.1:8765", "apps/desktop/mock_data/events.jsonl"))[0]
    .event_type,
  "session.created",
);
const summary = await getSummary(
  fetchImpl,
  "http://127.0.0.1:8765",
  "apps/desktop/mock_data/events.jsonl",
);
assert.equal(summary.latest_event.event_type, "tool.call_completed");
assert.deepEqual(
  buildCommandCenterViewModel(
    [
      { event_type: "session.created", sequence: 1 },
      { event_type: "tool.call_completed", sequence: 2, payload: { ok: true } },
    ],
    summary,
  ),
  {
    eventCount: 2,
    latestEventType: "tool.call_completed",
    latestSequence: 2,
    toolCounts: { completed: 1 },
    modelCounts: { deepseek: 1 },
    artifactCounts: { tool_result: 1 },
    modelTimeline: {
      callCount: 0,
      streamCount: 0,
      calls: [],
      streams: [],
    },
    workflowPanels: {
      tools: [
        {
          toolCallId: undefined,
          toolId: undefined,
          status: "completed",
          completedSequence: 2,
          lifecycle: [
            {
              phase: "completed",
              eventType: "tool.call_completed",
              sequence: 2,
              decision: undefined,
              mode: undefined,
              dispatcher: undefined,
              ok: undefined,
            },
          ],
        },
      ],
      permissions: [],
      patches: [],
      commands: [],
    },
    repoMap: null,
    toolCatalog: null,
    recentEvents: [{ event_type: "tool.call_completed", sequence: 2 }],
  },
);

const modelViewModel = buildCommandCenterViewModel(modelEvents);
assert.equal(modelViewModel.modelTimeline.callCount, 1);
assert.equal(modelViewModel.modelTimeline.streams[0].status, "completed");
assert.deepEqual(modelViewModel.modelCounts, { deepseek: 1 });
assert.equal(
  (await getModelTimeline(
    fetchImpl,
    "http://127.0.0.1:8765",
    "apps/desktop/mock_data/events.jsonl",
  )).call_count,
  1,
);
const sessionSnapshot = await getSessionSnapshot(
  fetchImpl,
  "http://127.0.0.1:8765",
  "apps/desktop/mock_data/events.jsonl",
);
assert.equal(sessionSnapshot.health, "blocked_for_permission");
assert.equal(sessionSnapshot.pending_permissions[0].permission_id, "perm_1");
assert.equal(
  (await getArtifacts(fetchImpl, "http://127.0.0.1:8765", "apps/desktop/mock_data"))[0].path,
  "artifact_manifest.json",
);
assert.deepEqual(await getRepoMap(fetchImpl, "http://127.0.0.1:8765"), {
  file_count: 2,
  omitted_count: 1,
  tech_stack: ["rust", "typescript/javascript"],
  important_files: ["Cargo.toml"],
  tree: ["Cargo.toml", "src/lib.rs"],
});
const toolCatalog = await getToolCatalog(fetchImpl, "http://127.0.0.1:8765");
assert.equal(toolCatalog.tool_count, 2);
assert.equal(toolCatalog.permission_required_count, 1);
assert.equal(toolCatalog.tools[1].tool_id, "shell.command");
const approvalQueue = await getApprovalQueue(fetchImpl, "http://127.0.0.1:8765");
assert.equal(approvalQueue.pending_count, 1);
assert.equal(approvalQueue.latest_pending.permission_id, "perm_1");
assert.equal((await previewTool(fetchImpl, "http://127.0.0.1:8765", "repo.map")).file_count, 2);
assert.equal(
  (await createPendingPackageFromSession(fetchImpl, "http://127.0.0.1:8765", {
    event_path: "runs/session/events.jsonl",
  })).pending_tool.permission_id,
  "perm_1",
);

console.log("desktop local api client tests passed");
