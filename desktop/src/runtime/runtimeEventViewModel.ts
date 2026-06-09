import type { ContextPressureState, RecoveryStatusState } from "@/components/RuntimeStatus";
import type { ProgressItem, RunStatus, TokenUsageStats } from "@/types";

export type RuntimeAutonomyMode = "fast_auto" | "manual_review" | "conservative";

export interface RuntimeObservabilityProgress {
  label: string;
  status: ProgressItem["status"];
  message?: string;
  toolCallId?: string;
  toolId?: string;
  permissionId?: string;
  kind?: ProgressItem["kind"];
  category?: ProgressItem["category"];
  detail?: string;
}

export function runtimeStateToRunStatus(state: string): RunStatus {
  if (state === "Executing") return "running";
  if (state === "WaitingForToolApproval" || state === "WaitingForPlanApproval") {
    return "waiting_approval";
  }
  if (state === "Completed") return "completed";
  if (state === "Failed") return "failed";
  if (state === "Cancelled" || state === "WaitingForUser") return "stopped";
  return "idle";
}

export function runtimeSnapshotStateToRunStatus(state: string, currentStatus?: RunStatus): RunStatus {
  if (state === "Failed" && currentStatus === "stopped") {
    return "stopped";
  }
  return runtimeStateToRunStatus(state);
}

export function shouldApplyRuntimeSnapshotState(
  snapshotEventCount: number,
  currentEventCursor: number,
): boolean {
  if (!Number.isFinite(snapshotEventCount) || !Number.isFinite(currentEventCursor)) {
    return false;
  }
  if (currentEventCursor <= 0) {
    return true;
  }
  return snapshotEventCount === currentEventCursor;
}

export function normalizeAutonomyMode(value: unknown): RuntimeAutonomyMode {
  const raw = String(value ?? "").trim();
  if (raw === "fast_auto" || raw === "manual_review" || raw === "conservative") {
    return raw;
  }
  return "conservative";
}

export function autonomyModeLabel(value: RuntimeAutonomyMode): string {
  if (value === "fast_auto") return "快速自动";
  if (value === "manual_review") return "手动审查";
  return "保守";
}

export function messageFromRuntimeError(eventType: string, payload: Record<string, unknown>): string {
  if (eventType === "model.call_blocked") {
    const gate = String(payload.gate ?? payload.error_code ?? "").trim();
    const suffix = gate && !String(payload.message ?? "").includes(gate) ? `（gate: ${gate}）` : "";
    return `模型调用被拒绝${suffix}: ${detectBlockedReason(payload)}`;
  }
  if (eventType === "tool.error.model_readable" && payload.retryable !== false) {
    const code = String(payload.error_code ?? "tool_observation");
    return `工具观察已反馈给模型: ${code}`;
  }
  const message = String(payload.message ?? payload.error_code ?? eventType);
  if (eventType === "runtime.error") {
    const lower = message.toLowerCase();
    if (lower.includes("network_not_enabled")) {
      return "Runtime 错误: 网络访问未开启（设置 RESEARCHCODE_ALLOW_NETWORK=1 后重启）";
    }
    if (lower.includes("missing_api_key")) {
      return "Runtime 错误: API Key 缺失（请设置 DEEPSEEK_API_KEY 或 QWEN_API_KEY）";
    }
    return `Runtime 错误: ${message}`;
  }
  return `工具错误: ${message}`;
}

export function contextPressureFromRuntimeEvent(
  eventType: string,
  payload: Record<string, unknown>,
  current: ContextPressureState,
): ContextPressureState | null {
  if (eventType === "model.context_budget") {
    const budget = contextBudgetNumbers(payload);
    return {
      promptTokens: budget.promptTokens,
      maxTokens: budget.maxTokens,
      remainingTokens: budget.remainingTokens,
      status: "normal",
      label: budget.promptTokens && budget.maxTokens
        ? `上下文 ${formatTokenCount(budget.promptTokens)} / ${formatTokenCount(budget.maxTokens)}`
        : "上下文预算已更新",
    };
  }
  if (!eventType.startsWith("context.compaction.")) {
    return null;
  }
  const suffix = eventType.replace("context.compaction.", "");
  const completedLabel = compactionContextPressureLabel(payload);
  return {
    ...current,
    status: suffix === "blocked" || suffix === "failed"
      ? "blocked"
      : suffix === "started"
        ? "compacting"
        : "normal",
    label: suffix === "completed"
      ? completedLabel
      : suffix === "started"
        ? "正在压缩上下文"
        : suffix === "skipped"
          ? current.promptTokens && current.maxTokens
            ? current.label
            : "上下文低于压缩阈值"
          : suffix === "blocked" || suffix === "failed"
            ? "上下文压缩受阻"
            : `上下文压缩 ${suffix}`,
  };
}

