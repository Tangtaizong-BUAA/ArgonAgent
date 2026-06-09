import { useState } from "react";
import type { PendingPermission } from "@/runtime/localRuntimeClient";

export interface PendingPlanApprovalState {
  plan_approval_id: string;
  goal: string;
  plan_preview?: string;
}

export type PendingPermissionState = PendingPermission;

export function usePermissionFlow() {
  const [pendingPermissions, setPendingPermissions] = useState<PendingPermissionState[]>([]);
  const [permissionDecisionInFlight, setPermissionDecisionInFlight] = useState<string | null>(null);
  const [permissionDecisionErrors, setPermissionDecisionErrors] = useState<Record<string, string>>({});
  const [pendingPlanApprovals, setPendingPlanApprovals] = useState<PendingPlanApprovalState[]>([]);
  const [planDecisionInFlight, setPlanDecisionInFlight] = useState<string | null>(null);
  const [planDecisionErrors, setPlanDecisionErrors] = useState<Record<string, string>>({});

  return {
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
  };
}
