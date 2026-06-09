import type { PendingPermissionState, PendingPlanApprovalState } from "@/hooks/usePermissionFlow";
import {
  completePermissionProgress,
  completeToolProgress,
  upsertPermissionProgress,
  upsertToolProgress,
} from "@/runtime/progressLedger";
import { trimProgressItems } from "@/runtime/runStore";
import {
  addTokenUsage,
  autonomyModeLabel,
  estimateTokenDelta,
  messageFromRuntimeError,
  normalizeAutonomyMode,
  runtimeObservabilityProgress,
  runtimeStateToRunStatus,
  type RuntimeAutonomyMode,
  toFiniteNumber,
} from "@/runtime/runtimeEventViewModel";
import type { ProgressItem, RunStatus, TokenUsageStats, TranscriptMessage } from "@/types";

type ProgressIdFactory = (prefix: string) => string;

export interface PermissionEventReduction {
  handled: boolean;
  pendingPermissions: PendingPermissionState[];
  message?: string;
  completedPermissionId?: string;
  clearDecisionForPermissionId?: string;
}

export interface PlanEventReduction {
  handled: boolean;
  pendingPlanApprovals: PendingPlanApprovalState[];
  planPreviewCache: Map<string, string>;
  message?: string;
  completedPlanApprovalId?: string;
  clearDecisionForPlanApprovalId?: string;
}

export interface ToolEventReduction {
  handled: boolean;
  progressItems: ProgressItem[];
  message?: Omit<TranscriptMessage, "id" | "timestamp">;
  discardActiveStreamingMessage?: boolean;
  settleActiveStreamingMessage?: boolean;
  recoverableToolFailure?: {
    toolCallId: string;
    toolId: string;
  };
}

export interface ModelStreamEstimates {
  downloadedByStreamId: Map<string, number>;
  reasoningByStreamId: Map<string, number>;
}

export interface ModelEventReduction {
  handled: boolean;
  tokenUsage: TokenUsageStats;
  estimates: ModelStreamEstimates;
  progressItem?: Omit<ProgressItem, "id">;
  streamChunk?: {
    preview: string;
    runtimeSanitized: boolean;
  };
  commitStreamingMessage?: string;
  scheduleStreamCommit?: boolean;
  suppressNextCallCompletedSettle?: boolean;
}

function isOutputTruncationStopReason(value: unknown): boolean {
  const reason = String(value ?? "").trim().toLowerCase();
  return (
    reason === "length" ||
    reason === "max_tokens" ||
    reason === "max_output_tokens" ||
    reason === "output_limit" ||
    reason === "output_token_limit" ||
    reason === "token_limit" ||
    reason.includes("max_tokens") ||
    reason.includes("max output") ||
    reason.includes("output token")
  );
}

function streamCompletionProgress(payload: Record<string, unknown>): Omit<ProgressItem, "id"> | undefined {
  const stopReason = String(
    payload.stop_reason ?? payload.finish_reason ?? payload.provider_stop_reason ?? payload.provider_finish_reason ?? "",
  ).trim();
  if (!stopReason) {
    return undefined;
  }
  const normalized = stopReason.toLowerCase();
  const streamId = String(payload.stream_id ?? "default");
  const completionTokens = toFiniteNumber(payload.completion_tokens);
  const reasoningTokens = toFiniteNumber(payload.reasoning_tokens);
  const parts = [`stream ${streamId}`];
  if (completionTokens > 0) parts.push(`${completionTokens} completion`);
  if (reasoningTokens > 0) parts.push(`${reasoningTokens} reasoning`);
  if (isOutputTruncationStopReason(stopReason)) {
    return {
      label: `模型输出达到上限: ${stopReason}`,
      status: "running",
      kind: "observability",
      category: "recovery",
      detail: parts.join(" · "),
      eventType: "model.stream_completed",
    };
  }
  if (normalized === "stop" || normalized === "end_turn" || normalized === "tool_use") {
    return undefined;
  }
  return {
    label: `模型流结束: ${stopReason}`,
    status: "done",
    kind: "observability",
    category: "model",
    detail: parts.join(" · "),
    eventType: "model.stream_completed",
  };
}

export interface SessionEventReduction {
  handled: boolean;
  progressItems: ProgressItem[];
  runStatus?: RunStatus;
  autonomyMode?: RuntimeAutonomyMode;
  stopStreaming?: boolean;
  message?: Omit<TranscriptMessage, "id" | "timestamp">;
}

