import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import ts from "typescript";

const tmpDir = await mkdtemp(path.join(tmpdir(), "desktop-runtime-event-replay-"));
const viewModelModulePath = await compileRuntimeModule("./src/runtime/runtimeEventViewModel.ts", "runtimeEventViewModel.mjs");
const streamSanitizerModulePath = await compileRuntimeModule("./src/runtime/streamSanitizer.ts", "streamSanitizer.mjs");
const transcriptDedupeModulePath = await compileRuntimeModule("./src/runtime/transcriptDedupe.ts", "transcriptDedupe.mjs");
await compileRuntimeModule("./src/runtime/progressLedger.ts", "progressLedger.mjs");
await compileRuntimeModule("./src/runtime/runStore.ts", "runStore.mjs");
const reducerModulePath = await compileRuntimeModule("./src/runtime/runtimeEventReducer.ts", "runtimeEventReducer.mjs", {
  "@/runtime/progressLedger": "./progressLedger.mjs",
  "@/runtime/runStore": "./runStore.mjs",
  "@/runtime/runtimeEventViewModel": "./runtimeEventViewModel.mjs",
});

try {
  const viewModel = await import(pathToFileURL(viewModelModulePath).href);
  const streamSanitizer = await import(pathToFileURL(streamSanitizerModulePath).href);
  const transcriptDedupe = await import(pathToFileURL(transcriptDedupeModulePath).href);
  const reducer = await import(pathToFileURL(reducerModulePath).href);
  const emptyContextPressure = {
    promptTokens: null,
    maxTokens: null,
    remainingTokens: null,
    status: "normal",
    label: "等待上下文预算",
  };
  const state = {
    runStatus: "idle",
    autonomyMode: "conservative",
    contextPressure: emptyContextPressure,
    recoveryStatus: null,
    tokenUsage: { uploaded: 0, downloaded: 0, reasoning: 0, cacheHit: 0, cacheMiss: 0 },
    modelEstimates: {
      downloadedByStreamId: new Map(),
      reasoningByStreamId: new Map(),
    },
    streamChunks: [],
    discardedStreamingMessages: 0,
    settledStreamingMessages: 0,
    errorStoppedStreamingMessages: 0,
    committedMessages: [],
    progressItems: [],
    messages: [],
    permissionErrors: {},
    recoverableToolFailures: [],
    pendingPermissions: [],
    pendingPlans: [],
    planPreviewCache: new Map(),
  };

  const events = [
    {
      event_type: "session.state_changed",
      payload: { to_state: "Executing" },
    },
    {
      event_type: "session.autonomy_mode_changed",
      payload: { autonomy_mode: "fast_auto" },
    },
    {
      event_type: "model.context_budget",
      payload: { prompt_tokens: 42000, max_context_tokens: 128000, budget_remaining: 86000 },
    },
    {
      event_type: "context.compaction.started",
      payload: { reason: "threshold" },
    },
    {
      event_type: "context.compaction.completed",
      payload: {
        token_estimate_before: 240000,
        token_estimate_after: 150000,
        reason: "deepseek_preflight",
        spine_json: {
          confirmed_facts: [{ reference: "ref://event/4" }],
          observations: [{ reference: "ref://event/5" }],
          resources: [{ reference: "ref://event/2" }, { reference: "ref://event/3" }],
          decisions: [{ reference: "ref://event/6" }],
        },
        summary: "latest evidence ref://event/4 and ref://event/5",
      },
    },
    {
      event_type: "agent.loop_budget_reached",
      payload: { reason: "max_iterations" },
    },
    {
      event_type: "agent.loop_stopped",
      payload: { category: "turn_budget", reason: "max_iterations" },
    },
    {
      event_type: "model.call_started",
      payload: { prompt_tokens_estimate: 100 },
    },
    {
      event_type: "model.stream_delta",
      payload: { stream_id: "stream_1", delta_kind: "content", preview: "chars=40", runtime_sanitized: true },
    },
    {
      event_type: "runtime.stream.narration",
      payload: {
        stream_id: "stream_1",
        reason: "tool_call_live_preamble_retracted",
        content: "工具间叙述会保留显示",
      },
    },
    {
      event_type: "runtime.stream.preamble_suppressed",
      payload: { stream_id: "stream_1", reason: "tool_call_live_preamble_retracted", chars: 40 },
    },
    {
      event_type: "model.stream_delta",
      payload: { stream_id: "stream_1", delta_kind: "reasoning_sanitized", preview: "chars=20" },
    },
    {
      event_type: "model.stream_completed",
      payload: {
        stream_id: "stream_1",
        completion_tokens: 20,
        reasoning_tokens: 8,
        prompt_cache_hit_tokens: 30,
        prompt_cache_miss_tokens: 4,
        stop_reason: "max_tokens",
      },
    },
    {
      event_type: "assistant.message",
      payload: { content: "Final answer" },
    },
    {
      event_type: "assistant.message",
      payload: {
        reason: "permission_resume_tool_completed",
        content: "已完成审批后的 shell.command 工具执行。stdout (last 80 lines): noisy output",
      },
    },
    {
      event_type: "deepseek.tool_call.partial",
      payload: { tool_id: "shell.command", argument_delta_bytes: 64 },
    },
    {
      event_type: "tool.call_requested",
      payload: { tool_id: "shell.command", tool_call_id: "tool_1" },
    },
    {
      event_type: "tool.call_completed",
      payload: { tool_id: "shell.command", tool_call_id: "tool_1", ok: false },
    },
    {
      event_type: "tool.result_recorded",
      payload: {
        tool_id: "shell.command",
        tool_call_id: "tool_1",
        preview: "tool error duplicate observation next_action=continue",
      },
    },
    {
      event_type: "permission.requested",
      payload: {
        permission_id: "perm_shell",
        request_type: "command",
        tool_id: "shell.command",
        args_preview: "echo replay-fixture",
        path_preview: "/tmp/replay",
        risk_level: "low",
      },
    },
    {
      event_type: "permission.context",
      payload: {
        permission_id: "perm_shell",
        request_type: "command",
        tool_id: "shell.command",
        args_preview: "echo replay-fixture",
        path_preview: "/tmp/replay",
        risk_level: "low",
      },
    },
    {
      event_type: "permission.decided",
      payload: { permission_id: "perm_shell", decision: "allow_once" },
    },
    {
      event_type: "tool.call_requested",
      payload: { tool_id: "shell.command", tool_call_id: "tool_resume" },
    },
    {
      event_type: "runtime.permission_resume.completed",
      payload: {
        permission_id: "perm_shell",
        tool_id: "shell.command",
        tool_call_id: "tool_resume",
        tool_executed: true,
        ok: true,
      },
    },
    {
      event_type: "plan.mode_entered",
      payload: {
        plan_approval_id: "plan_1",
        plan_preview: "## Plan\n\n| Step | Action |\n| --- | --- |\n| 1 | replay |",
      },
    },
    {
      event_type: "plan.approval_requested",
      payload: { plan_approval_id: "plan_1", goal: "Replay coverage" },
    },
    {
      event_type: "runtime.plan_approval.model_continued",
      payload: { plan_approval_id: "plan_1" },
    },
    {
      event_type: "plan.approval_decided",
      payload: { plan_approval_id: "plan_1", decision: "approve" },
    },
    {
      event_type: "plan.mode_exited",
      payload: { plan_approval_id: "plan_orphaned" },
    },
    {
      event_type: "tool.input_repaired",
      payload: { tool_id: "file.write", reason: "normalized_path" },
    },
    {
      event_type: "subagent.spawned",
      payload: { subagent_id: "subagent_review_001", agent_type: "reviewer" },
    },
    {
      event_type: "subagent.message_sent",
      payload: { subagent_id: "subagent_review_001", content_preview: "检查 VoiceNote 测试覆盖" },
    },
    {
      event_type: "subagent.model_turn_started",
      payload: { subagent_id: "subagent_review_001", agent_type: "reviewer" },
    },
    {
      event_type: "subagent.tool_completed",
      payload: { subagent_id: "subagent_review_001", tool_id: "search.ripgrep", ok: true },
    },
    {
      event_type: "subagent.summary_recorded",
      payload: {
        subagent_id: "subagent_review_001",
        summary: "发现 ConfigTests 与 service mock 覆盖最关键。",
        artifact_refs: ["ref://subagent/subagent_review_001/summary"],
      },
    },
    {
      event_type: "subagent.completed",
      payload: {
        subagent_id: "subagent_review_001",
        summary: "子 Agent 已完成覆盖面审查。",
        artifact_refs: [
          "ref://subagent/subagent_review_001/summary",
          "ref://subagent/subagent_review_001/evidence/search-ripgrep",
        ],
      },
    },
    {
      event_type: "subagent.tool_blocked",
      payload: {
        subagent_id: "subagent_review_failed_001",
        tool_id: "shell.command",
        reason_code: "permission_denied",
      },
    },
    {
      event_type: "subagent.failed",
      payload: {
        subagent_id: "subagent_review_failed_001",
        reason_code: "model_error",
      },
    },
    {
      event_type: "subagent.cancelled",
      payload: {
        subagent_id: "subagent_review_cancelled_001",
        reason_code: "parent_cancelled",
      },
    },
    {
      event_type: "model.call_blocked",
      payload: { gate: "network_not_enabled" },
    },
    {
      event_type: "runtime.error",
      payload: { permission_id: "perm_error", message: "missing_api_key" },
    },
    {
      event_type: "runtime.turn_cancel_requested",
      payload: {},
    },
  ];

  for (const event of events) {
    replayEvent(state, event, viewModel, reducer);
  }

  assert.equal(state.runStatus, "stopped");
  assert.equal(state.autonomyMode, "fast_auto");
  assert.equal(state.contextPressure.status, "normal");
  assert.equal(state.contextPressure.promptTokens, 42000);
  assert.equal(state.contextPressure.remainingTokens, 86000);
  assert.equal(state.contextPressure.label, "上下文已压缩 240k -> 150k");
  assert.deepEqual(state.recoveryStatus, {
    eventType: "agent.loop_stopped",
    label: "恢复/循环: loop_stopped · max_iterations",
    status: "failed",
  });
  assert.equal(
    viewModel.recoveryStatusFromRuntimeEvent("agent.loop_budget.normalized", {
      effective_max_tool_calls: 256,
    }).status,
    "done",
  );
  assert.deepEqual(state.tokenUsage, {
    uploaded: 100,
    downloaded: 28,
    reasoning: 8,
    cacheHit: 30,
    cacheMiss: 4,
  });
  assert.deepEqual(state.streamChunks, [{ preview: "chars=40", runtimeSanitized: true }]);
  assert.equal(state.discardedStreamingMessages, 0);
  assert.equal(state.settledStreamingMessages, 1);
  assert.equal(state.errorStoppedStreamingMessages, 1);
  assert.equal(state.committedMessages.at(-1), "Final answer");
  assert.equal(state.committedMessages.some((message) => message.includes("noisy output")), false);
  assert.equal(state.messages.some((message) => message.includes("工具间叙述会保留显示")), true);
  assert.equal(state.pendingPermissions.length, 0);
  assert.equal(state.pendingPlans.length, 0);
  assert.equal(state.progressItems.some((item) => item.kind === "tool" && item.toolId === "shell.command"), true);
  assert.equal(state.progressItems.some((item) => item.toolCallId === "tool_1" && item.status === "done"), true);
  assert.equal(
    state.progressItems.some(
      (item) =>
        item.eventType === "model.stream_completed" &&
        item.category === "recovery" &&
        item.label.includes("模型输出达到上限: max_tokens") &&
        item.detail.includes("stream stream_1"),
    ),
    true,
  );
  const providerFinishReasonReduction = reducer.reduceModelRuntimeEvent(
    state.tokenUsage,
    "model.stream_completed",
    {
      stream_id: "stream_provider_finish",
      finish_reason: "content_filter",
      completion_tokens: 3,
    },
    state.modelEstimates,
  );
  assert.equal(providerFinishReasonReduction.progressItem.label, "模型流结束: content_filter");
  assert.equal(providerFinishReasonReduction.progressItem.category, "model");
  const providerStopReasonReduction = reducer.reduceModelRuntimeEvent(
    state.tokenUsage,
    "model.stream_completed",
    {
      stream_id: "stream_provider_stop",
      provider_stop_reason: "output_token_limit",
    },
    state.modelEstimates,
  );
  assert.equal(providerStopReasonReduction.progressItem.label, "模型输出达到上限: output_token_limit");
  assert.equal(providerStopReasonReduction.progressItem.category, "recovery");
  const normalStopReduction = reducer.reduceModelRuntimeEvent(
    state.tokenUsage,
    "model.stream_completed",
    {
      stream_id: "stream_normal_stop",
      stop_reason: "end_turn",
    },
    state.modelEstimates,
  );
  assert.equal(normalStopReduction.progressItem, undefined);
  assert.equal(state.progressItems.some((item) => item.label.includes("计划已批准，模型继续执行")), true);
  assert.equal(
    state.progressItems.some(
      (item) => item.label.includes("工具输入已规范化: file.write") && item.category === "repair",
    ),
    true,
  );
  assert.equal(
    state.progressItems.some(
      (item) => item.eventType === "tool.input_repaired" && item.category === "repair",
    ),
    true,
  );
  assert.equal(
    state.progressItems.some(
      (item) =>
        item.eventType === "subagent.spawned" &&
        item.category === "subagent" &&
        item.status === "running" &&
        item.detail === "subagent_review_001",
    ),
    true,
  );
  assert.equal(
    state.progressItems.some(
      (item) =>
        item.eventType === "subagent.tool_completed" &&
        item.category === "subagent" &&
        item.status === "done" &&
        item.label.includes("search.ripgrep"),
    ),
    true,
  );
  assert.equal(
    state.progressItems.some(
      (item) =>
        item.eventType === "subagent.summary_recorded" &&
        item.category === "subagent" &&
        item.status === "done" &&
        item.detail === "subagent_review_001",
    ),
    true,
  );
  assert.equal(
    state.progressItems.some(
      (item) =>
        item.eventType === "subagent.completed" &&
        item.category === "subagent" &&
        item.status === "done" &&
        item.detail === "subagent_review_001",
    ),
    true,
  );
  assert.equal(
    state.progressItems.some(
      (item) =>
        item.eventType === "subagent.tool_blocked" &&
        item.category === "subagent" &&
        item.status === "failed" &&
        item.detail.includes("subagent_review_failed_001") &&
        item.detail.includes("permission_denied"),
    ),
    true,
  );
  assert.equal(
    state.progressItems.some(
      (item) =>
        item.eventType === "subagent.failed" &&
        item.category === "subagent" &&
        item.status === "failed" &&
        item.detail.includes("subagent_review_failed_001") &&
        item.detail.includes("model_error"),
    ),
    true,
  );
  assert.equal(
    state.progressItems.some(
      (item) =>
        item.eventType === "subagent.cancelled" &&
        item.category === "subagent" &&
        item.status === "failed" &&
        item.detail.includes("subagent_review_cancelled_001") &&
        item.detail.includes("parent_cancelled"),
    ),
    true,
  );
  assert.equal(
    state.progressItems.some(
      (item) =>
        item.eventType === "context.compaction.completed" &&
        item.category === "context" &&
        item.detail.includes("压缩 240k -> 150k") &&
        item.detail.includes("L1 1 facts/1 obs/2 resources/1 decisions") &&
        item.detail.includes("refs 2"),
    ),
    true,
  );
  assert.equal(
    state.progressItems.some(
      (item) => item.eventType === "agent.loop_stopped" && item.category === "recovery" && item.status === "failed",
    ),
    true,
  );
  assert.equal(state.messages.some((message) => message.includes("Runtime 已修复")), false);
  assert.equal(state.messages.some((message) => message.includes("Agent 已达到本轮迭代上限")), false);
  assert.equal(state.messages.some((message) => message.includes("本轮已停止: max_iterations")), true);
  assert.equal(state.messages.some((message) => message.includes("调用工具: shell.command")), true);
  assert.equal(state.messages.some((message) => message.includes("工具失败: shell.command")), true);
  assert.equal(state.messages.some((message) => message.includes("工具完成: shell.command")), true);
  assert.equal(state.progressItems.some((item) => item.toolCallId === "tool_resume" && item.status === "done"), true);
  assert.deepEqual(state.recoverableToolFailures, [{ toolCallId: "tool_1", toolId: "shell.command" }]);
  assert.equal(state.messages.some((message) => message.includes("需要权限审批: command (perm_shell)")), true);
  assert.equal(state.messages.some((message) => message.includes("权限已处理: perm_shell (allow_once)")), true);
  assert.equal(state.messages.some((message) => message.includes("计划等待审批: Replay coverage")), true);
  assert.equal(state.messages.some((message) => message.includes("计划审批已处理: plan_1")), true);
  assert.equal(
    state.messages.some((message) => message.includes("模型调用被拒绝（gate: network_not_enabled）")),
    true,
  );
  assert.equal(state.messages.some((message) => message.includes("已中断当前轮")), true);
  assert.match(state.permissionErrors.perm_error, /API Key/);

  const blockedState = {
    ...state,
    runStatus: "running",
    messages: [],
    permissionErrors: {},
    errorStoppedStreamingMessages: 0,
  };
  replayEvent(blockedState, {
    event_type: "model.call_blocked",
    payload: { gate: "http_status_400" },
  }, viewModel, reducer);
  assert.equal(blockedState.runStatus, "failed");
  assert.equal(blockedState.errorStoppedStreamingMessages, 1);
  assert.equal(blockedState.messages.some((message) => message.includes("http_status_400")), true);

  const loopStoppedBlocked = reducer.reduceSessionRuntimeEvent(
    [],
    "agent.loop_stopped",
    { status: "blocked", category: "turn_budget", reason: "max_iterations" },
    (prefix) => `${prefix}_blocked`,
  );
  assert.equal(loopStoppedBlocked.runStatus, "stopped");
  assert.equal(loopStoppedBlocked.progressItems.at(-1)?.status, "done");
  assert.equal(loopStoppedBlocked.message?.content.includes("max_iterations"), true);

  const loopStoppedFailed = reducer.reduceSessionRuntimeEvent(
    [],
    "agent.loop_stopped",
    { status: "failed", category: "provider_failure", reason: "initial_model_http_failure" },
    (prefix) => `${prefix}_failed`,
  );
  assert.equal(loopStoppedFailed.runStatus, "failed");
  assert.equal(loopStoppedFailed.progressItems.at(-1)?.status, "failed");

  assert.equal(viewModel.runtimeStateToRunStatus("WaitingForPlanApproval"), "waiting_approval");
  assert.equal(viewModel.runtimeStateToRunStatus("WaitingForToolApproval"), "waiting_approval");
  assert.equal(viewModel.runtimeStateToRunStatus("WaitingForUser"), "stopped");
  assert.equal(viewModel.runtimeSnapshotStateToRunStatus("Failed", "stopped"), "stopped");
  assert.equal(viewModel.runtimeSnapshotStateToRunStatus("Failed", "running"), "failed");
  assert.equal(viewModel.runtimeSnapshotStateToRunStatus("Executing", "stopped"), "running");
  assert.equal(viewModel.shouldApplyRuntimeSnapshotState(12, 12), true);
  assert.equal(viewModel.shouldApplyRuntimeSnapshotState(0, 0), true);
  assert.equal(viewModel.shouldApplyRuntimeSnapshotState(13, 12), false);
  assert.equal(viewModel.shouldApplyRuntimeSnapshotState(11, 12), false);
  assert.equal(viewModel.shouldApplyRuntimeSnapshotState(0, 12), false);
  let snapshotGuardedRunStatus = "running";
  if (viewModel.shouldApplyRuntimeSnapshotState(13, 12)) {
    snapshotGuardedRunStatus = viewModel.runtimeSnapshotStateToRunStatus(
      "Completed",
      snapshotGuardedRunStatus,
    );
  }
  assert.equal(snapshotGuardedRunStatus, "running");
  assert.equal(viewModel.normalizeAutonomyMode("unknown"), "conservative");
  assert.equal(viewModel.autonomyModeLabel("manual_review"), "手动审查");
  assert.equal(
    viewModel.runtimeObservabilityProgress("model.context_budget", {
      prompt_tokens: 42000,
      max_context_tokens: 128000,
      budget_remaining: 86000,
    }).label,
    "上下文预算 42k / 128k",
  );
  assert.equal(
    viewModel.runtimeObservabilityProgress("model.context_budget", {
      estimated_total_tokens: 18500,
      target_limit_tokens: 128000,
      hard_limit_tokens: 256000,
    }).label,
    "上下文预算 19k / 256k",
  );
  assert.deepEqual(
    viewModel.contextPressureFromRuntimeEvent(
      "model.context_budget",
      {
        estimated_total_tokens: 18500,
        target_limit_tokens: 128000,
        hard_limit_tokens: 256000,
      },
      emptyContextPressure,
    ),
    {
      promptTokens: 18500,
      maxTokens: 256000,
      remainingTokens: 237500,
      status: "normal",
      label: "上下文 19k / 256k",
    },
  );
  assert.equal(
    viewModel.runtimeObservabilityProgress("model.context_budget", {
      budget_remaining: 86000,
    }).label,
    "上下文预算已更新",
  );
  assert.equal(
    viewModel.runtimeObservabilityProgress("context.compaction.skipped", {
      reason: "below_threshold",
    }).label,
    "暂未压缩上下文",
  );
  assert.equal(
    viewModel.contextPressureFromRuntimeEvent("context.compaction.completed", {
      token_estimate_before: 240000,
      token_estimate_after: 150000,
      spine_json: JSON.stringify({ confirmed_facts: [{}], resources: [{}, {}] }),
    }, emptyContextPressure).label,
    "上下文已压缩 240k -> 150k",
  );
  assert.equal(
    viewModel.runtimeObservabilityProgress("deepseek.cache.zone_b.hit", {}).label,
    "zone b 缓存命中",
  );
  assert.equal(viewModel.estimateTokenDelta("chars=64"), 16);
  assert.equal(viewModel.addTokenUsage({ uploaded: 1, downloaded: 2, reasoning: 3, cacheHit: 4, cacheMiss: 5 }, {
    uploaded: 2,
    downloaded: 3,
  }).downloaded, 5);
  assert.equal(
    streamSanitizer.resolveFinalStreamingContent(
      "第一段完整流式内容。\n\n第二段仍然应该保留。",
      "最终补充。",
    ),
    "第一段完整流式内容。\n\n第二段仍然应该保留。\n\n最终补充。",
  );
  assert.equal(
    streamSanitizer.resolveFinalStreamingContent("完整内容", "完整内容"),
    "完整内容",
  );
  assert.equal(
    streamSanitizer.resolveFinalStreamingContent("第一句。nn第二句：nn1. 项目", "第一句。\n\n第二句：\n\n1. 项目"),
    "第一句。\n\n第二句：\n\n1. 项目",
  );
  assert.equal(
    transcriptDedupe.isDuplicateAgentText(
      "你好啊！我是 Agent。nn我可以帮你做很多事情，比如：nn1. 浏览代码",
      "你好啊！我是 Agent。\n\n我可以帮你做很多事情，比如：\n\n1. 浏览代码",
    ),
    true,
  );

  console.log("desktop runtime event replay tests passed");
} finally {
  await rm(tmpDir, { recursive: true, force: true });
}

