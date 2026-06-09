import { useCallback, type Dispatch, type SetStateAction } from "react";
import {
  submitRuntimePermissionDecision,
  submitRuntimePlanDecision,
} from "@/runtime/localRuntimeClient";
import type { RuntimeBootstrap } from "@/types/runtime";
import type { TranscriptMessage } from "@/types";
import type { PendingPermissionState, PendingPlanApprovalState } from "./usePermissionFlow";

type PermissionDecision = "allow_once" | "allow_session" | "allow_project_rule" | "deny" | "modify";
type PlanDecision = "approve" | "request_revision";
type AppendMessage = (message: Omit<TranscriptMessage, "id" | "timestamp">) => void;

interface UseApprovalActionsOptions {
  bootstrap: RuntimeBootstrap | null;
  sessionId: string | null;
  pendingPermissions: PendingPermissionState[];
  pendingPlanApprovals: PendingPlanApprovalState[];
  appendMessage: AppendMessage;
  pollRuntimeEvents: (sessionId: string) => Promise<void>;
  setRuntimeError: Dispatch<SetStateAction<string | null>>;
  setPermissionDecisionInFlight: Dispatch<SetStateAction<string | null>>;
  setPermissionDecisionErrors: Dispatch<SetStateAction<Record<string, string>>>;
  setPendingPlanApprovals: Dispatch<SetStateAction<PendingPlanApprovalState[]>>;
  setPlanDecisionInFlight: Dispatch<SetStateAction<string | null>>;
  setPlanDecisionErrors: Dispatch<SetStateAction<Record<string, string>>>;
}

function approvalUnavailableReason(bootstrap: RuntimeBootstrap | null, sessionId: string | null): string | null {
  if (!bootstrap) {
    return "运行时桥接尚未就绪，无法提交审批";
  }
  if (!sessionId) {
    return "当前会话尚未就绪，无法提交审批";
  }
  return null;
}

