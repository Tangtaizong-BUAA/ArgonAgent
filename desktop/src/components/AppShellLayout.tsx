import { useMemo } from "react";
import { Sidebar } from "./Sidebar";
import { Topbar, type AutonomyModeId } from "./Topbar";
import { EmptyState } from "./EmptyState";
import { Transcript } from "./Transcript";
import { RightInspector } from "./RightInspector";
import { BottomComposer, type DeepSeekModelId } from "./BottomComposer";
import { PlanApprovalBanner, RunDashboardStrip, type ContextPressureState, type RecoveryStatusState } from "./RuntimeStatus";
import type { PendingPermissionState, PendingPlanApprovalState } from "@/hooks/usePermissionFlow";
import type { ProgressItem, Project, AgentRun, BranchDetail, RunStatus, TokenUsageStats } from "@/types";

type ProviderId = "deepseek" | "qwen";
type PermissionDecision = "allow_once" | "allow_session" | "allow_project_rule" | "deny" | "modify";
type PlanDecision = "approve" | "request_revision";

interface AppShellLayoutProps {
  activeModelId: DeepSeekModelId;
  autonomyMode: AutonomyModeId;
  bootstrapReady: boolean;
  branchDetail: BranchDetail;
  composerFocusNonce: number;
  composerLayoutMode: "center" | "bottom";
  commandSuggestions: string[];
  configProvider: ProviderId;
  contextPressure: ContextPressureState;
  currentRun: AgentRun | null;
  hasConversation: boolean;
  inputValue: string;
  isRightPanelOpen: boolean;
  isStreaming: boolean;
  messages: Parameters<typeof Transcript>[0]["messages"];
  modelSwitchHint: string | null;
  onApproveAllPermissions: () => void;
  onApproveAllPlans: () => void;
  onAutonomyModeChange: (mode: AutonomyModeId) => void;
  onContinue: () => void;
  onDecisionPermission: (permissionId: string, decision: PermissionDecision, feedback?: string) => void;
  onDecisionPlan: (planApprovalId: string, decision: PlanDecision, feedback?: string) => void;
  onExport: () => void;
  onInputChange: (value: string) => void;
  onInsertCommand: (command: string) => void;
  onLogout: () => void;
  onModelChange: (modelId: DeepSeekModelId) => void;
  onNewRun: () => void;
  onOpenArtifact: (path: string) => void;
  onPickProjectFolder: () => void;
  onRetry: () => void;
  onSelectProject: (project: Project) => void;
  onSelectRun: (run: AgentRun) => void;
  onSelectSuggestedTask: (task: string) => void;
  onStop: () => void;
  onSubmit: () => void;
  onToggleRightPanel: () => void;
  pendingPermissions: PendingPermissionState[];
  pendingPlanApprovals: PendingPlanApprovalState[];
  permissionDecisionErrors: Record<string, string>;
  permissionDecisionInFlight: string | null;
  planDecisionErrors: Record<string, string>;
  planDecisionInFlight: string | null;
  progressItems: ProgressItem[];
  project: Project;
  projects: Project[];
  recoveryStatus: RecoveryStatusState | null;
  runStatus: RunStatus;
  runtimeError: string | null;
  selectedSourceWorkspaceRoot: string;
  tokenUsage: TokenUsageStats;
}

