import { useCallback, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import { pickRuntimeProjectFolder } from "@/runtime/localRuntimeClient";
import { normalizeStoredProgressItems } from "@/runtime/progressLedger";
import {
  normalizeProjectPath,
  stablePathProjectId,
  type StoredRunState,
} from "@/runtime/runStore";
import type { AgentRun, ProgressItem, Project, RunStatus, TokenUsageStats } from "@/types";
import type { DeepSeekModelId } from "@/components/BottomComposer";
import type { ContextPressureState, RecoveryStatusState } from "@/components/RuntimeStatus";
import type { PendingPermissionState, PendingPlanApprovalState } from "./usePermissionFlow";

interface UseRunSelectionActionsOptions {
  projectPickStorageKey: string;
  runStore: Record<string, StoredRunState>;
  selectedSourceWorkspaceRoot: string;
  cursorRef: MutableRefObject<number>;
  seenEventKeysRef: MutableRefObject<Set<string>>;
  streamDownloadEstimateRef: MutableRefObject<Map<string, number>>;
  streamReasoningEstimateRef: MutableRefObject<Map<string, number>>;
  streamToolMarkupStateRef: MutableRefObject<{ insideMarkup: boolean; carry: string }>;
  planPreviewByApprovalIdRef: MutableRefObject<Map<string, string>>;
  emptyTokenUsage: TokenUsageStats;
  emptyContextPressure: ContextPressureState;
  clearRuntimeSubscription: () => void;
  clearStreamingState: () => void;
  setActiveModelId: Dispatch<SetStateAction<DeepSeekModelId>>;
  setActiveRunId: Dispatch<SetStateAction<string>>;
  setContextPressure: Dispatch<SetStateAction<ContextPressureState>>;
  setCursor: Dispatch<SetStateAction<number>>;
  setInputValue: Dispatch<SetStateAction<string>>;
  setMessages: Dispatch<SetStateAction<import("@/types").TranscriptMessage[]>>;
  setModelSwitchHint: Dispatch<SetStateAction<string | null>>;
  setPendingPermissions: Dispatch<SetStateAction<PendingPermissionState[]>>;
  setPendingPlanApprovals: Dispatch<SetStateAction<PendingPlanApprovalState[]>>;
  setPlanDecisionErrors: Dispatch<SetStateAction<Record<string, string>>>;
  setPlanDecisionInFlight: Dispatch<SetStateAction<string | null>>;
  setProgressItems: Dispatch<SetStateAction<ProgressItem[]>>;
  setRecoveryStatus: Dispatch<SetStateAction<RecoveryStatusState | null>>;
  setRunStatus: Dispatch<SetStateAction<RunStatus>>;
  setRunTitle: Dispatch<SetStateAction<string>>;
  setSelectedProjectId: Dispatch<SetStateAction<string>>;
  setSelectedSourceWorkspaceRoot: Dispatch<SetStateAction<string>>;
  setSessionId: Dispatch<SetStateAction<string | null>>;
  setTokenUsage: Dispatch<SetStateAction<TokenUsageStats>>;
  setWorkspaceRoot: Dispatch<SetStateAction<string>>;
}

function makeRunId(): string {
  return `run_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
}

function restoreDeepSeekModelId(value: string): DeepSeekModelId | null {
  return value === "deepseek-v4-pro" || value === "deepseek-v4-flash" ? value : null;
}

export function useRunSelectionActions({
  projectPickStorageKey,
  runStore,
  selectedSourceWorkspaceRoot,
  cursorRef,
  seenEventKeysRef,
  streamDownloadEstimateRef,
  streamReasoningEstimateRef,
  streamToolMarkupStateRef,
  planPreviewByApprovalIdRef,
  emptyTokenUsage,
  emptyContextPressure,
  clearRuntimeSubscription,
  clearStreamingState,
  setActiveModelId,
  setActiveRunId,
  setContextPressure,
  setCursor,
  setInputValue,
  setMessages,
  setModelSwitchHint,
  setPendingPermissions,
  setPendingPlanApprovals,
  setPlanDecisionErrors,
  setPlanDecisionInFlight,
  setProgressItems,
  setRecoveryStatus,
  setRunStatus,
  setRunTitle,
  setSelectedProjectId,
  setSelectedSourceWorkspaceRoot,
  setSessionId,
  setTokenUsage,
  setWorkspaceRoot,
}: UseRunSelectionActionsOptions) {
  const resetStreamRefs = useCallback(() => {
    cursorRef.current = 0;
    seenEventKeysRef.current.clear();
    streamDownloadEstimateRef.current.clear();
    streamReasoningEstimateRef.current.clear();
    streamToolMarkupStateRef.current.insideMarkup = false;
    streamToolMarkupStateRef.current.carry = "";
  }, [
    cursorRef,
    seenEventKeysRef,
    streamDownloadEstimateRef,
    streamReasoningEstimateRef,
    streamToolMarkupStateRef,
  ]);

  const handleNewRun = useCallback((sourcePathOverride?: string) => {
    const targetSourcePath = normalizeProjectPath(sourcePathOverride || selectedSourceWorkspaceRoot);
    setActiveRunId(makeRunId());
    setSessionId(null);
    setRunTitle("新会话");
    setMessages([]);
    setInputValue("");
    setRunStatus("idle");
    setCursor(0);
    resetStreamRefs();
    setProgressItems([]);
    setTokenUsage(emptyTokenUsage);
    setContextPressure(emptyContextPressure);
    setRecoveryStatus(null);
    setPendingPermissions([]);
    setPendingPlanApprovals([]);
    setPlanDecisionInFlight(null);
    setPlanDecisionErrors({});
    setModelSwitchHint(null);
    if (targetSourcePath) {
      setSelectedSourceWorkspaceRoot(targetSourcePath);
      setWorkspaceRoot(targetSourcePath);
    }
    setSelectedProjectId(stablePathProjectId(targetSourcePath));
    planPreviewByApprovalIdRef.current.clear();
    clearStreamingState();
    clearRuntimeSubscription();
  }, [
    clearRuntimeSubscription,
    clearStreamingState,
    emptyContextPressure,
    emptyTokenUsage,
    planPreviewByApprovalIdRef,
    resetStreamRefs,
    selectedSourceWorkspaceRoot,
    setActiveRunId,
    setContextPressure,
    setCursor,
    setInputValue,
    setMessages,
    setModelSwitchHint,
    setPendingPermissions,
    setPendingPlanApprovals,
    setPlanDecisionErrors,
    setPlanDecisionInFlight,
    setProgressItems,
    setRecoveryStatus,
    setRunStatus,
    setRunTitle,
    setSelectedProjectId,
    setSelectedSourceWorkspaceRoot,
    setSessionId,
    setTokenUsage,
    setWorkspaceRoot,
  ]);

  const restoreStoredRun = useCallback((stored: StoredRunState) => {
    clearRuntimeSubscription();
    setActiveRunId(stored.run.id);
    setSessionId(stored.sessionId);
    setRunTitle(stored.run.title || "新会话");
    setRunStatus(stored.run.status);
    setMessages(stored.messages);
    setProgressItems(normalizeStoredProgressItems(stored.progressItems));
    setPendingPermissions(stored.pendingPermissions);
    setPendingPlanApprovals(stored.pendingPlanApprovals);
    setPlanDecisionInFlight(null);
    setPlanDecisionErrors({});
    setWorkspaceRoot(stored.workspaceRoot || ".");
    const sourceWorkspace = normalizeProjectPath(
      stored.sourceWorkspaceRoot || stored.workspaceRoot || ".",
    );
    setSelectedSourceWorkspaceRoot(sourceWorkspace);
    setSelectedProjectId(stablePathProjectId(sourceWorkspace));
    setCursor(stored.cursor);
    cursorRef.current = stored.cursor;
    const restoredModel = restoreDeepSeekModelId(stored.run.model);
    if (restoredModel) {
      setActiveModelId(restoredModel);
    }
    seenEventKeysRef.current.clear();
    planPreviewByApprovalIdRef.current.clear();
    streamToolMarkupStateRef.current.insideMarkup = false;
    streamToolMarkupStateRef.current.carry = "";
    clearStreamingState();
    setInputValue("");
    setModelSwitchHint(null);
    setContextPressure(emptyContextPressure);
    setRecoveryStatus(null);
  }, [
    clearRuntimeSubscription,
    clearStreamingState,
    cursorRef,
    emptyContextPressure,
    planPreviewByApprovalIdRef,
    seenEventKeysRef,
    setActiveModelId,
    setActiveRunId,
    setContextPressure,
    setCursor,
    setInputValue,
    setMessages,
    setModelSwitchHint,
    setPendingPermissions,
    setPendingPlanApprovals,
    setPlanDecisionErrors,
    setPlanDecisionInFlight,
    setProgressItems,
    setRecoveryStatus,
    setRunStatus,
    setRunTitle,
    setSelectedProjectId,
    setSelectedSourceWorkspaceRoot,
    setSessionId,
    setWorkspaceRoot,
    streamToolMarkupStateRef,
  ]);

  const handleSelectRun = useCallback((run: AgentRun) => {
    const stored = runStore[run.id];
    if (!stored) {
      return;
    }
    restoreStoredRun(stored);
  }, [restoreStoredRun, runStore]);

  const handleSelectProject = useCallback((project: Project) => {
    const normalizedProjectPath = normalizeProjectPath(project.path);
    setSelectedProjectId(project.id);
    setSelectedSourceWorkspaceRoot(normalizedProjectPath);
    const firstRun = project.runs[0];
    if (firstRun) {
      const stored = runStore[firstRun.id];
      if (stored) {
        restoreStoredRun(stored);
      }
      return;
    }
    handleNewRun(normalizedProjectPath);
  }, [
    handleNewRun,
    restoreStoredRun,
    runStore,
    setSelectedProjectId,
    setSelectedSourceWorkspaceRoot,
  ]);

  const handlePickProjectFolder = useCallback(async () => {
    const chosen = await pickRuntimeProjectFolder(selectedSourceWorkspaceRoot);
    if (!chosen) {
      return;
    }
    const normalizedChosen = normalizeProjectPath(chosen);
    setSelectedSourceWorkspaceRoot(normalizedChosen);
    setSelectedProjectId(stablePathProjectId(normalizedChosen));
    localStorage.setItem(projectPickStorageKey, normalizedChosen);
    handleNewRun(normalizedChosen);
  }, [
    handleNewRun,
    projectPickStorageKey,
    selectedSourceWorkspaceRoot,
    setSelectedProjectId,
    setSelectedSourceWorkspaceRoot,
  ]);

  return {
    handleNewRun,
    restoreStoredRun,
    handleSelectRun,
    handleSelectProject,
    handlePickProjectFolder,
  };
}