async function compileRuntimeModule(sourcePath, outputName, replacements = {}) {
  const sourceUrl = new URL(sourcePath, import.meta.url);
  const source = await readFile(sourceUrl, "utf8");
  const compiled = ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.ES2022,
      target: ts.ScriptTarget.ES2022,
      importsNotUsedAsValues: ts.ImportsNotUsedAsValues.Remove,
      verbatimModuleSyntax: false,
    },
  });
  const modulePath = path.join(tmpDir, outputName);
  let outputText = compiled.outputText;
  for (const [from, to] of Object.entries(replacements)) {
    outputText = outputText.replaceAll(`"${from}"`, `"${to}"`);
  }
  await writeFile(modulePath, outputText, "utf8");
  return modulePath;
}

function replayEvent(state, event, viewModel, reducer) {
  const payload = event.payload ?? {};
  const eventType = event.event_type;
  const contextUpdate = viewModel.contextPressureFromRuntimeEvent(eventType, payload, state.contextPressure);
  if (contextUpdate) {
    state.contextPressure = contextUpdate;
  }
  const recoveryUpdate = viewModel.recoveryStatusFromRuntimeEvent(eventType, payload);
  if (recoveryUpdate) {
    state.recoveryStatus = recoveryUpdate;
  }
  const sessionReduction = reducer.reduceSessionRuntimeEvent(
    state.progressItems,
    eventType,
    payload,
    (prefix) => `${prefix}_${state.progressItems.length}`,
  );
  if (sessionReduction.handled) {
    state.progressItems = sessionReduction.progressItems;
    if (sessionReduction.runStatus) {
      state.runStatus = sessionReduction.runStatus;
    }
    if (sessionReduction.autonomyMode) {
      state.autonomyMode = sessionReduction.autonomyMode;
    }
    if (sessionReduction.message) {
      state.messages.push(sessionReduction.message.content);
    }
    return;
  }
  const modelReduction = reducer.reduceModelRuntimeEvent(
    state.tokenUsage,
    eventType,
    payload,
    state.modelEstimates,
  );
    if (modelReduction.handled) {
      state.tokenUsage = modelReduction.tokenUsage;
      state.modelEstimates = modelReduction.estimates;
      if (modelReduction.progressItem) {
        state.progressItems.push({
          id: `progress_${state.progressItems.length}`,
          ...modelReduction.progressItem,
        });
      }
      if (modelReduction.streamChunk) {
        state.streamChunks.push(modelReduction.streamChunk);
      }
    if (modelReduction.commitStreamingMessage !== undefined) {
      state.committedMessages.push(modelReduction.commitStreamingMessage);
    }
    if (modelReduction.scheduleStreamCommit) {
      state.committedMessages.push("");
    }
    return;
  }
  const toolReduction = reducer.reduceToolRuntimeEvent(
    state.progressItems,
    eventType,
    payload,
    (prefix) => `${prefix}_${state.progressItems.length}`,
  );
  if (toolReduction.handled) {
    state.progressItems = toolReduction.progressItems;
    if (toolReduction.message) {
      state.messages.push(toolReduction.message.content);
    }
    if (toolReduction.discardActiveStreamingMessage) {
      state.discardedStreamingMessages += 1;
    }
    if (toolReduction.settleActiveStreamingMessage) {
      state.settledStreamingMessages += 1;
    }
    if (toolReduction.recoverableToolFailure) {
      state.recoverableToolFailures.push(toolReduction.recoverableToolFailure);
    }
    return;
  }
  if (eventType === "permission.requested") {
    const reduction = reducer.reducePermissionRuntimeEvent(state.pendingPermissions, eventType, payload);
    state.pendingPermissions = reduction.pendingPermissions;
    const permissionId = String(payload.permission_id ?? "");
    assert.equal(state.pendingPermissions.find((item) => item.permission_id === permissionId)?.args_preview, "echo replay-fixture");
    if (reduction.message) {
      state.messages.push(reduction.message);
    }
    return;
  }
  if (eventType === "permission.context" || eventType === "permission.decided" || eventType === "permission.suggestion_submitted") {
    const reduction = reducer.reducePermissionRuntimeEvent(state.pendingPermissions, eventType, payload);
    state.pendingPermissions = reduction.pendingPermissions;
    if (eventType === "permission.context") {
      const permissionId = String(payload.permission_id ?? "");
      assert.equal(state.pendingPermissions.find((item) => item.permission_id === permissionId)?.args_preview, "echo replay-fixture");
    }
    if (reduction.message) {
      state.messages.push(reduction.message);
    }
    return;
  }
  if (
    eventType === "plan.mode_entered" ||
    eventType === "plan.approval_requested" ||
    eventType === "plan.approval_decided" ||
    eventType === "plan.mode_exited"
  ) {
    const reduction = reducer.reducePlanApprovalRuntimeEvent(
      state.pendingPlans,
      eventType,
      payload,
      state.planPreviewCache,
    );
    state.pendingPlans = reduction.pendingPlanApprovals;
    state.planPreviewCache = reduction.planPreviewCache;
    if (eventType === "plan.approval_requested") {
      const planApprovalId = String(payload.plan_approval_id ?? "");
      assert.match(state.pendingPlans.find((item) => item.plan_approval_id === planApprovalId)?.plan_preview ?? "", /\| Step \| Action \|/);
    }
    if (reduction.message) {
      state.messages.push(reduction.message);
    }
    return;
  }
  const observabilityReduction = reducer.reduceObservabilityRuntimeEvent(
    state.progressItems,
    eventType,
    payload,
    (prefix) => `${prefix}_${state.progressItems.length}`,
  );
  if (observabilityReduction.handled) {
    state.progressItems = observabilityReduction.progressItems;
    if (observabilityReduction.message) {
      state.messages.push(observabilityReduction.message.content);
    }
    if (observabilityReduction.clearDecisionForPermissionId) {
      state.pendingPermissions = state.pendingPermissions.filter(
        (item) => item.permission_id !== observabilityReduction.clearDecisionForPermissionId,
      );
    }
    return;
  }
  const errorReduction = reducer.reduceRuntimeErrorEvent(eventType, payload);
  if (errorReduction.handled) {
    if (errorReduction.runStatus) {
      state.runStatus = errorReduction.runStatus;
    }
    if (errorReduction.stopStreaming) {
      state.errorStoppedStreamingMessages += 1;
    }
    if (errorReduction.message) {
      state.messages.push(errorReduction.message.content);
    }
    if (errorReduction.permissionError) {
      state.permissionErrors[errorReduction.permissionError.permissionId] = errorReduction.permissionError.message;
    }
  }
}