export function recoveryStatusFromRuntimeEvent(
  eventType: string,
  payload: Record<string, unknown>,
): RecoveryStatusState | null {
  if (!isRecoveryEventType(eventType)) {
    return null;
  }
  const observability = runtimeObservabilityProgress(eventType, payload);
  return {
    eventType,
    label: observability?.label ?? eventType.replace("agent.", ""),
    status: observability?.status ?? "running",
  };
}

export function runtimeObservabilityProgress(
  eventType: string,
  payload: Record<string, unknown>,
): RuntimeObservabilityProgress | null {
  if (eventType === "deepseek.error.classified") {
    const code = String(payload.error_code ?? payload.category ?? "unknown");
    return { label: `DeepSeek 错误分类: ${code}`, status: "failed" };
  }
  if (eventType === "model.call_recovery_planned") {
    const strategy = String(payload.recovery_strategy ?? payload.strategy ?? "retry");
    return { label: `模型恢复计划: ${strategy}`, status: "running" };
  }
  if (eventType === "deepseek.retry.scheduled") {
    const strategy = String(payload.recovery_strategy ?? "retry");
    return { label: `DeepSeek 重试: ${strategy}`, status: "running" };
  }
  if (eventType === "deepseek.protocol_fallback.started") {
    return { label: "DeepSeek 协议降级开始", status: "running" };
  }
  if (eventType === "deepseek.protocol_fallback.completed") {
    return { label: "DeepSeek 协议降级完成", status: "done" };
  }
  if (eventType === "deepseek.protocol_fallback.failed") {
    return { label: "DeepSeek 协议降级失败", status: "failed" };
  }
  if (eventType === "reasoning.budget.checked") {
    const action = String(payload.action ?? "checked");
    return { label: `reasoning 预算检查: ${action}`, status: "done" };
  }
  if (eventType === "reasoning.fold.started") {
    return { label: "reasoning 折叠开始", status: "running" };
  }
  if (eventType === "reasoning.fold.completed") {
    return { label: "reasoning 折叠完成", status: "done" };
  }
  if (eventType === "reasoning.replay.required") {
    return { label: "reasoning replay 需要补齐", status: "running" };
  }
  if (eventType === "reasoning.replay.injected") {
    return { label: "reasoning replay 已注入", status: "done" };
  }
  if (eventType === "reasoning.replay.missing") {
    return { label: "reasoning replay 缺失", status: "failed" };
  }
  if (eventType === "tool.protocol.policy.updated") {
    const protocol = String(payload.protocol ?? payload.selected_protocol ?? "updated");
    return { label: `工具协议策略更新: ${protocol}`, status: "done" };
  }
  if (eventType === "hook.DsmlFallbackTriggered") {
    return { label: "DSML fallback 已触发", status: "running" };
  }
  if (eventType.startsWith("deepseek.cache_plan.")) {
    const suffix = eventType.replace("deepseek.cache_plan.", "");
    return {
      label: suffix === "skipped" ? "缓存计划已跳过" : "缓存计划已更新",
      status: suffix === "skipped" ? "pending" : "done",
      kind: "observability",
      category: "cache",
      detail: suffix,
    };
  }
  if (eventType === "deepseek.cache_stats.recorded") {
    const hit = String(payload.prompt_cache_hit_tokens ?? payload.hit_tokens ?? "0");
    return {
      label: `缓存命中 ${formatTokenCount(Number(hit) || 0)} tokens`,
      status: "done",
      kind: "observability",
      category: "cache",
    };
  }
  if (eventType.startsWith("deepseek.cache.zone_")) {
    const suffix = eventType.replace("deepseek.cache.", "");
    const zone = suffix.replace(/\.(hit|miss)$/u, "").replace("zone_", "zone ");
    const isMiss = eventType.endsWith(".miss");
    const tokens = optionalFiniteNumber(payload.tokens) ?? optionalFiniteNumber(payload.prompt_tokens);
    return {
      label: `${zone} ${isMiss ? "缓存未命中" : "缓存命中"}`,
      status: isMiss ? "pending" : "done",
      kind: "observability",
      category: "cache",
      detail: tokens ? `${formatTokenCount(tokens)} tokens` : undefined,
    };
  }
  if (eventType === "convergence.disagreement") {
    const reason = String(payload.reason ?? payload.verdict ?? "").trim();
    return {
      label: reason ? `收敛分歧: ${reason}` : "收敛分歧，Agent 将继续确认",
      status: "running",
      kind: "observability",
    };
  }
  if (eventType === "tool.input_repaired" || eventType === "tool.auto_recovery") {
    const toolId = String(payload.tool_id ?? payload.resolved_tool ?? payload.requested_tool ?? "tool");
    const repair = String(payload.repair ?? payload.status ?? payload.syntax ?? "已规范化").trim();
    return {
      label: `工具输入已规范化: ${toolId}`,
      status: "done",
      kind: "observability",
      category: "repair",
      detail: repair,
    };
  }
  if (eventType === "agent.telemetry.turn_summary") {
    const cacheHitRate = formatPercent(payload.cache_hit_rate);
    const replayCount = String(payload.reasoning_replay_count ?? "0");
    const recoveryCount = String(payload.recovery_count ?? "0");
    return {
      label: `遥测汇总: cache ${cacheHitRate} · replay ${replayCount} · recovery ${recoveryCount}`,
      status: "done",
      kind: "observability",
      category: "model",
    };
  }
  if (eventType === "model.context_budget") {
    const budget = contextBudgetNumbers(payload);
    const label = budget.promptTokens && budget.maxTokens
      ? `上下文预算 ${formatTokenCount(budget.promptTokens)} / ${formatTokenCount(budget.maxTokens)}`
      : budget.promptTokens
        ? `上下文预算 ${formatTokenCount(budget.promptTokens)}`
        : "上下文预算已更新";
    return {
      label,
      status: "done",
      kind: "observability",
      category: "context",
      detail: budget.remainingTokens ? `剩余 ${formatTokenCount(budget.remainingTokens)} tokens` : undefined,
    };
  }
  if (eventType.startsWith("context.compaction.")) {
    const suffix = eventType.replace("context.compaction.", "");
    const reason = String(payload.reason ?? payload.status ?? "").trim();
    const completedDetail = compactionCompletedDetail(payload, reason);
    const status: ProgressItem["status"] =
      suffix === "blocked" || suffix === "failed" ? "failed" : suffix === "started" ? "running" : "done";
    return {
      label: compactionLabel(suffix, reason),
      status,
      kind: "observability",
      category: "context",
      detail: suffix === "completed" ? completedDetail : compactionDetail(suffix, reason),
      message: suffix === "completed" ? "上下文已压缩，后续轮次会使用压缩后的历史。" : undefined,
    };
  }
  if (eventType === "agent.recovery.output_truncated") {
    const nextMaxTokens = String(payload.next_max_tokens ?? "?");
    const reason = String(payload.stop_reason ?? "output_limit");
    return {
      label: `输出达到上限，正在续写: ${reason}`,
      status: "running",
      kind: "observability",
      category: "recovery",
      detail: `下一次 max tokens: ${nextMaxTokens}`,
      message: `模型输出达到上限，已提高输出预算到 ${nextMaxTokens} tokens 并继续。`,
    };
  }
  if (eventType === "agent.loop_budget.normalized") {
    const effective = optionalFiniteNumber(payload.effective_max_tool_calls);
    return {
      label: effective ? `工具预算已归一化: ${effective}` : "工具预算已归一化",
      status: "done",
      kind: "observability",
      category: "recovery",
      detail: String(payload.reason ?? "zero_means_uncapped_native_loop"),
    };
  }
  if (isRecoveryEventType(eventType)) {
    const suffix = eventType.replace("agent.", "");
    const reason = String(payload.reason ?? payload.error_code ?? payload.status ?? "").trim();
    const status: ProgressItem["status"] =
      suffix.includes("failed") ||
      suffix.includes("blocked") ||
      suffix.includes("stopped") ||
      suffix.includes("budget_reached") ||
      suffix.includes("incomplete") ||
      suffix.includes("exhausted")
        ? "failed"
        : suffix.includes("completed") || suffix.includes("summary") || suffix.includes("finalized")
          ? "done"
          : "running";
    return {
      label: reason ? `恢复/循环: ${suffix} · ${reason}` : `恢复/循环: ${suffix}`,
      status,
      kind: "observability",
      category: "recovery",
      detail: reason || undefined,
    };
  }
  if (eventType === "deepseek.tool_call.partial") {
    const toolName = String(payload.name_so_far ?? payload.tool_id ?? "tool");
    const bytes = String(payload.argument_delta_bytes ?? payload.argument_bytes ?? "0");
    return {
      label: `工具参数流式生成: ${toolName} (${bytes} bytes)`,
      status: "running",
      kind: "tool",
      toolId: toolName,
      category: "tool",
      detail: `${bytes} bytes`,
    };
  }
  if (eventType === "runtime.plan_approval.model_continued") {
    return {
      label: "计划已批准，模型继续执行",
      status: "done",
      kind: "observability",
      category: "recovery",
      message: "计划已批准，模型继续执行。",
    };
  }
  if (
    eventType === "subagent.child_created" ||
    eventType === "subagent.spawned" ||
    eventType === "subagent.model_turn_started" ||
    eventType === "subagent.message_sent" ||
    eventType === "subagent.message_received"
  ) {
    const agentType = String(payload.agent_type ?? "subagent");
    const subagentId = String(payload.subagent_id ?? "").trim();
    const label = eventType === "subagent.message_sent"
      ? `子 Agent 已派发: ${agentType}`
      : eventType === "subagent.message_received"
        ? `子 Agent 有回复: ${agentType}`
        : eventType === "subagent.model_turn_started"
          ? `子 Agent 推理中: ${agentType}`
          : `子 Agent 启动: ${agentType}`;
    return {
      label,
      status: "running",
      kind: "observability",
      category: "subagent",
      detail: subagentId || undefined,
    };
  }
  if (
    eventType === "subagent.completed" ||
    eventType === "subagent.summary_recorded" ||
    eventType === "subagent.model_turn_completed" ||
    eventType === "subagent.tool_completed"
  ) {
    const status = String(payload.status ?? "completed");
    const subagentId = String(payload.subagent_id ?? "").trim();
    const toolId = String(payload.tool_id ?? "").trim();
    const label = eventType === "subagent.tool_completed"
      ? `子 Agent 工具完成: ${toolId || "tool"}`
      : eventType === "subagent.summary_recorded"
        ? "子 Agent 摘要已记录"
        : eventType === "subagent.model_turn_completed"
          ? "子 Agent 推理完成"
          : "子 Agent 已完成";
    return {
      label,
      status: status === "completed" ? "done" : "running",
      kind: "observability",
      category: "subagent",
      detail: subagentId || undefined,
    };
  }
  if (
    eventType === "subagent.cancelled" ||
    eventType === "subagent.tool_blocked" ||
    eventType === "subagent.failed"
  ) {
    const reason = String(payload.reason_code ?? "failed");
    const subagentId = String(payload.subagent_id ?? "").trim();
    const label = eventType === "subagent.cancelled"
      ? `子 Agent 已取消: ${reason}`
      : eventType === "subagent.tool_blocked"
        ? `子 Agent 工具受阻: ${reason}`
        : `子 Agent 失败: ${reason}`;
    return {
      label,
      status: "failed",
      kind: "observability",
      category: "subagent",
      detail: subagentId ? `${subagentId} · ${reason}` : reason,
    };
  }
  if (eventType === "runtime.permission_resume.started") {
    const toolId = String(payload.tool_id ?? "tool");
    const toolCallId = String(payload.tool_call_id ?? "");
    const permissionId = String(payload.permission_id ?? "");
    return {
      label: `权限恢复执行: ${toolId}`,
      status: "running",
      toolCallId: toolCallId || undefined,
      toolId,
      permissionId: permissionId || undefined,
      kind: "tool",
      category: "permission",
    };
  }
  if (eventType === "runtime.permission_resume.completed") {
    const executed = payload.tool_executed !== false;
    const toolId = String(payload.tool_id ?? "tool");
    const toolCallId = String(payload.tool_call_id ?? "");
    const permissionId = String(payload.permission_id ?? "");
    return {
      label: executed ? `权限恢复完成: ${toolId}` : `权限恢复未执行工具: ${toolId}`,
      status: executed ? "done" : "failed",
      toolCallId: toolCallId || undefined,
      toolId,
      permissionId: permissionId || undefined,
      kind: "tool",
      category: "permission",
    };
  }
  if (eventType === "runtime.permission_submission.queued") {
    const permissionId = String(payload.permission_id ?? "");
    return {
      label: "权限审批已排队",
      status: "running",
      permissionId: permissionId || undefined,
      kind: "permission",
      category: "permission",
    };
  }
  if (eventType === "runtime.permission_submission.waiting_for_runtime") {
    const permissionId = String(payload.permission_id ?? "");
    return {
      label: "等待 runtime 接收审批",
      status: "running",
      permissionId: permissionId || undefined,
      kind: "permission",
      category: "permission",
    };
  }
  if (eventType === "runtime.permission_submission.accepted") {
    const toolId = String(payload.tool_id ?? "tool");
    const permissionId = String(payload.permission_id ?? "");
    return {
      label: `权限审批已接收: ${toolId}`,
      status: "done",
      toolId,
      permissionId: permissionId || undefined,
      kind: "permission",
      category: "permission",
    };
  }
  return null;
}