export function reduceSessionRuntimeEvent(
  progressItems: ProgressItem[],
  eventType: string,
  payload: Record<string, unknown>,
  makeId: ProgressIdFactory,
): SessionEventReduction {
  if (eventType === "session.state_changed") {
    return {
      handled: true,
      progressItems,
      runStatus: runtimeStateToRunStatus(String(payload.to_state ?? "")),
    };
  }

  if (eventType === "session.autonomy_mode_changed") {
    const mode = normalizeAutonomyMode(payload.autonomy_mode);
    return {
      handled: true,
      progressItems: trimProgressItems([
        ...progressItems,
        {
          id: makeId("progress"),
          label: `自主模式已切换: ${autonomyModeLabel(mode)}`,
          status: "done",
          kind: "observability",
        },
      ]),
      autonomyMode: mode,
    };
  }

  if (eventType === "runtime.turn_cancel_requested") {
    return {
      handled: true,
      progressItems,
      runStatus: "stopped",
      stopStreaming: true,
      message: {
        role: "system",
        type: "text",
        content: "已中断当前轮。",
      },
    };
  }

  if (eventType === "agent.loop_stopped") {
    const reason = String(payload.reason ?? payload.category ?? "stopped").trim();
    const stoppedStatus = String(payload.status ?? "").trim();
    const blockedStop = stoppedStatus === "blocked";
    const observability = runtimeObservabilityProgress(eventType, payload);
    return {
      handled: true,
      progressItems: trimProgressItems([
        ...progressItems,
        {
          id: makeId("progress"),
          label: observability?.label ?? (reason ? `本轮已停止: ${reason}` : "本轮已停止"),
          status: blockedStop ? "done" : (observability?.status ?? "failed"),
          kind: "observability",
          category: observability?.category ?? "recovery",
          detail: observability?.detail ?? (reason || undefined),
          eventType,
        },
      ]),
      runStatus: blockedStop ? "stopped" : "failed",
      stopStreaming: true,
      message: {
        role: "system",
        type: "text",
        content: reason ? `本轮已停止: ${reason}` : "本轮已停止。",
      },
    };
  }

  if (eventType === "agent.loop_incomplete") {
    const observability = runtimeObservabilityProgress(eventType, payload);
    return {
      handled: true,
      progressItems: trimProgressItems([
        ...progressItems,
        {
          id: makeId("progress"),
          label: observability?.label ?? "本轮未完整结束",
          status: observability?.status ?? "failed",
          kind: "observability",
          category: observability?.category ?? "recovery",
          detail: observability?.detail,
          eventType,
        },
      ]),
      runStatus: "failed",
      stopStreaming: true,
    };
  }

  return { handled: false, progressItems };
}

export interface RuntimeErrorEventReduction {
  handled: boolean;
  runStatus?: RunStatus;
  stopStreaming?: boolean;
  message?: Omit<TranscriptMessage, "id" | "timestamp">;
  permissionError?: {
    permissionId: string;
    message: string;
  };
}

export interface ObservabilityEventReduction {
  handled: boolean;
  progressItems: ProgressItem[];
  message?: Omit<TranscriptMessage, "id" | "timestamp">;
  clearDecisionForPermissionId?: string;
  completedPermissionId?: string;
}