export function AppShellLayout({
  activeModelId,
  autonomyMode,
  bootstrapReady,
  branchDetail,
  composerFocusNonce,
  composerLayoutMode,
  commandSuggestions,
  configProvider,
  contextPressure,
  currentRun,
  hasConversation,
  inputValue,
  isRightPanelOpen,
  isStreaming,
  messages,
  modelSwitchHint,
  onApproveAllPermissions,
  onApproveAllPlans,
  onAutonomyModeChange,
  onContinue,
  onDecisionPermission,
  onDecisionPlan,
  onExport,
  onInputChange,
  onInsertCommand,
  onLogout,
  onModelChange,
  onNewRun,
  onOpenArtifact,
  onPickProjectFolder,
  onRetry,
  onSelectProject,
  onSelectRun,
  onSelectSuggestedTask,
  onStop,
  onSubmit,
  onToggleRightPanel,
  pendingPermissions,
  pendingPlanApprovals,
  permissionDecisionErrors,
  permissionDecisionInFlight,
  planDecisionErrors,
  planDecisionInFlight,
  progressItems,
  project,
  projects,
  recoveryStatus,
  runStatus,
  runtimeError,
  selectedSourceWorkspaceRoot,
  tokenUsage,
}: AppShellLayoutProps) {
  const pendingPermission = pendingPermissions[0];
  const pendingPlanApproval = pendingPlanApprovals[0];
  const runDashboard = useMemo(() => {
    const running = progressItems.filter((item) => item.status === "running").length;
    const done = progressItems.filter((item) => item.status === "done").length;
    const failed = progressItems.filter((item) => item.status === "failed").length;
    const toolCount = progressItems.filter((item) => item.kind === "tool" || item.toolId).length;
    const pending = pendingPermissions.length + pendingPlanApprovals.length;
    const additions = currentRun?.changes.reduce((sum, change) => sum + change.additions, 0) ?? 0;
    const deletions = currentRun?.changes.reduce((sum, change) => sum + change.deletions, 0) ?? 0;
    const lastLabel =
      progressItems
        .slice()
        .reverse()
        .find((item) => item.status === "running" || item.status === "failed")?.label ??
      progressItems.at(-1)?.label ??
      "等待任务输入";
    return { running, done, failed, toolCount, pending, additions, deletions, lastLabel };
  }, [currentRun?.changes, pendingPermissions.length, pendingPlanApprovals.length, progressItems]);

  return (
    <div className="flex h-full w-full bg-bg-app text-text-primary select-none">
      <Sidebar
        projects={projects}
        selectedProject={project}
        currentRun={currentRun}
        onSelectProject={onSelectProject}
        onSelectRun={onSelectRun}
        onNewRun={onNewRun}
        onPickProjectFolder={onPickProjectFolder}
        onLogout={onLogout}
      />

      <div className="flex flex-1 flex-col min-w-0">
        <Topbar
          currentRun={currentRun}
          runStatus={runStatus}
          isRightPanelOpen={isRightPanelOpen}
          onToggleRightPanel={onToggleRightPanel}
          onContinue={onContinue}
          onRetry={onRetry}
          onExport={onExport}
          autonomyMode={autonomyMode}
          onAutonomyModeChange={onAutonomyModeChange}
        />

        <div className="flex flex-1 min-h-0 relative">
          <div className="flex flex-1 flex-col min-w-0 relative">
            <div className={`flex-1 overflow-y-auto min-h-0 ${hasConversation ? "pb-[180px]" : "pb-10"}`}>
              {hasConversation && currentRun ? (
                <>
                  <RunDashboardStrip
                    status={runStatus}
                    running={runDashboard.running}
                    done={runDashboard.done}
                    failed={runDashboard.failed}
                    toolCount={runDashboard.toolCount}
                    pending={runDashboard.pending}
                    additions={runDashboard.additions}
                    deletions={runDashboard.deletions}
                    tokenUsage={tokenUsage}
                    lastLabel={runDashboard.lastLabel}
                    runtimeError={runtimeError}
                    contextPressure={contextPressure}
                    recoveryStatus={recoveryStatus}
                  />
                  <Transcript
                    messages={messages}
                    currentRun={currentRun}
                    isStreaming={isStreaming && runStatus === "running"}
                  />
                </>
              ) : (
                <EmptyState
                  projectName={project.name}
                  projectPath={selectedSourceWorkspaceRoot}
                  reserveComposerSpace
                  onPickProjectFolder={onPickProjectFolder}
                  onSelectSuggestedTask={onSelectSuggestedTask}
                />
              )}
            </div>

            {pendingPermission && (
              <div
                role="dialog"
                aria-modal="false"
                aria-label="权限审批"
                className="absolute left-1/2 top-5 z-50 w-[min(520px,calc(100%-32px))] -translate-x-1/2 rounded-lg border border-accent/50 bg-bg-panel/95 shadow-xl backdrop-blur pointer-events-auto"
              >
                <div className="flex items-start justify-between gap-4 px-4 py-3">
                  <div className="min-w-0">
                    <div className="text-[12px] font-medium text-text-primary">
                      {permissionDecisionInFlight === pendingPermission.permission_id
                        ? "正在处理审批"
                        : "需要 Shell 命令审批"}
                    </div>
                    <div className="mt-1 truncate text-[11px] text-text-muted">
                      {pendingPermission.tool_id} · {pendingPermission.request_type} · {pendingPermission.permission_id}
                    </div>
                    {pendingPermission.args_preview ? (
                      <code
                        className="mt-2 block max-h-16 overflow-auto whitespace-pre-wrap break-words rounded-md border border-border-subtle/70 bg-bg-card/70 px-2 py-1.5 text-[11px] leading-5 text-text-secondary"
                        title={pendingPermission.args_preview}
                      >
                        {pendingPermission.args_preview}
                      </code>
                    ) : null}
                  </div>
                  <div className="grid shrink-0 grid-cols-2 gap-2">
                    <button
                      type="button"
                      aria-label="允许本次权限请求"
                      disabled={permissionDecisionInFlight === pendingPermission.permission_id}
                      onClick={() => onDecisionPermission(pendingPermission.permission_id, "allow_once")}
                      className="rounded-md bg-success/20 px-2.5 py-1.5 text-[11px] text-success transition-colors hover:bg-success/30 disabled:cursor-wait disabled:opacity-50"
                    >
                      {permissionDecisionInFlight === pendingPermission.permission_id ? "处理中" : "允许一次"}
                    </button>
                    <button
                      type="button"
                      aria-label="允许本会话中的同类权限请求"
                      disabled={permissionDecisionInFlight === pendingPermission.permission_id}
                      onClick={() => onDecisionPermission(pendingPermission.permission_id, "allow_session")}
                      className="rounded-md bg-accent/20 px-2.5 py-1.5 text-[11px] text-accent transition-colors hover:bg-accent/30 disabled:cursor-wait disabled:opacity-50"
                    >
                      允许本会话
                    </button>
                    <button
                      type="button"
                      aria-label="为项目保存权限规则"
                      disabled={permissionDecisionInFlight === pendingPermission.permission_id}
                      onClick={() => onDecisionPermission(pendingPermission.permission_id, "allow_project_rule")}
                      className="rounded-md bg-accent/20 px-2.5 py-1.5 text-[11px] text-accent transition-colors hover:bg-accent/30 disabled:cursor-wait disabled:opacity-50"
                    >
                      项目规则
                    </button>
                    <button
                      type="button"
                      aria-label="拒绝权限请求"
                      disabled={permissionDecisionInFlight === pendingPermission.permission_id}
                      onClick={() => onDecisionPermission(pendingPermission.permission_id, "deny")}
                      className="rounded-md bg-danger/20 px-2.5 py-1.5 text-[11px] text-danger transition-colors hover:bg-danger/30 disabled:cursor-wait disabled:opacity-50"
                    >
                      拒绝
                    </button>
                  </div>
                </div>
                {permissionDecisionErrors[pendingPermission.permission_id] && (
                  <div className="flex items-center justify-between gap-3 border-t border-danger/20 px-4 py-2 text-[11px] leading-5 text-danger">
                    <span className="min-w-0">{permissionDecisionErrors[pendingPermission.permission_id]}</span>
                    <button
                      type="button"
                      disabled={permissionDecisionInFlight === pendingPermission.permission_id}
                      onClick={() => onDecisionPermission(pendingPermission.permission_id, "allow_once")}
                      className="shrink-0 rounded-md border border-danger/30 px-2 py-1 text-[11px] text-danger transition-colors hover:bg-danger/10 disabled:cursor-wait disabled:opacity-50"
                    >
                      重试
                    </button>
                  </div>
                )}
              </div>
            )}

            {pendingPlanApproval && (
              <PlanApprovalBanner
                plan={pendingPlanApproval}
                queueCount={pendingPlanApprovals.length}
                inFlight={planDecisionInFlight === pendingPlanApproval.plan_approval_id}
                error={planDecisionErrors[pendingPlanApproval.plan_approval_id]}
                onApprove={() => onDecisionPlan(pendingPlanApproval.plan_approval_id, "approve")}
                onRequestRevision={(feedback) =>
                  onDecisionPlan(pendingPlanApproval.plan_approval_id, "request_revision", feedback)
                }
              />
            )}

            <div
              className={`absolute left-0 right-0 z-20 transition-all duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none ${
                composerLayoutMode === "center"
                  ? "top-1/2 -translate-y-[46%]"
                  : "bottom-0 translate-y-0"
              }`}
            >
              <BottomComposer
                projectName={project.name}
                runStatus={runStatus}
                provider={configProvider}
                modelId={activeModelId}
                layoutMode={composerLayoutMode}
                inputValue={inputValue}
                onInputChange={onInputChange}
                commandSuggestions={commandSuggestions}
                onInsertCommand={onInsertCommand}
                onSubmit={onSubmit}
                onStop={onStop}
                onModelChange={onModelChange}
                modelSwitchHint={modelSwitchHint}
                focusNonce={composerFocusNonce}
                disabled={!bootstrapReady || pendingPlanApprovals.length > 0}
              />
            </div>
          </div>

          {isRightPanelOpen && currentRun && hasConversation && (
            <div className="absolute right-5 top-4 bottom-[196px] z-30 w-[320px] pointer-events-none">
              <div className="pointer-events-auto h-full">
                <RightInspector
                  run={currentRun}
                  progressItems={progressItems}
                  branchDetail={branchDetail}
                  contextPressure={contextPressure}
                  tokenUsage={tokenUsage}
                  recoveryStatus={recoveryStatus}
                  pendingPermissions={pendingPermissions}
                  permissionDecisionInFlight={permissionDecisionInFlight}
                  permissionDecisionErrors={permissionDecisionErrors}
                  pendingPlanApprovals={pendingPlanApprovals}
                  onDecisionPermission={onDecisionPermission}
                  onDecisionPlan={onDecisionPlan}
                  onApproveAllPermissions={onApproveAllPermissions}
                  onApproveAllPlans={onApproveAllPlans}
                  onOpenArtifact={onOpenArtifact}
                />
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