export function toFiniteNumber(value: unknown): number {
  const parsed = typeof value === "number" ? value : Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
}

export function addTokenUsage(current: TokenUsageStats, patch: Partial<TokenUsageStats>): TokenUsageStats {
  return {
    uploaded: Math.max(0, current.uploaded + (patch.uploaded ?? 0)),
    downloaded: Math.max(0, current.downloaded + (patch.downloaded ?? 0)),
    reasoning: Math.max(0, current.reasoning + (patch.reasoning ?? 0)),
    cacheHit: Math.max(0, current.cacheHit + (patch.cacheHit ?? 0)),
    cacheMiss: Math.max(0, current.cacheMiss + (patch.cacheMiss ?? 0)),
  };
}

export function estimateTokenDelta(preview: unknown): number {
  const text = String(preview ?? "");
  if (!text) {
    return 0;
  }
  const charsMatch = /^chars=(\d+)$/u.exec(text.trim());
  if (charsMatch) {
    return Math.max(1, Math.ceil(Number(charsMatch[1]) / 4));
  }
  const charTokens = Math.ceil([...text].length / 4);
  const wordTokens = text.trim() ? text.trim().split(/\s+/u).length : 0;
  return Math.max(1, charTokens, wordTokens);
}

function detectBlockedReason(payload: Record<string, unknown>): string {
  const raw = String(
    payload.message ?? payload.gate ?? payload.error_code ?? payload.reason ?? "model.call_blocked",
  ).toLowerCase();
  if (raw.includes("network_not_enabled")) {
    return "未开启网络访问（设置 RESEARCHCODE_ALLOW_NETWORK=1 后重启）";
  }
  if (raw.includes("missing_api_key")) {
    return "未配置 API Key（请设置 DEEPSEEK_API_KEY 或 QWEN_API_KEY）";
  }
  if (raw.includes("missing_api_key_env")) {
    return "模型端点缺少 API Key 环境变量配置";
  }
  if (raw.includes("missing_api_key_value")) {
    return "API Key 环境变量为空（请检查 DEEPSEEK_API_KEY / QWEN_API_KEY）";
  }
  if (raw.includes("disabled_by_default")) {
    return "模型实时调用未启用";
  }
  if (raw.includes("sidecar_skipped")) {
    return "模型 sidecar 跳过请求（通常是网络或 key 前置条件未满足）";
  }
  if (raw.includes("secret_detected")) {
    return "请求包含敏感信息，已被安全策略阻止";
  }
  return String(payload.message ?? payload.gate ?? payload.error_code ?? "model.call_blocked");
}