export function reduceObservabilityRuntimeEvent(
  progressItems: ProgressItem[],
  eventType: string,
  payload: Record<string, unknown>,
  makeId: ProgressIdFactory,
): ObservabilityEventReduction {
  const observability = runtimeObservabilityProgress(eventType, payload);
  if (!observability) {
    return { handled: false, progressItems };
  }

  let nextProgressItems = progressItems;
  if (observability.kind === "permission") {
    nextProgressItems = upsertPermissionProgress(progressItems, {
      id: makeId("progress"),
      label: observability.label,
      status: observability.status,
      permissionId: observability.permissionId,
      toolId: observability.toolId,
      category: observability.category ?? diagnosticCategoryFromEvent(eventType),
      detail: observability.detail,
      eventType,
    });
  } else if (observability.kind === "tool") {
    const withTool = observability.status === "running"
      ? upsertToolProgress(progressItems, {
          id: makeId("progress"),
          label: observability.label,
          status: observability.status,
          toolCallId: observability.toolCallId,
          toolId: observability.toolId,
          category: observability.category ?? diagnosticCategoryFromEvent(eventType),
          detail: observability.detail,
          eventType,
        })
      : completeToolProgress(progressItems, {
          toolCallId: observability.toolCallId,
          toolId: observability.toolId,
          status: observability.status === "failed" ? "failed" : "done",
          category: observability.category ?? diagnosticCategoryFromEvent(eventType),
          detail: observability.detail,
          eventType,
        });
    nextProgressItems = observability.permissionId
      ? completePermissionProgress(
          withTool,
          observability.permissionId,
          observability.status === "failed" ? "failed" : "done",
          {
            category: observability.category ?? diagnosticCategoryFromEvent(eventType),
            detail: observability.detail,
            eventType,
          },
        )
      : withTool;
  } else {
    nextProgressItems = trimProgressItems([
      ...progressItems,
      {
        id: makeId("progress"),
        label: observability.label,
        status: observability.status,
        kind: observability.kind,
        category: observability.category ?? diagnosticCategoryFromEvent(eventType),
        detail: observability.detail,
        eventType,
      },
    ]);
  }

  const permissionId = eventType === "runtime.permission_resume.completed"
    ? String(payload.permission_id ?? "")
    : "";
  const permissionResumeToolId = eventType === "runtime.permission_resume.completed"
    ? String(payload.tool_id ?? "")
    : "";
  const permissionResumeToolCallId = eventType === "runtime.permission_resume.completed"
    ? String(payload.tool_call_id ?? "")
    : "";
  const permissionResumeExecuted = eventType === "runtime.permission_resume.completed"
    ? payload.tool_executed !== false && payload.ok !== false
    : false;
  return {
    handled: true,
    progressItems: nextProgressItems,
    message: permissionResumeExecuted && permissionResumeToolId
      ? {
          role: "agent",
          type: "tool_call",
          content: `工具完成: ${permissionResumeToolId}`,
          metadata: {
            tool_id: permissionResumeToolId,
            tool_call_id: permissionResumeToolCallId || undefined,
            tool_phase: "completed",
            permission_id: permissionId || undefined,
          },
        }
      : observability.message
        ? {
          role: "system",
          type: "text",
          content: observability.message,
        }
        : undefined,
    clearDecisionForPermissionId: permissionId || undefined,
    completedPermissionId: permissionId || undefined,
  };
}

export function reduceRuntimeErrorEvent(
  eventType: string,
  payload: Record<string, unknown>,
): RuntimeErrorEventReduction {
  if (
    eventType !== "runtime.error" &&
    eventType !== "tool.error.model_readable" &&
    eventType !== "model.call_blocked"
  ) {
    return { handled: false };
  }
  const content = messageFromRuntimeError(eventType, payload);
  const permissionId = String(payload.permission_id ?? "");
  return {
    handled: true,
    runStatus: eventType === "model.call_blocked" ? "failed" : undefined,
    stopStreaming: eventType === "model.call_blocked",
    message: {
      role: "system",
      type: "text",
      content,
    },
    permissionError: permissionId
      ? {
          permissionId,
          message: content,
        }
      : undefined,
  };
}