export function useApprovalActions({
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
}: UseApprovalActionsOptions) {
  const handlePermissionDecision = useCallback(async (
    permissionId: string,
    decision: PermissionDecision,
    feedback?: string,
  ) => {
    const unavailableReason = approvalUnavailableReason(bootstrap, sessionId);
    if (unavailableReason || !bootstrap || !sessionId) {
      const reason = unavailableReason ?? "当前会话尚未就绪，无法提交审批";
      setRuntimeError(reason);
      appendMessage({
        role: "system",
        type: "approval",
        content: reason,
      });
      return;
    }
    setRuntimeError(null);
    setPermissionDecisionErrors((prev) => {
      const next = { ...prev };
      delete next[permissionId];
      return next;
    });
    setPermissionDecisionInFlight(permissionId);
    appendMessage({
      role: "system",
      type: "approval",
      content: `正在提交权限审批: ${permissionId}`,
    });
    try {
      const result = await submitRuntimePermissionDecision(bootstrap, {
        session_id: sessionId,
        permission_id: permissionId,
        decision,
        feedback,
      });
      if (!result.ok) {
        throw new Error(result.error_code ?? "runtime_permission_decision_failed");
      }
      setPermissionDecisionErrors((prev) => {
        const next = { ...prev };
        delete next[permissionId];
        return next;
      });
      appendMessage({
        role: "system",
        type: "approval",
        content:
          decision === "modify"
            ? `已带建议退回权限请求: ${permissionId}${feedback?.trim() ? `\n${feedback.trim()}` : ""}`
            : `权限审批已提交，等待 runtime 恢复工具: ${permissionId}`,
      });
      await pollRuntimeEvents(sessionId);
    } catch (error) {
      const message = String((error as Error).message ?? error);
      setRuntimeError(message);
      setPermissionDecisionErrors((prev) => ({ ...prev, [permissionId]: message }));
      setPermissionDecisionInFlight((current) => (current === permissionId ? null : current));
      appendMessage({
        role: "system",
        type: "approval",
        content: `权限审批提交失败: ${message}`,
      });
    }
  }, [
    appendMessage,
    bootstrap,
    pollRuntimeEvents,
    sessionId,
    setPermissionDecisionErrors,
    setPermissionDecisionInFlight,
    setRuntimeError,
  ]);

  const handleApproveAllPermissions = useCallback(async () => {
    const unavailableReason = approvalUnavailableReason(bootstrap, sessionId);
    if (unavailableReason || !bootstrap || !sessionId || pendingPermissions.length === 0) {
      if (unavailableReason) {
        setRuntimeError(unavailableReason);
      }
      return;
    }
    let activePermissionId: string | null = null;
    try {
      for (const permission of pendingPermissions) {
        activePermissionId = permission.permission_id;
        setPermissionDecisionErrors((prev) => {
          const next = { ...prev };
          delete next[permission.permission_id];
          return next;
        });
        setPermissionDecisionInFlight(permission.permission_id);
        const result = await submitRuntimePermissionDecision(bootstrap, {
          session_id: sessionId,
          permission_id: permission.permission_id,
          decision: "allow_session",
        });
        if (!result.ok) {
          throw new Error(result.error_code ?? "runtime_permission_decision_failed");
        }
        setPermissionDecisionErrors((prev) => {
          const next = { ...prev };
          delete next[permission.permission_id];
          return next;
        });
      }
      await pollRuntimeEvents(sessionId);
    } catch (error) {
      const message = String((error as Error).message ?? error);
      setRuntimeError(message);
      const failedPermissionId = activePermissionId;
      if (failedPermissionId) {
        setPermissionDecisionErrors((prev) => ({ ...prev, [failedPermissionId]: message }));
        setPermissionDecisionInFlight((current) => (current === failedPermissionId ? null : current));
      }
    }
  }, [
    bootstrap,
    pendingPermissions,
    pollRuntimeEvents,
    sessionId,
    setPermissionDecisionErrors,
    setPermissionDecisionInFlight,
    setRuntimeError,
  ]);

  const handlePlanDecision = useCallback(async (
    planApprovalId: string,
    decision: PlanDecision,
    feedback?: string,
  ) => {
    if (!bootstrap || !sessionId) {
      return;
    }
    setPlanDecisionInFlight(planApprovalId);
    setPlanDecisionErrors((prev) => {
      const next = { ...prev };
      delete next[planApprovalId];
      return next;
    });
    try {
      const result = await submitRuntimePlanDecision(bootstrap, {
        session_id: sessionId,
        plan_approval_id: planApprovalId,
        decision,
        feedback: decision === "approve" ? "" : (feedback?.trim() || "请根据用户反馈修订计划。"),
      });
      if (!result.ok) {
        throw new Error(result.error_code ?? "runtime_plan_decision_failed");
      }
      setPendingPlanApprovals((prev) => prev.filter((plan) => plan.plan_approval_id !== planApprovalId));
      setPlanDecisionInFlight((current) => (current === planApprovalId ? null : current));
      setPlanDecisionErrors((prev) => {
        const next = { ...prev };
        delete next[planApprovalId];
        return next;
      });
      await pollRuntimeEvents(sessionId);
    } catch (error) {
      const message = String((error as Error).message ?? error);
      setRuntimeError(message);
      setPlanDecisionErrors((prev) => ({ ...prev, [planApprovalId]: message }));
      setPlanDecisionInFlight((current) => (current === planApprovalId ? null : current));
      await pollRuntimeEvents(sessionId).catch(() => {});
    }
  }, [
    bootstrap,
    pollRuntimeEvents,
    sessionId,
    setPendingPlanApprovals,
    setPlanDecisionErrors,
    setPlanDecisionInFlight,
    setRuntimeError,
  ]);

  const handleApproveAllPlans = useCallback(async () => {
    if (!bootstrap || !sessionId || pendingPlanApprovals.length === 0) {
      return;
    }
    let activePlanId: string | null = null;
    try {
      for (const plan of pendingPlanApprovals) {
        activePlanId = plan.plan_approval_id;
        setPlanDecisionInFlight(plan.plan_approval_id);
        const result = await submitRuntimePlanDecision(bootstrap, {
          session_id: sessionId,
          plan_approval_id: plan.plan_approval_id,
          decision: "approve",
          feedback: "",
        });
        if (!result.ok) {
          throw new Error(result.error_code ?? "runtime_plan_decision_failed");
        }
        setPendingPlanApprovals((prev) =>
          prev.filter((pendingPlan) => pendingPlan.plan_approval_id !== plan.plan_approval_id),
        );
      }
      setPlanDecisionInFlight(null);
      await pollRuntimeEvents(sessionId);
    } catch (error) {
      const message = String((error as Error).message ?? error);
      setRuntimeError(message);
      const failedPlanId = activePlanId;
      if (failedPlanId) {
        setPlanDecisionErrors((prev) => ({ ...prev, [failedPlanId]: message }));
        setPlanDecisionInFlight((current) => (current === failedPlanId ? null : current));
      }
      await pollRuntimeEvents(sessionId).catch(() => {});
    }
  }, [
    bootstrap,
    pendingPlanApprovals,
    pollRuntimeEvents,
    sessionId,
    setPendingPlanApprovals,
    setPlanDecisionErrors,
    setPlanDecisionInFlight,
    setRuntimeError,
  ]);

  return {
    handlePermissionDecision,
    handleApproveAllPermissions,
    handlePlanDecision,
    handleApproveAllPlans,
  };
}