function contextBudgetNumbers(payload: Record<string, unknown>): {
  promptTokens: number | null;
  maxTokens: number | null;
  remainingTokens: number | null;
} {
  const promptTokens =
    optionalFiniteNumber(payload.prompt_tokens) ??
    optionalFiniteNumber(payload.prompt_tokens_estimate) ??
    optionalFiniteNumber(payload.estimated_total_tokens) ??
    optionalFiniteNumber(payload.estimated_request_tokens) ??
    optionalFiniteNumber(payload.token_estimate_before) ??
    null;
  const maxTokens =
    optionalFiniteNumber(payload.max_context_tokens) ??
    optionalFiniteNumber(payload.hard_limit_tokens) ??
    optionalFiniteNumber(payload.target_limit_tokens) ??
    optionalFiniteNumber(payload.context_budget) ??
    optionalFiniteNumber(payload.budget_tokens) ??
    null;
  const remainingTokens =
    optionalFiniteNumber(payload.budget_remaining) ??
    (promptTokens && maxTokens ? Math.max(0, maxTokens - promptTokens) : null);
  return { promptTokens, maxTokens, remainingTokens };
}

function compactionLabel(suffix: string, reason: string): string {
  if (suffix === "skipped") {
    return "暂未压缩上下文";
  }
  if (suffix === "started") {
    return "正在压缩上下文";
  }
  if (suffix === "completed") {
    return "上下文已压缩";
  }
  if (suffix === "blocked" || suffix === "failed") {
    return "上下文压缩受阻";
  }
  return reason ? `上下文压缩 ${suffix}` : "上下文压缩状态更新";
}

