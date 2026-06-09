import { useMemo } from "react";
import { makeStoredRunSnapshot } from "@/runtime/runStore";
import type { PendingPermissionState, PendingPlanApprovalState } from "@/hooks/usePermissionFlow";
import type { DeepSeekModelId } from "@/components/BottomComposer";
import type { ProgressItem, RunStatus, TranscriptMessage } from "@/types";

interface RunPersistenceSnapshotInputArgs {
  activeModelId: DeepSeekModelId;
  activeRunId: string;
  cursor: number;
  messages: TranscriptMessage[];
  pendingPermissions: PendingPermissionState[];
  pendingPlanApprovals: PendingPlanApprovalState[];
  progressItems: ProgressItem[];
  provider: "deepseek" | "qwen";
  runStatus: RunStatus;
  runTitle: string;
  selectedSourceWorkspaceRoot: string;
  sessionId: string | null;
  workspaceRoot: string;
}

type SnapshotInput = Parameters<typeof makeStoredRunSnapshot>[0];

export function useRunPersistenceSnapshotInput(args: RunPersistenceSnapshotInputArgs): SnapshotInput {
  return useMemo(() => ({
    activeModelId: args.activeModelId,
    activeRunId: args.activeRunId,
    cursor: args.cursor,
    messages: args.messages,
    pendingPermissions: args.pendingPermissions,
    pendingPlanApprovals: args.pendingPlanApprovals,
    progressItems: args.progressItems,
    provider: args.provider,
    runStatus: args.runStatus,
    runTitle: args.runTitle,
    selectedSourceWorkspaceRoot: args.selectedSourceWorkspaceRoot,
    sessionId: args.sessionId,
    workspaceRoot: args.workspaceRoot,
  }), [
    args.activeModelId,
    args.activeRunId,
    args.cursor,
    args.messages,
    args.pendingPermissions,
    args.pendingPlanApprovals,
    args.progressItems,
    args.provider,
    args.runStatus,
    args.runTitle,
    args.selectedSourceWorkspaceRoot,
    args.sessionId,
    args.workspaceRoot,
  ]);
}
