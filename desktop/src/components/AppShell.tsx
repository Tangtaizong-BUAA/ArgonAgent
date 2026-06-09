import { useCallback, useEffect, useRef, useState } from "react";
import type { DeepSeekModelId } from "./BottomComposer";
import type { AutonomyModeId } from "./Topbar";
import { AppShellLayout } from "./AppShellLayout";
import { type ContextPressureState, type RecoveryStatusState } from "./RuntimeStatus";
import {
  EMPTY_CONTEXT_PRESSURE,
  EMPTY_TOKEN_USAGE,
  FALLBACK_SLASH_COMMANDS,
  FRONTEND_BUILD_MARK,
  PROJECT_PICK_STORAGE_KEY,
  makeId,
  normalizeDeepSeekModelId,
} from "./appShellConfig";
import { type StoredRunState } from "@/runtime/runStore";
import {
  normalizeAutonomyMode,
  runtimeSnapshotStateToRunStatus,
  runtimeStateToRunStatus,
} from "@/runtime/runtimeEventViewModel";
import { usePermissionFlow, type PendingPermissionState, type PendingPlanApprovalState } from "@/hooks/usePermissionFlow";
import { useApprovalActions } from "@/hooks/useApprovalActions";
import { useAppKeyboardShortcuts } from "@/hooks/useAppKeyboardShortcuts";
import { useRuntimeBootstrap } from "@/hooks/useRuntimeBootstrap";
import { useRunPersistence } from "@/hooks/useRunPersistence";
import { useRunPersistenceSnapshotInput } from "@/hooks/useRunPersistenceSnapshotInput";
import { useRuntimeEventSubscription } from "@/hooks/useRuntimeEventSubscription";
import { useTranscriptState } from "@/hooks/useTranscriptState";
import { useRunCollections } from "@/hooks/useRunCollections";
import { useRuntimeSessionActions } from "@/hooks/useRuntimeSessionActions";
import { useRuntimeSettingsActions } from "@/hooks/useRuntimeSettingsActions";
import { useRunSelectionActions } from "@/hooks/useRunSelectionActions";
import { useStreamingTranscript } from "@/hooks/useStreamingTranscript";
import { useRuntimeEventApplication } from "@/hooks/useRuntimeEventApplication";
import type { RuntimeBootstrap } from "@/types/runtime";
import type { ProgressItem, RunStatus, TokenUsageStats, TranscriptMessage } from "@/types";

interface AppConfig {
  provider: "deepseek" | "qwen";
  apiKey?: string;
  baseUrl?: string;
  modelId?: string;
}

interface AppShellProps {
  config: AppConfig;
  onLogout: () => void;
}