export function reduceModelRuntimeEvent(
  tokenUsage: TokenUsageStats,
  eventType: string,
  payload: Record<string, unknown>,
  estimates: ModelStreamEstimates,
): ModelEventReduction {
  const nextEstimates: ModelStreamEstimates = {
    downloadedByStreamId: new Map(estimates.downloadedByStreamId),
    reasoningByStreamId: new Map(estimates.reasoningByStreamId),
  };

  if (eventType === "model.call_started") {
    const uploaded = toFiniteNumber(payload.prompt_tokens_estimate);
    return {
      handled: true,
      tokenUsage: uploaded > 0 ? addTokenUsage(tokenUsage, { uploaded }) : tokenUsage,
      estimates: nextEstimates,
    };
  }

  if (eventType === "model.stream_delta") {
    const deltaKind = String(payload.delta_kind ?? "content");
    const streamId = String(payload.stream_id ?? "default");
    let nextTokenUsage = tokenUsage;
    if (deltaKind === "content" || deltaKind === "reasoning_sanitized" || deltaKind === "thinking_sanitized") {
      const estimatedTokens = estimateTokenDelta(payload.preview);
      if (estimatedTokens > 0) {
        nextEstimates.downloadedByStreamId.set(
          streamId,
          (nextEstimates.downloadedByStreamId.get(streamId) ?? 0) + estimatedTokens,
        );
        if (deltaKind !== "content") {
          nextEstimates.reasoningByStreamId.set(
            streamId,
            (nextEstimates.reasoningByStreamId.get(streamId) ?? 0) + estimatedTokens,
          );
        }
        nextTokenUsage = addTokenUsage(nextTokenUsage, {
          downloaded: estimatedTokens,
          reasoning: deltaKind === "content" ? 0 : estimatedTokens,
        });
      }
    }
    return {
      handled: true,
      tokenUsage: nextTokenUsage,
      estimates: nextEstimates,
      streamChunk: deltaKind === "content"
        ? {
            preview: String(payload.preview ?? ""),
            runtimeSanitized: payload.runtime_sanitized === true,
          }
        : undefined,
    };
  }

  if (eventType === "model.stream_completed") {
    const streamId = String(payload.stream_id ?? "default");
    const actualDownloaded = toFiniteNumber(payload.completion_tokens) + toFiniteNumber(payload.reasoning_tokens);
    const estimatedDownloaded = nextEstimates.downloadedByStreamId.get(streamId) ?? 0;
    const estimatedReasoning = nextEstimates.reasoningByStreamId.get(streamId) ?? 0;
    nextEstimates.downloadedByStreamId.delete(streamId);
    nextEstimates.reasoningByStreamId.delete(streamId);
    const downloadedAdjustment = actualDownloaded > 0 ? actualDownloaded - estimatedDownloaded : 0;
    const reasoningActual = toFiniteNumber(payload.reasoning_tokens);
    const reasoningAdjustment = reasoningActual > 0 ? reasoningActual - estimatedReasoning : 0;
    return {
      handled: true,
      tokenUsage: addTokenUsage(tokenUsage, {
        downloaded: downloadedAdjustment,
        reasoning: reasoningAdjustment,
        cacheHit: toFiniteNumber(payload.prompt_cache_hit_tokens),
        cacheMiss: toFiniteNumber(payload.prompt_cache_miss_tokens),
      }),
      estimates: nextEstimates,
      progressItem: streamCompletionProgress(payload),
      suppressNextCallCompletedSettle: isOutputTruncationStopReason(payload.stop_reason),
    };
  }

  if (eventType === "model.call_completed") {
    return {
      handled: true,
      tokenUsage,
      estimates: nextEstimates,
      scheduleStreamCommit: true,
    };
  }

  if (eventType === "assistant.message") {
    if (String(payload.reason ?? "") === "permission_resume_tool_completed") {
      return {
        handled: true,
        tokenUsage,
        estimates: nextEstimates,
      };
    }
    return {
      handled: true,
      tokenUsage,
      estimates: nextEstimates,
      commitStreamingMessage: String(payload.content ?? ""),
    };
  }

  return {
    handled: false,
    tokenUsage,
    estimates: nextEstimates,
  };
}