function compactionDetail(suffix: string, reason: string): string | undefined {
  if (suffix === "skipped" && reason === "below_threshold") {
    return "当前上下文低于压缩阈值，暂时无需压缩。";
  }
  if (suffix === "skipped") {
    return reason ? `跳过原因: ${reason}` : "暂时无需压缩。";
  }
  return reason || undefined;
}

function compactionContextPressureLabel(payload: Record<string, unknown>): string {
  const before = optionalFiniteNumber(payload.token_estimate_before);
  const after = optionalFiniteNumber(payload.token_estimate_after) ?? optionalFiniteNumber(payload.prompt_tokens_after_injection);
  if (before && after) {
    return `上下文已压缩 ${formatTokenCount(before)} -> ${formatTokenCount(after)}`;
  }
  return "上下文已压缩";
}

function compactionCompletedDetail(payload: Record<string, unknown>, reason: string): string | undefined {
  const parts: string[] = [];
  const before = optionalFiniteNumber(payload.token_estimate_before);
  const after = optionalFiniteNumber(payload.token_estimate_after) ?? optionalFiniteNumber(payload.prompt_tokens_after_injection);
  if (before && after) {
    parts.push(`压缩 ${formatTokenCount(before)} -> ${formatTokenCount(after)}`);
  }
  const spineCounts = contextSpineCounts(payload.spine_json);
  if (spineCounts) {
    parts.push(`L1 ${spineCounts}`);
  }
  const summary = String(payload.summary ?? "").trim();
  const refCount = (summary.match(/ref:\/\/event\//g) ?? []).length;
  if (refCount > 0) {
    parts.push(`refs ${refCount}`);
  }
  if (reason) {
    parts.push(`原因: ${reason}`);
  }
  return parts.length > 0 ? parts.join(" · ") : undefined;
}

function contextSpineCounts(raw: unknown): string | null {
  const spine = parseSpineJson(raw);
  if (!spine || typeof spine !== "object" || Array.isArray(spine)) {
    return null;
  }
  const record = spine as Record<string, unknown>;
  const facts = Array.isArray(record.confirmed_facts) ? record.confirmed_facts.length : 0;
  const observations = Array.isArray(record.observations) ? record.observations.length : 0;
  const resources = Array.isArray(record.resources) ? record.resources.length : 0;
  const decisions = Array.isArray(record.decisions) ? record.decisions.length : 0;
  const chunks = [
    facts > 0 ? `${facts} facts` : null,
    observations > 0 ? `${observations} obs` : null,
    resources > 0 ? `${resources} resources` : null,
    decisions > 0 ? `${decisions} decisions` : null,
  ].filter(Boolean);
  return chunks.length > 0 ? chunks.join("/") : null;
}

function parseSpineJson(raw: unknown): unknown {
  if (!raw) {
    return null;
  }
  if (typeof raw === "object") {
    return raw;
  }
  if (typeof raw !== "string") {
    return null;
  }
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function isRecoveryEventType(eventType: string): boolean {
  return (
    eventType.startsWith("agent.recovery.") ||
    eventType.startsWith("agent.loop_") ||
    eventType.startsWith("agent.fast_auto_write.") ||
    eventType === "agent.loop_recovery" ||
    eventType === "agent.loop_budget_reached" ||
    eventType === "agent.loop_plateau_stopped" ||
    eventType === "agent.continuation_summary"
  );
}

function optionalFiniteNumber(value: unknown): number | null {
  const parsed = toFiniteNumber(value);
  return parsed > 0 ? parsed : null;
}

function formatPercent(value: unknown): string {
  const parsed = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return "0.0%";
  }
  return `${(parsed * 100).toFixed(1)}%`;
}

function formatTokenCount(value: number): string {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 10_000) {
    return `${Math.round(value / 1000)}k`;
  }
  if (value >= 1000) {
    return `${(value / 1000).toFixed(1)}k`;
  }
  return String(value);
}
