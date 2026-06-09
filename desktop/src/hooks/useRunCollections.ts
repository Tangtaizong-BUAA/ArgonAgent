import { useMemo } from "react";
import {
  buildAgentRun,
  buildProjectsFromRuns,
  extractProjectName,
  normalizeProjectPath,
  stablePathProjectId,
  storedRunsWithCurrent,
  type StoredRunState,
} from "@/runtime/runStore";
import type { AgentRun, BranchDetail, ProgressItem, Project, RunStatus, TranscriptMessage } from "@/types";
import type { PendingPermissionState, PendingPlanApprovalState } from "./usePermissionFlow";

interface UseRunCollectionsOptions {
  activeModelId: string;
  activeRunId: string;
  configProvider: "deepseek" | "qwen";
  cursor: number;
  messages: TranscriptMessage[];
  pendingPermissions: PendingPermissionState[];
  pendingPlanApprovals: PendingPlanApprovalState[];
  progressItems: ProgressItem[];
  runStore: Record<string, StoredRunState>;
  runStatus: RunStatus;
  runTitle: string;
  runtimeError: string | null;
  selectedProjectId: string;
  selectedSourceWorkspaceRoot: string;
  sessionId: string | null;
  workspaceRoot: string;
}

export function useRunCollections({
  activeModelId,
  activeRunId,
  configProvider,
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
}: UseRunCollectionsOptions) {
  const currentRun = useMemo<AgentRun | null>(() => {
    return buildAgentRun({
      activeModelId,
      activeRunId,
      messages,
      progressItems,
      provider: configProvider,
      runStatus,
      runTitle,
      selectedSourceWorkspaceRoot,
      sessionId,
    });
  }, [
    activeModelId,
    activeRunId,
    configProvider,
    messages,
    progressItems,
    runStatus,
    runTitle,
    selectedSourceWorkspaceRoot,
    sessionId,
  ]);

  const runs = useMemo(() => {
    return storedRunsWithCurrent(runStore, currentRun);
  }, [currentRun, runStore]);

  const projects = useMemo<Project[]>(() => {
    return buildProjectsFromRuns({
      runStore,
      runs,
      selectedSourceWorkspaceRoot,
      workspaceRoot,
    });
  }, [runStore, runs, selectedSourceWorkspaceRoot, workspaceRoot]);

  const project = useMemo<Project>(() => {
    const selected =
      projects.find((item) => item.id === selectedProjectId) ??
      projects.find((item) => normalizeProjectPath(item.path) === normalizeProjectPath(selectedSourceWorkspaceRoot));
    return (
      selected ??
      projects[0] ?? {
        id: stablePathProjectId(selectedSourceWorkspaceRoot || workspaceRoot),
        name: extractProjectName(selectedSourceWorkspaceRoot || workspaceRoot),
        path: selectedSourceWorkspaceRoot || workspaceRoot,
        branch: "runtime/new",
        runs: [],
      }
    );
  }, [projects, selectedProjectId, selectedSourceWorkspaceRoot, workspaceRoot]);

  const branchDetail = useMemo<BranchDetail>(() => {
    return {
      name: currentRun?.branch ?? "runtime/new",
      checkpoint: runtimeError ? runtimeError : sessionId ? `cursor=${cursor}` : "runtime ready",
      hasChanges: pendingPermissions.length > 0 || pendingPlanApprovals.length > 0,
    };
  }, [cursor, currentRun?.branch, pendingPermissions.length, pendingPlanApprovals.length, runtimeError, sessionId]);

  return {
    currentRun,
    runs,
    projects,
    project,
    branchDetail,
  };
}