export function reduceToolRuntimeEvent(
  progressItems: ProgressItem[],
  eventType: string,
  payload: Record<string, unknown>,
  makeId: ProgressIdFactory,
): ToolEventReduction {
  if (eventType === "runtime.stream.narration") {
    const content = String(payload.content ?? "").trim();
    if (!content) {
      return { handled: true, progressItems };
    }
    return {
      handled: true,
      progressItems,
      message: {
        role: "agent",
        type: "text",
        content,
        metadata: {
          runtimeNarration: true,
          reason: String(payload.reason ?? ""),
          streamId: String(payload.stream_id ?? ""),
        },
      },
    };
  }

  if (eventType === "runtime.stream.preamble_suppressed") {
    const reason = String(payload.reason ?? "");
    if (reason === "tool_call_stream_preamble" || reason === "tool_call_live_preamble_retracted") {
      return {
        handled: true,
        progressItems,
        settleActiveStreamingMessage: true,
      };
    }
  }

  if (eventType === "tool.call.assembled") {
    const toolId = String(payload.tool_id ?? "unknown");
    const toolCallId = String(payload.tool_call_id ?? "");
    return {
      handled: true,
      progressItems: trimProgressItems([
        ...progressItems,
        {
          id: makeId("progress"),
          label: `参数已组装 ${toolId}`,
          status: "done",
          toolCallId: toolCallId || undefined,
        },
      ]),
    };
  }

  if (eventType === "tool.call_requested" || eventType === "agent.tool.pending") {
    const toolId = String(payload.tool_id ?? "unknown");
    const toolCallId = String(payload.tool_call_id ?? "");
    return {
      handled: true,
      progressItems: upsertToolProgress(progressItems, {
        id: makeId("progress"),
        label: `运行 ${toolId}`,
        status: "running",
        toolCallId: toolCallId || undefined,
        toolId,
      }),
      message: {
        role: "agent",
        type: "tool_call",
        content: `调用工具: ${toolId}`,
        metadata: {
          tool_id: toolId,
          tool_call_id: toolCallId || undefined,
          tool_phase: "requested",
        },
      },
    };
  }

  if (eventType === "tool.call_completed" || eventType === "agent.tool.completed") {
    const toolId = String(payload.tool_id ?? "unknown");
    const toolCallId = String(payload.tool_call_id ?? "");
    const ok = payload.ok !== false;
    return {
      handled: true,
      progressItems: completeToolProgress(progressItems, {
        toolCallId: toolCallId || undefined,
        toolId,
        status: ok ? "done" : "failed",
      }),
      message: {
        role: "agent",
        type: "tool_call",
        content: `${ok ? "工具完成" : "工具失败"}: ${toolId}`,
        metadata: {
          tool_id: toolId,
          tool_call_id: toolCallId || undefined,
          tool_phase: ok ? "completed" : "failed",
        },
      },
    };
  }

  if (eventType === "tool.result_recorded") {
    const toolId = String(payload.tool_id ?? "");
    const toolCallId = String(payload.tool_call_id ?? "");
    const recoverableObservation = isRecoverableToolObservationPreview(payload.preview);
    return {
      handled: true,
      progressItems: completeToolProgress(progressItems, {
        toolCallId: toolCallId || undefined,
        toolId: toolId || undefined,
        status: "done",
        allowTerminalUpdate: recoverableObservation,
      }),
      recoverableToolFailure: recoverableObservation && toolCallId
        ? {
            toolCallId,
            toolId: toolId || "tool",
          }
        : undefined,
    };
  }

  return { handled: false, progressItems };
}

function isRecoverableToolObservationPreview(value: unknown): boolean {
  const preview = String(value ?? "").trim().toLowerCase();
  if (!preview) {
    return false;
  }
  if (preview.includes("duplicate observation")) {
    return true;
  }
  return preview.startsWith("tool error ") && preview.includes("next_action=");
}

function diagnosticCategoryFromEvent(eventType: string): ProgressItem["category"] {
  if (eventType.startsWith("context.") || eventType === "model.context_budget") {
    return "context";
  }
  if (eventType.startsWith("agent.") || eventType.includes("recovery")) {
    return "recovery";
  }
  if (eventType.startsWith("deepseek.cache")) {
    return "cache";
  }
  if (eventType.startsWith("tool.input_") || eventType.includes("repair")) {
    return "repair";
  }
  if (eventType.startsWith("permission.") || eventType.startsWith("runtime.permission_")) {
    return "permission";
  }
  if (eventType.startsWith("subagent.")) {
    return "subagent";
  }
  if (eventType.startsWith("model.") || eventType.startsWith("deepseek.") || eventType.startsWith("qwen.")) {
    return "model";
  }
  return "other";
}

