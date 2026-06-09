import { useCallback, useRef, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import { completePermissionProgress } from "@/runtime/progressLedger";
import { trimProgressItems } from "@/runtime/runStore";
import {
  coalesceAdjacentRuntimeEvents,
  eventFallbackDedupKey,
} from "@/runtime/runtimeEventStream";
import {
  reduceModelRuntimeEvent,
  reduceObservabilityRuntimeEvent,
  reducePermissionRuntimeEvent,
  reducePlanApprovalRuntimeEvent,
  reduceRuntimeErrorEvent,
  reduceSessionRuntimeEvent,
  reduceToolRuntimeEvent,
} from "@/runtime/runtimeEventReducer";
import {
  contextPressureFromRuntimeEvent,
  recoveryStatusFromRuntimeEvent,
} from "@/runtime/runtimeEventViewModel";
import {
  repairPseudoMarkdownNewlines,
  trimLeadingBlankLines,
} from "@/runtime/streamSanitizer";
import type { RuntimeEvent } from "@/runtime/localRuntimeClient";
import type { ContextPressureState, RecoveryStatusState } from "@/components/RuntimeStatus";
import type { PendingPermissionState, PendingPlanApprovalState } from "./usePermissionFlow";
import type { ProgressItem, RunStatus, TokenUsageStats, TranscriptMessage } from "@/types";
import type { AutonomyModeId } from "@/components/Topbar";

type AppendMessage = (message: Omit<TranscriptMessage, "id" | "timestamp">) => void;

interface UseRuntimeEventApplicationOptions {
  pendingPermissions: PendingPermissionState[];
  pendingPlanApprovals: PendingPlanApprovalState[];
  emptyTokenUsage: TokenUsageStats;
  seenEventKeysRef: MutableRefObject<Set<string>>;
  streamDownloadEstimateRef: MutableRefObject<Map<string, number>>;
  streamReasoningEstimateRef: MutableRefObject<Map<string, number>>;
  planPreviewByApprovalIdRef: MutableRefObject<Map<string, string>>;
  callCompletedSettleTimerRef: MutableRefObject<ReturnType<typeof setTimeout> | null>;
  appendMessage: AppendMessage;
  applyStreamChunk: (chunk: string) => void;
  discardActiveStreamingMessage: () => void;
  commitStreamingMessage: (finalContent: string) => void;
  sanitizeStreamChunk: (rawChunk: string) => string;
  setAutonomyMode: Dispatch<SetStateAction<AutonomyModeId>>;
  setContextPressure: Dispatch<SetStateAction<ContextPressureState>>;
  setIsStreaming: Dispatch<SetStateAction<boolean>>;
  setMessages: Dispatch<SetStateAction<TranscriptMessage[]>>;
  setPendingPermissions: Dispatch<SetStateAction<PendingPermissionState[]>>;
  setPendingPlanApprovals: Dispatch<SetStateAction<PendingPlanApprovalState[]>>;
  setPermissionDecisionErrors: Dispatch<SetStateAction<Record<string, string>>>;
  setPermissionDecisionInFlight: Dispatch<SetStateAction<string | null>>;
  setPlanDecisionErrors: Dispatch<SetStateAction<Record<string, string>>>;
  setPlanDecisionInFlight: Dispatch<SetStateAction<string | null>>;
  setProgressItems: Dispatch<SetStateAction<ProgressItem[]>>;
  setRecoveryStatus: Dispatch<SetStateAction<RecoveryStatusState | null>>;
  setRunStatus: Dispatch<SetStateAction<RunStatus>>;
  setTokenUsage: Dispatch<SetStateAction<TokenUsageStats>>;
}

function makeReducerId(prefix: string): string {
  return `${prefix}_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
}

function markRecoverableToolFailureRecorded(
  messages: TranscriptMessage[],
  toolCallId: string,
  toolId: string,
): TranscriptMessage[] {
  if (!toolCallId) {
    return messages;
  }
  let changed = false;
  const next = messages.map((message) => {
    const metadata = (message.metadata ?? {}) as Record<string, unknown>;
    if (
      message.type !== "tool_call" ||
      metadata.tool_call_id !== toolCallId ||
      metadata.tool_phase !== "failed"
    ) {
      return message;
    }
    changed = true;
    return {
      ...message,
      content: `工具已反馈: ${toolId}`,
      metadata: {
        ...metadata,
        tool_phase: "completed",
        tool_recoverable_observation: true,
      },
    };
  });
  return changed ? next : messages;
}

export function useRuntimeEventApplication({
  pendingPermissions,
  pendingPlanApprovals,
  emptyTokenUsage,
  seenEventKeysRef,
  streamDownloadEstimateRef,
  streamReasoningEstimateRef,
  planPreviewByApprovalIdRef,
  callCompletedSettleTimerRef,
  appendMessage,
  applyStreamChunk,
  discardActiveStreamingMessage,
  commitStreamingMessage,
  sanitizeStreamChunk,
  setAutonomyMode,
  setContextPressure,
  setIsStreaming,
  setMessages,
  setPendingPermissions,
  setPendingPlanApprovals,
  setPermissionDecisionErrors,
  setPermissionDecisionInFlight,
  setPlanDecisionErrors,
  setPlanDecisionInFlight,
  setProgressItems,
  setRecoveryStatus,
  setRunStatus,
  setTokenUsage,
}: UseRuntimeEventApplicationOptions) {
  const suppressNextCallCompletedSettleRef = useRef(false);
  const terminalStreamClosedRef = useRef(false);

  return useCallback((events: RuntimeEvent[]) => {
    const seenEventKeys = seenEventKeysRef.current;
    for (const event of coalesceAdjacentRuntimeEvents(events)) {
      const dedupKey =
        event.event_id && event.event_id.trim()
          ? `id:${event.event_id}`
          : typeof event.sequence === "number"
            ? `seq:${event.sequence}`
            : eventFallbackDedupKey(event);
      if (dedupKey) {
        if (seenEventKeys.has(dedupKey)) {
          continue;
        }
        seenEventKeys.add(dedupKey);
        if (seenEventKeys.size > 5000) {
          seenEventKeysRef.current = new Set(Array.from(seenEventKeys).slice(-3000));
        }
      }
      const payload = (event.payload ?? {}) as Record<string, unknown>;
      const eventType = event.event_type;
      if (eventType === "model.context_budget" || eventType.startsWith("context.compaction.")) {
        setContextPressure((current) => contextPressureFromRuntimeEvent(eventType, payload, current) ?? current);
      }
      const recoveryStatusUpdate = recoveryStatusFromRuntimeEvent(eventType, payload);
      if (recoveryStatusUpdate) {
        setRecoveryStatus(recoveryStatusUpdate);
      }
      const sessionReduction = reduceSessionRuntimeEvent([], eventType, payload, makeReducerId);
      if (sessionReduction.handled) {
        if (eventType === "session.state_changed" && String(payload.to_state ?? "") === "Executing") {
          terminalStreamClosedRef.current = false;
        }
        if (sessionReduction.runStatus) {
          setRunStatus(sessionReduction.runStatus);
        }
        if (sessionReduction.autonomyMode) {
          setAutonomyMode(sessionReduction.autonomyMode);
        }
        if (sessionReduction.stopStreaming) {
          terminalStreamClosedRef.current = true;
          commitStreamingMessage("");
        }
        if (sessionReduction.message) {
          appendMessage(sessionReduction.message);
        }
        setProgressItems((prev) =>
          reduceSessionRuntimeEvent(prev, eventType, payload, makeReducerId).progressItems,
        );
        continue;
      }
      const modelEstimateSnapshot = {
        downloadedByStreamId: streamDownloadEstimateRef.current,
        reasoningByStreamId: streamReasoningEstimateRef.current,
      };
      const modelReduction = reduceModelRuntimeEvent(emptyTokenUsage, eventType, payload, modelEstimateSnapshot);
      if (modelReduction.handled) {
        if (eventType === "model.call_started") {
          terminalStreamClosedRef.current = false;
        }
        setTokenUsage((current) =>
          reduceModelRuntimeEvent(current, eventType, payload, modelEstimateSnapshot).tokenUsage,
        );
        streamDownloadEstimateRef.current = modelReduction.estimates.downloadedByStreamId;
        streamReasoningEstimateRef.current = modelReduction.estimates.reasoningByStreamId;
        if (modelReduction.progressItem) {
          setProgressItems((prev) =>
            trimProgressItems([
              ...prev,
              {
                id: makeReducerId("progress"),
                ...modelReduction.progressItem!,
              },
            ]),
          );
        }
        if (modelReduction.suppressNextCallCompletedSettle) {
          suppressNextCallCompletedSettleRef.current = true;
        }
        if (modelReduction.streamChunk) {
          const { preview, runtimeSanitized } = modelReduction.streamChunk;
          if (!terminalStreamClosedRef.current) {
            applyStreamChunk(
              runtimeSanitized
                ? trimLeadingBlankLines(repairPseudoMarkdownNewlines(preview))
                : sanitizeStreamChunk(preview),
            );
          }
        }
        if (modelReduction.scheduleStreamCommit) {
          if (suppressNextCallCompletedSettleRef.current) {
            suppressNextCallCompletedSettleRef.current = false;
            continue;
          }
          if (callCompletedSettleTimerRef.current) {
            clearTimeout(callCompletedSettleTimerRef.current);
          }
          callCompletedSettleTimerRef.current = setTimeout(() => {
            callCompletedSettleTimerRef.current = null;
            commitStreamingMessage("");
          }, 220);
        }
        if (modelReduction.commitStreamingMessage !== undefined) {
          if (callCompletedSettleTimerRef.current) {
            clearTimeout(callCompletedSettleTimerRef.current);
            callCompletedSettleTimerRef.current = null;
          }
          commitStreamingMessage(repairPseudoMarkdownNewlines(modelReduction.commitStreamingMessage));
        }
        continue;
      }
      const toolReduction = reduceToolRuntimeEvent([], eventType, payload, makeReducerId);
      if (toolReduction.handled) {
        if (toolReduction.discardActiveStreamingMessage) {
          discardActiveStreamingMessage();
        }
        if (toolReduction.settleActiveStreamingMessage) {
          commitStreamingMessage("");
        }
        if (toolReduction.message) {
          appendMessage(toolReduction.message);
        }
        if (toolReduction.recoverableToolFailure) {
          const { toolCallId, toolId } = toolReduction.recoverableToolFailure;
          setMessages((prev) => markRecoverableToolFailureRecorded(prev, toolCallId, toolId));
        }
        setProgressItems((prev) =>
          reduceToolRuntimeEvent(prev, eventType, payload, makeReducerId).progressItems,
        );
        continue;
      }
      if (eventType === "permission.requested") {
        const reduction = reducePermissionRuntimeEvent(pendingPermissions, eventType, payload);
        setPendingPermissions((prev) => reducePermissionRuntimeEvent(prev, eventType, payload).pendingPermissions);
        if (reduction.clearDecisionForPermissionId) {
          setPermissionDecisionInFlight((current) =>
            current === reduction.clearDecisionForPermissionId ? null : current,
          );
          setPermissionDecisionErrors((prev) => {
            const next = { ...prev };
            delete next[reduction.clearDecisionForPermissionId!];
            return next;
          });
        }
        if (reduction.completedPermissionId) {
          setProgressItems((prev) => completePermissionProgress(prev, reduction.completedPermissionId!, "done"));
        }
        if (reduction.message) {
          appendMessage({
            role: "system",
            type: "approval",
            content: reduction.message,
          });
        }
        continue;
      }
      if (eventType === "permission.context" || eventType === "permission.decided" || eventType === "permission.suggestion_submitted") {
        const reduction = reducePermissionRuntimeEvent(pendingPermissions, eventType, payload);
        setPendingPermissions((prev) => reducePermissionRuntimeEvent(prev, eventType, payload).pendingPermissions);
        if (reduction.clearDecisionForPermissionId) {
          setPermissionDecisionInFlight((current) =>
            current === reduction.clearDecisionForPermissionId ? null : current,
          );
          setPermissionDecisionErrors((prev) => {
            const next = { ...prev };
            delete next[reduction.clearDecisionForPermissionId!];
            return next;
          });
        }
        if (reduction.completedPermissionId) {
          setProgressItems((prev) => completePermissionProgress(prev, reduction.completedPermissionId!, "done"));
        }
        if (reduction.message) {
          appendMessage({
            role: "system",
            type: "approval",
            content: reduction.message,
          });
        }
        continue;
      }
      if (
        eventType === "plan.mode_entered" ||
        eventType === "plan.approval_requested" ||
        eventType === "plan.approval_decided" ||
        eventType === "plan.mode_exited"
      ) {
        const reduction = reducePlanApprovalRuntimeEvent(
          pendingPlanApprovals,
          eventType,
          payload,
          planPreviewByApprovalIdRef.current,
        );
        planPreviewByApprovalIdRef.current = reduction.planPreviewCache;
        setPendingPlanApprovals((prev) => {
          const nextReduction = reducePlanApprovalRuntimeEvent(
            prev,
            eventType,
            payload,
            planPreviewByApprovalIdRef.current,
          );
          planPreviewByApprovalIdRef.current = nextReduction.planPreviewCache;
          return nextReduction.pendingPlanApprovals;
        });
        if (reduction.clearDecisionForPlanApprovalId) {
          setPlanDecisionInFlight((current) =>
            current === reduction.clearDecisionForPlanApprovalId ? null : current,
          );
          setPlanDecisionErrors((prev) => {
            const next = { ...prev };
            delete next[reduction.clearDecisionForPlanApprovalId!];
            return next;
          });
        }
        if (reduction.message) {
          appendMessage({
            role: "system",
            type: "approval",
            content: reduction.message,
          });
        }
        continue;
      }
      const observabilityReduction = reduceObservabilityRuntimeEvent([], eventType, payload, makeReducerId);
      if (observabilityReduction.handled) {
        if (observabilityReduction.clearDecisionForPermissionId) {
          const permissionId = observabilityReduction.clearDecisionForPermissionId;
          setPendingPermissions((prev) => prev.filter((item) => item.permission_id !== permissionId));
          setPermissionDecisionInFlight((current) => (current === permissionId ? null : current));
          setPermissionDecisionErrors((prev) => {
            const next = { ...prev };
            delete next[permissionId];
            return next;
          });
        }
        if (observabilityReduction.message) {
          appendMessage(observabilityReduction.message);
        }
        setProgressItems((prev) =>
          reduceObservabilityRuntimeEvent(prev, eventType, payload, makeReducerId).progressItems,
        );
        continue;
      }
      const errorReduction = reduceRuntimeErrorEvent(eventType, payload);
      if (errorReduction.handled) {
        if (errorReduction.runStatus) {
          setRunStatus(errorReduction.runStatus);
        }
        if (errorReduction.stopStreaming) {
          terminalStreamClosedRef.current = true;
          if (callCompletedSettleTimerRef.current) {
            clearTimeout(callCompletedSettleTimerRef.current);
            callCompletedSettleTimerRef.current = null;
          }
          commitStreamingMessage("");
          setIsStreaming(false);
        }
        if (errorReduction.permissionError) {
          const { permissionId, message } = errorReduction.permissionError;
          setPermissionDecisionInFlight((current) => (current === permissionId ? null : current));
          setPermissionDecisionErrors((prev) => ({
            ...prev,
            [permissionId]: message,
          }));
        }
        if (errorReduction.message) {
          appendMessage(errorReduction.message);
        }
      }
    }
  }, [
    appendMessage,
    applyStreamChunk,
    callCompletedSettleTimerRef,
    discardActiveStreamingMessage,
    emptyTokenUsage,
    commitStreamingMessage,
    pendingPermissions,
    pendingPlanApprovals,
    planPreviewByApprovalIdRef,
    sanitizeStreamChunk,
    seenEventKeysRef,
    setAutonomyMode,
    setContextPressure,
    setIsStreaming,
    setMessages,
    setPendingPermissions,
    setPendingPlanApprovals,
    setPermissionDecisionErrors,
    setPermissionDecisionInFlight,
    setPlanDecisionErrors,
    setPlanDecisionInFlight,
    setProgressItems,
    setRecoveryStatus,
    setRunStatus,
    setTokenUsage,
    streamDownloadEstimateRef,
    streamReasoningEstimateRef,
  ]);
}