export function AppShell({ config, onLogout }: AppShellProps) {
  const [bootstrap, setBootstrap] = useState<RuntimeBootstrap | null>(null);
  const [runtimeError, setRuntimeError] = useState<string | null>(null);
  const [activeRunId, setActiveRunId] = useState(() => makeId("run"));
  const [runStore, setRunStore] = useState<Record<string, StoredRunState>>({});
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [workspaceRoot, setWorkspaceRoot] = useState(".");
  const [selectedSourceWorkspaceRoot, setSelectedSourceWorkspaceRoot] = useState(".");
  const [selectedProjectId, setSelectedProjectId] = useState("project_default");
  const [cursor, setCursor] = useState(0);
  const [runStatus, setRunStatus] = useState<RunStatus>("idle");
  const [isRightPanelOpen, setIsRightPanelOpen] = useState(true);
  const {
    messages,
    setMessages,
    inputValue,
    setInputValue,
    runTitle,
    setRunTitle,
    isStreaming,
    setIsStreaming,
    composerFocusNonce,
    setComposerFocusNonce,
  } = useTranscriptState();
  const [slashCommands, setSlashCommands] = useState<string[]>(FALLBACK_SLASH_COMMANDS);
  const [progressItems, setProgressItems] = useState<ProgressItem[]>([]);
  const [activeModelId, setActiveModelId] = useState<DeepSeekModelId>(
    normalizeDeepSeekModelId(config.modelId),
  );
  const [modelSwitchHint, setModelSwitchHint] = useState<string | null>(null);
  const [autonomyMode, setAutonomyMode] = useState<AutonomyModeId>("conservative");
  const {
    pendingPermissions,
    setPendingPermissions,
    permissionDecisionInFlight,
    setPermissionDecisionInFlight,
    permissionDecisionErrors,
    setPermissionDecisionErrors,
    pendingPlanApprovals,
    setPendingPlanApprovals,
    planDecisionInFlight,
    setPlanDecisionInFlight,
    planDecisionErrors,
    setPlanDecisionErrors,
  } = usePermissionFlow();
  const [tokenUsage, setTokenUsage] = useState<TokenUsageStats>(EMPTY_TOKEN_USAGE);
  const [contextPressure, setContextPressure] = useState<ContextPressureState>(EMPTY_CONTEXT_PRESSURE);
  const [recoveryStatus, setRecoveryStatus] = useState<RecoveryStatusState | null>(null);
  const cursorRef = useRef(0);
  const seenEventKeysRef = useRef<Set<string>>(new Set());
  const planPreviewByApprovalIdRef = useRef<Map<string, string>>(new Map());
  const streamDownloadEstimateRef = useRef<Map<string, number>>(new Map());
  const streamReasoningEstimateRef = useRef<Map<string, number>>(new Map());

  const {
    appendMessage,
    clearStreamingState,
    discardActiveStreamingMessage,
    applyStreamChunk,
    sanitizeStreamChunk,
    commitStreamingMessage,
    callCompletedSettleTimerRef,
    streamToolMarkupStateRef,
  } = useStreamingTranscript({
    setMessages,
    setIsStreaming,
  });

  const activeKeyboardPermission = pendingPermissions[0];

  useEffect(() => {
    if (config.provider === "deepseek") {
      setActiveModelId(normalizeDeepSeekModelId(config.modelId));
    }
  }, [config.modelId, config.provider]);

  useEffect(() => {
    if (!modelSwitchHint) {
      return;
    }
    const timer = setTimeout(() => {
      setModelSwitchHint(null);
    }, 3800);
    return () => clearTimeout(timer);
  }, [modelSwitchHint]);

  const applyRuntimeEvents = useRuntimeEventApplication({
    pendingPermissions,
    pendingPlanApprovals,
    emptyTokenUsage: EMPTY_TOKEN_USAGE,
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
  });

  const handleRuntimeSnapshotState = useCallback((state: string) => {
    setRunStatus((current) => runtimeSnapshotStateToRunStatus(state, current));
  }, []);

  const handleRuntimeEventError = useCallback((message: string) => {
    setRuntimeError(message);
  }, []);

  const {
    clearRuntimeSubscription,
    ensureRuntimeSubscription,
    pollRuntimeEvents,
  } = useRuntimeEventSubscription({
    bootstrap,
    sessionId,
    cursorRef,
    onCursorChange: setCursor,
    onEvents: applyRuntimeEvents,
    onSnapshotState: handleRuntimeSnapshotState,
    onError: handleRuntimeEventError,
  });

  const handleSessionStarted = useCallback((session: {
    session_id: string;
    workspace_root: string;
    autonomy_mode: string;
    state: string;
  }) => {
    setWorkspaceRoot(session.workspace_root);
    setSessionId(session.session_id);
    setActiveRunId(session.session_id);
    setAutonomyMode(normalizeAutonomyMode(session.autonomy_mode));
    setRunStatus(runtimeStateToRunStatus(session.state));
    cursorRef.current = 0;
    seenEventKeysRef.current.clear();
    streamDownloadEstimateRef.current.clear();
    streamReasoningEstimateRef.current.clear();
    streamToolMarkupStateRef.current.insideMarkup = false;
    streamToolMarkupStateRef.current.carry = "";
    setTokenUsage(EMPTY_TOKEN_USAGE);
    setContextPressure(EMPTY_CONTEXT_PRESSURE);
    setRecoveryStatus(null);
    setCursor(0);
  }, []);

  const {
    handleSubmit,
    handleStopRun,
    handleContinueRun,
    handleRetryLast,
    handleExportEvents,
    handleOpenArtifact,
  } = useRuntimeSessionActions({
    autonomyMode,
    bootstrap,
    config,
    inputValue,
    messages,
    runTitle,
    selectedSourceWorkspaceRoot,
    sessionId,
    appendMessage,
    ensureRuntimeSubscription,
    onSessionStarted: handleSessionStarted,
    pollRuntimeEvents,
    setInputValue,
    setIsStreaming,
    setRunStatus,
    setRunTitle,
    setRuntimeError,
  });

  const {
    handleNewRun,
    handleSelectRun,
    handleSelectProject,
    handlePickProjectFolder,
  } = useRunSelectionActions({
    projectPickStorageKey: PROJECT_PICK_STORAGE_KEY,
    runStore,
    selectedSourceWorkspaceRoot,
    cursorRef,
    seenEventKeysRef,
    streamDownloadEstimateRef,
    streamReasoningEstimateRef,
    streamToolMarkupStateRef,
    planPreviewByApprovalIdRef,
    emptyTokenUsage: EMPTY_TOKEN_USAGE,
    emptyContextPressure: EMPTY_CONTEXT_PRESSURE,
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
  });

  const {
    handleModelChange,
    handleAutonomyModeChange,
  } = useRuntimeSettingsActions({
    activeModelId,
    bootstrap,
    config,
    runStatus,
    sessionId,
    appendMessage,
    pollRuntimeEvents,
    setActiveModelId,
    setAutonomyMode,
    setModelSwitchHint,
    setRuntimeError,
  });

  const {
    handlePermissionDecision,
    handleApproveAllPermissions,
    handlePlanDecision,
    handleApproveAllPlans,
  } = useApprovalActions({
    bootstrap,
    sessionId,
    pendingPermissions,
    pendingPlanApprovals,
    appendMessage,
    pollRuntimeEvents,
    setRuntimeError,
    setPermissionDecisionInFlight,
    setPermissionDecisionErrors,
    setPendingPlanApprovals,
    setPlanDecisionInFlight,
    setPlanDecisionErrors,
  });

  const runPersistenceSnapshotInput = useRunPersistenceSnapshotInput({
    activeModelId,
    activeRunId,
    cursor,
    messages,
    pendingPermissions,
    pendingPlanApprovals,
    progressItems,
    provider: config.provider,
    runStatus,
    runTitle,
    selectedSourceWorkspaceRoot,
    sessionId,
    workspaceRoot,
  });

  useRunPersistence({
    snapshotInput: runPersistenceSnapshotInput,
    runStore,
    setRunStore,
    setSelectedProjectId,
    setSelectedSourceWorkspaceRoot,
  });

  useRuntimeBootstrap({
    buildMark: FRONTEND_BUILD_MARK,
    fallbackCommands: FALLBACK_SLASH_COMMANDS,
    projectPickStorageKey: PROJECT_PICK_STORAGE_KEY,
    setBootstrap,
    setRuntimeError,
    setSelectedProjectId,
    setSelectedSourceWorkspaceRoot,
    setSlashCommands,
    setWorkspaceRoot,
  });

  useAppKeyboardShortcuts({
    activePermission: activeKeyboardPermission,
    permissionDecisionInFlight,
    runStatus,
    onPermissionDecision: (permissionId, decision) => {
      void handlePermissionDecision(permissionId, decision);
    },
    onStopRun: () => {
      void handleStopRun();
    },
  });

  const {
    currentRun,
    runs,
    projects,
    project,
    branchDetail,
  } = useRunCollections({
    activeModelId,
    activeRunId,
    configProvider: config.provider,
    cursor,
    messages,
    pendingPermissions,
    pendingPlanApprovals,
    progressItems,
    runStore,
    runStatus,
    runTitle,
    runtimeError,
    selectedProjectId,
    selectedSourceWorkspaceRoot,
    sessionId,
    workspaceRoot,
  });

  const hasConversation = Boolean(sessionId) || messages.length > 0 || runStatus !== "idle";
  const composerLayoutMode: "center" | "bottom" = hasConversation ? "bottom" : "center";

  useEffect(() => {
    window.__ARGON_GUI_DEBUG__ = {
      cursor,
      progress_count: progressItems.length,
      message_count: messages.length,
      run_status: runStatus,
      session_id: sessionId,
    };
  }, [cursor, messages.length, progressItems.length, runStatus, sessionId]);

  return (
    <AppShellLayout
      activeModelId={activeModelId}
      autonomyMode={autonomyMode}
      bootstrapReady={Boolean(bootstrap)}
      branchDetail={branchDetail}
      composerFocusNonce={composerFocusNonce}
      composerLayoutMode={composerLayoutMode}
      commandSuggestions={slashCommands}
      configProvider={config.provider}
      contextPressure={contextPressure}
      currentRun={currentRun}
      hasConversation={hasConversation}
      inputValue={inputValue}
      isRightPanelOpen={isRightPanelOpen}
      isStreaming={isStreaming}
      messages={messages}
      modelSwitchHint={modelSwitchHint}
      onApproveAllPermissions={() => void handleApproveAllPermissions()}
      onApproveAllPlans={() => void handleApproveAllPlans()}
      onAutonomyModeChange={(mode) => void handleAutonomyModeChange(mode)}
      onContinue={() => void handleContinueRun()}
      onDecisionPermission={(permissionId, decision, feedback) =>
        void handlePermissionDecision(permissionId, decision, feedback)
      }
      onDecisionPlan={(planApprovalId, decision, feedback) =>
        void handlePlanDecision(planApprovalId, decision, feedback)
      }
      onExport={() => void handleExportEvents()}
      onInputChange={setInputValue}
      onInsertCommand={(command) => {
        const next = command.endsWith(" ") ? command : `${command} `;
        setInputValue(next);
      }}
      onLogout={onLogout}
      onModelChange={(modelId) => void handleModelChange(modelId)}
      onNewRun={handleNewRun}
      onOpenArtifact={(path) => void handleOpenArtifact(path)}
      onPickProjectFolder={() => void handlePickProjectFolder()}
      onRetry={() => void handleRetryLast()}
      onSelectProject={handleSelectProject}
      onSelectRun={handleSelectRun}
      onSelectSuggestedTask={(task) => {
        setInputValue(task);
        setComposerFocusNonce((value) => value + 1);
      }}
      onStop={() => void handleStopRun()}
      onSubmit={() => void handleSubmit()}
      onToggleRightPanel={() => setIsRightPanelOpen((value) => !value)}
      pendingPermissions={pendingPermissions}
      pendingPlanApprovals={pendingPlanApprovals}
      permissionDecisionErrors={permissionDecisionErrors}
      permissionDecisionInFlight={permissionDecisionInFlight}
      planDecisionErrors={planDecisionErrors}
      planDecisionInFlight={planDecisionInFlight}
      progressItems={progressItems}
      project={project}
      projects={projects}
      recoveryStatus={recoveryStatus}
      runStatus={runStatus}
      runtimeError={runtimeError}
      selectedSourceWorkspaceRoot={selectedSourceWorkspaceRoot}
      tokenUsage={tokenUsage}
    />
  );
}