export function reducePermissionRuntimeEvent(
  pendingPermissions: PendingPermissionState[],
  eventType: string,
  payload: Record<string, unknown>,
): PermissionEventReduction {
  if (eventType === "permission.requested") {
    const permissionId = String(payload.permission_id ?? "");
    const requestType = String(payload.request_type ?? "unknown");
    const rawToolId = String(payload.tool_id ?? "").trim();
    const toolId = rawToolId || (requestType === "command" ? "shell.command" : "unknown");
    return {
      handled: true,
      pendingPermissions: [
        ...pendingPermissions.filter((item) => item.permission_id !== permissionId),
        {
          permission_id: permissionId,
          request_type: requestType,
          tool_id: toolId,
          args_preview: String(payload.args_preview ?? ""),
          path_preview: String(payload.path_preview ?? ""),
          risk_level: String(payload.risk_level ?? ""),
        },
      ],
      message: `需要权限审批: ${requestType} (${permissionId})`,
    };
  }

  if (eventType === "permission.context") {
    const permissionId = String(payload.permission_id ?? "");
    if (!permissionId) {
      return { handled: true, pendingPermissions };
    }
    return {
      handled: true,
      pendingPermissions: pendingPermissions.map((item) =>
        item.permission_id === permissionId
          ? {
              ...item,
              tool_id: String(payload.tool_id ?? item.tool_id),
              request_type: String(payload.request_type ?? item.request_type),
              args_preview: String(payload.args_preview ?? item.args_preview ?? ""),
              path_preview: String(payload.path_preview ?? item.path_preview ?? ""),
              risk_level: String(payload.risk_level ?? item.risk_level ?? ""),
            }
          : item,
      ),
    };
  }

  if (eventType === "permission.decided") {
    const permissionId = String(payload.permission_id ?? "");
    const decision = String(payload.decision ?? "unknown");
    return {
      handled: true,
      pendingPermissions: pendingPermissions.filter((item) => item.permission_id !== permissionId),
      message: `权限已处理: ${permissionId} (${decision})`,
      completedPermissionId: permissionId,
      clearDecisionForPermissionId: permissionId,
    };
  }

  if (eventType === "permission.suggestion_submitted") {
    const permissionId = String(payload.permission_id ?? "");
    const feedback = String(payload.feedback ?? "").trim();
    return {
      handled: true,
      pendingPermissions,
      message: feedback ? `权限建议已提交: ${permissionId}\n${feedback}` : `权限建议已提交: ${permissionId}`,
    };
  }

  return { handled: false, pendingPermissions };
}

export function reducePlanApprovalRuntimeEvent(
  pendingPlanApprovals: PendingPlanApprovalState[],
  eventType: string,
  payload: Record<string, unknown>,
  planPreviewCache: ReadonlyMap<string, string>,
): PlanEventReduction {
  const nextCache = new Map(planPreviewCache);

  if (eventType === "plan.mode_entered") {
    const planApprovalId = String(payload.plan_approval_id ?? "");
    const planPreview = String(payload.plan_preview ?? "").trim();
    if (planApprovalId && planPreview) {
      nextCache.set(planApprovalId, planPreview);
    }
    return {
      handled: true,
      pendingPlanApprovals,
      planPreviewCache: nextCache,
    };
  }

  if (eventType === "plan.approval_requested") {
    const planApprovalId = String(payload.plan_approval_id ?? "");
    if (!planApprovalId) {
      return {
        handled: true,
        pendingPlanApprovals,
        planPreviewCache: nextCache,
      };
    }
    const goal = String(payload.goal ?? "");
    const planPreview = String(
      payload.plan_preview ?? nextCache.get(planApprovalId) ?? goal,
    ).trim();
    return {
      handled: true,
      pendingPlanApprovals: [
        ...pendingPlanApprovals.filter((item) => item.plan_approval_id !== planApprovalId),
        { plan_approval_id: planApprovalId, goal, plan_preview: planPreview || undefined },
      ],
      planPreviewCache: nextCache,
      message: `计划等待审批: ${goal || planApprovalId}`,
    };
  }

  if (eventType === "plan.approval_decided") {
    const planApprovalId = String(payload.plan_approval_id ?? "");
    if (!planApprovalId) {
      return {
        handled: true,
        pendingPlanApprovals,
        planPreviewCache: nextCache,
      };
    }
    nextCache.delete(planApprovalId);
    return {
      handled: true,
      pendingPlanApprovals: pendingPlanApprovals.filter((item) => item.plan_approval_id !== planApprovalId),
      planPreviewCache: nextCache,
      message: `计划审批已处理: ${planApprovalId}`,
      completedPlanApprovalId: planApprovalId,
      clearDecisionForPlanApprovalId: planApprovalId,
    };
  }

  if (eventType === "plan.mode_exited") {
    const planApprovalId = String(payload.plan_approval_id ?? "");
    if (!planApprovalId) {
      return {
        handled: true,
        pendingPlanApprovals,
        planPreviewCache: nextCache,
      };
    }
    nextCache.delete(planApprovalId);
    return {
      handled: true,
      pendingPlanApprovals: pendingPlanApprovals.filter((item) => item.plan_approval_id !== planApprovalId),
      planPreviewCache: nextCache,
      message: `计划模式已退出: ${planApprovalId}`,
      completedPlanApprovalId: planApprovalId,
      clearDecisionForPlanApprovalId: planApprovalId,
    };
  }

  return {
    handled: false,
    pendingPlanApprovals,
    planPreviewCache: nextCache,
  };
}
