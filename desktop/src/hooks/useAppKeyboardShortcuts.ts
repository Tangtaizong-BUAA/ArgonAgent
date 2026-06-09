import { useEffect } from "react";
import type { PendingPermissionState } from "@/hooks/usePermissionFlow";
import type { RunStatus } from "@/types";

type KeyboardPermissionDecision = "allow_once" | "allow_session" | "deny";

interface UseAppKeyboardShortcutsArgs {
  activePermission: PendingPermissionState | undefined;
  permissionDecisionInFlight: string | null;
  runStatus: RunStatus;
  onPermissionDecision: (permissionId: string, decision: KeyboardPermissionDecision) => void;
  onStopRun: () => void;
}

export function useAppKeyboardShortcuts({
  activePermission,
  permissionDecisionInFlight,
  runStatus,
  onPermissionDecision,
  onStopRun,
}: UseAppKeyboardShortcutsArgs) {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (isEditableKeyboardTarget(event.target)) {
        return;
      }
      if (event.key === "Escape" && runStatus === "running") {
        event.preventDefault();
        onStopRun();
        return;
      }
      if (!activePermission || permissionDecisionInFlight) {
        return;
      }
      const key = event.key.toLowerCase();
      if (key === "y") {
        event.preventDefault();
        onPermissionDecision(activePermission.permission_id, "allow_once");
      } else if (key === "a") {
        event.preventDefault();
        onPermissionDecision(activePermission.permission_id, "allow_session");
      } else if (key === "n") {
        event.preventDefault();
        onPermissionDecision(activePermission.permission_id, "deny");
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activePermission, onPermissionDecision, onStopRun, permissionDecisionInFlight, runStatus]);
}

function isEditableKeyboardTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  const tagName = target.tagName.toLowerCase();
  return target.isContentEditable || tagName === "input" || tagName === "textarea" || tagName === "select";
}
