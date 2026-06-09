import { useEffect } from "react";
import type { Dispatch, SetStateAction } from "react";
import { writeRuntimeSessionRecord } from "@/runtime/localRuntimeClient";
import {
  makeStoredRunSnapshot,
  mergeStoredRun,
  normalizeProjectPath,
  nowIso,
  parseStoredRunStore,
  RUN_STORE_STORAGE_KEY,
  stablePathProjectId,
  type StoredRunState,
} from "@/runtime/runStore";

type SnapshotInput = Parameters<typeof makeStoredRunSnapshot>[0];

interface UseRunPersistenceArgs {
  snapshotInput: SnapshotInput;
  runStore: Record<string, StoredRunState>;
  setRunStore: Dispatch<SetStateAction<Record<string, StoredRunState>>>;
  setSelectedSourceWorkspaceRoot: Dispatch<SetStateAction<string>>;
  setSelectedProjectId: Dispatch<SetStateAction<string>>;
}

export function useRunPersistence({
  snapshotInput,
  runStore,
  setRunStore,
  setSelectedSourceWorkspaceRoot,
  setSelectedProjectId,
}: UseRunPersistenceArgs) {
  useEffect(() => {
    try {
      const { store, latest } = parseStoredRunStore(localStorage.getItem(RUN_STORE_STORAGE_KEY));
      if (!latest) {
        return;
      }
      setRunStore(store);
      const preferredPath = normalizeProjectPath(
        latest.sourceWorkspaceRoot.trim() || latest.workspaceRoot.trim() || "",
      );
      if (preferredPath) {
        setSelectedSourceWorkspaceRoot(preferredPath);
        setSelectedProjectId(stablePathProjectId(preferredPath));
      }
    } catch {
      // ignore malformed local snapshot
    }
  }, [setRunStore, setSelectedProjectId, setSelectedSourceWorkspaceRoot]);

  useEffect(() => {
    const snapshot = makeStoredRunSnapshot(snapshotInput);
    if (!snapshot) {
      return;
    }
    const timer = setTimeout(() => {
      setRunStore((prev) => mergeStoredRun(prev, snapshot));
    }, snapshotInput.runStatus === "running" ? 700 : 120);
    return () => clearTimeout(timer);
  }, [setRunStore, snapshotInput]);

  useEffect(() => {
    const timer = setTimeout(() => {
      try {
        localStorage.setItem(RUN_STORE_STORAGE_KEY, JSON.stringify(runStore));
      } catch {
        // ignore persistence error
      }
    }, snapshotInput.runStatus === "running" ? 900 : 180);
    return () => clearTimeout(timer);
  }, [runStore, snapshotInput.runStatus]);

  useEffect(() => {
    if (!snapshotInput.sessionId || !snapshotInput.workspaceRoot || snapshotInput.workspaceRoot === ".") {
      return;
    }
    const snapshot = makeStoredRunSnapshot(snapshotInput);
    if (!snapshot) {
      return;
    }
    const timer = setTimeout(() => {
      const payload = {
        run: snapshot.run,
        source_workspace_root: snapshot.sourceWorkspaceRoot,
        runtime_workspace_root: snapshot.workspaceRoot,
        cursor: snapshot.cursor,
        messages: snapshot.messages,
        progress_items: snapshot.progressItems,
        pending_permissions: snapshot.pendingPermissions,
        pending_plan_approvals: snapshot.pendingPlanApprovals,
        updated_at: nowIso(),
      };
      void writeRuntimeSessionRecord({
        workspaceRoot: snapshotInput.workspaceRoot,
        runId: snapshot.run.id,
        sessionId: snapshotInput.sessionId!,
        contentJson: JSON.stringify(payload),
      }).catch(() => {
        // ignore transient fs write errors for session mirror
      });
    }, 280);
    return () => clearTimeout(timer);
  }, [snapshotInput]);
}
