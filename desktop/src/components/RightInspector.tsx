import { useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  AlertCircle,
  Boxes,
  Check,
  ChevronDown,
  CircleDot,
  Clock3,
  Download,
  FileText,
  GitBranch,
  GitCommit,
  History,
  Layers3,
  Loader2,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  Upload,
  Wrench,
} from "lucide-react";
import type { ContextPressureState, RecoveryStatusState } from "./RuntimeStatus";
import type { AgentRun, BranchDetail, ProgressItem, TokenUsageStats } from "@/types";

interface RightInspectorProps {
  run: AgentRun;
  progressItems: ProgressItem[];
  branchDetail: BranchDetail;
  contextPressure: ContextPressureState;
  tokenUsage: TokenUsageStats;
  recoveryStatus: RecoveryStatusState | null;
  pendingPermissions: Array<{
    permission_id: string;
    request_type: string;
    tool_id: string;
    args_preview?: string;
    path_preview?: string;
    risk_level?: string;
  }>;
  permissionDecisionInFlight: string | null;
  permissionDecisionErrors: Record<string, string>;
  pendingPlanApprovals: Array<{
    plan_approval_id: string;
    goal: string;
  }>;
  onDecisionPermission: (
    permissionId: string,
    decision: "allow_once" | "allow_session" | "allow_project_rule" | "deny" | "modify",
    feedback?: string,
  ) => void;
  onDecisionPlan: (planApprovalId: string, decision: "approve" | "request_revision") => void;
  onApproveAllPermissions: () => void;
  onApproveAllPlans: () => void;
  onOpenArtifact?: (path: string) => void;
}

type InspectorTab = "overview" | "diagnostics" | "approvals" | "tools" | "artifacts" | "changes";

const TABS: Array<{ id: InspectorTab; label: string }> = [
  { id: "overview", label: "概览" },
  { id: "diagnostics", label: "诊断" },
  { id: "approvals", label: "审批" },
  { id: "tools", label: "工具" },
  { id: "artifacts", label: "产物" },
  { id: "changes", label: "变更" },
];

export function RightInspector({
  run,
  progressItems,
  branchDetail,
  contextPressure,
  tokenUsage,
  recoveryStatus,
  pendingPermissions,
  permissionDecisionInFlight,
  permissionDecisionErrors,
  pendingPlanApprovals,
  onDecisionPermission,
  onDecisionPlan,
  onApproveAllPermissions,
  onApproveAllPlans,
  onOpenArtifact,
}: RightInspectorProps) {
  const [activeTab, setActiveTab] = useState<InspectorTab>("overview");
  const stats = useMemo(() => buildStats(progressItems, pendingPermissions.length, pendingPlanApprovals.length, run.status), [
    pendingPermissions.length,
    pendingPlanApprovals.length,
    progressItems,
    run.status,
  ]);
  const recentTools = useMemo(
    () => progressItems.filter((item) => item.kind === "tool" || item.toolId).slice(-14).reverse(),
    [progressItems],
  );
  const observability = useMemo(
    () => progressItems.filter((item) => item.kind === "observability").slice(-8).reverse(),
    [progressItems],
  );
  const diagnostics = useMemo(
    () =>
      uniqueDiagnostics(progressItems.filter((item) => item.kind === "observability" || item.category))
        .slice(-28)
        .reverse(),
    [progressItems],
  );
  const recentArtifacts = run.artifacts.slice().reverse();
  const recentChanges = run.changes.slice(-12).reverse();
  const hasPending = pendingPermissions.length > 0 || pendingPlanApprovals.length > 0;

  return (
    <aside className="h-full overflow-hidden rounded-xl border border-white/12 bg-[rgba(20,20,24,0.72)] shadow-[0_25px_60px_rgba(0,0,0,0.42)] backdrop-blur-xl">
      <div className="flex h-full flex-col">
        <div className="border-b border-border-subtle/80 p-4">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-text-muted">
                <CircleDot size={12} className={statusDotClass(run.status)} />
                运行控制台
              </div>
              <div className="mt-2 truncate text-[14px] font-medium text-text-primary">{run.title}</div>
              <div className="mt-1 flex items-center gap-2 text-[11px] text-text-muted">
                <span className="truncate">{run.model}</span>
                <span className="h-1 w-1 rounded-full bg-text-muted/60" />
                <span>{statusLabel(run.status)}</span>
              </div>
            </div>
            <div className={`rounded-lg px-2 py-1 text-[11px] ${healthBadgeClass(stats.health)}`}>
              {stats.health}
            </div>
          </div>

          <div className="mt-4 grid grid-cols-4 gap-2">
            <Metric label="运行" value={stats.running} tone="warning" />
            <Metric label="完成" value={stats.done} tone="success" />
            <Metric label="失败" value={stats.failed} tone={stats.failed > 0 ? "danger" : "muted"} />
            <Metric label="审批" value={stats.pending} tone={stats.pending > 0 ? "warning" : "muted"} />
          </div>
        </div>

        <div className="border-b border-border-subtle/80 px-3 py-2">
          <div className="grid grid-cols-6 gap-1">
            {TABS.map((tab) => (
              <button
                key={tab.id}
                type="button"
                onClick={() => setActiveTab(tab.id)}
                className={`rounded-md px-1.5 py-1.5 text-[11px] transition-colors ${
                  activeTab === tab.id
                    ? "bg-bg-card text-text-primary"
                    : "text-text-muted hover:bg-bg-card/60 hover:text-text-secondary"
                }`}
              >
                {tab.label}
              </button>
            ))}
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto">
          {activeTab === "overview" && (
            <OverviewTab
              stats={stats}
              run={run}
              branchDetail={branchDetail}
              tokenUsage={tokenUsage}
              observability={observability}
              recentTools={recentTools}
            />
          )}
          {activeTab === "diagnostics" && (
            <DiagnosticsTab
              contextPressure={contextPressure}
              recoveryStatus={recoveryStatus}
              diagnostics={diagnostics}
            />
          )}
          {activeTab === "approvals" && (
            <ApprovalsTab
              hasPending={hasPending}
              pendingPermissions={pendingPermissions}
              permissionDecisionInFlight={permissionDecisionInFlight}
              permissionDecisionErrors={permissionDecisionErrors}
              pendingPlanApprovals={pendingPlanApprovals}
              onDecisionPermission={onDecisionPermission}
              onDecisionPlan={onDecisionPlan}
              onApproveAllPermissions={onApproveAllPermissions}
              onApproveAllPlans={onApproveAllPlans}
            />
          )}
          {activeTab === "tools" && <ToolsTab tools={recentTools} />}
          {activeTab === "artifacts" && (
            <ArtifactsTab artifacts={recentArtifacts} total={run.artifacts.length} onOpenArtifact={onOpenArtifact} />
          )}
          {activeTab === "changes" && (
            <ChangesTab changes={recentChanges} total={run.changes.length} branchDetail={branchDetail} />
          )}
        </div>
      </div>
    </aside>
  );
}

function OverviewTab({
  stats,
  run,
  branchDetail,
  tokenUsage,
  observability,
  recentTools,
}: {
  stats: ReturnType<typeof buildStats>;
  run: AgentRun;
  branchDetail: BranchDetail;
  tokenUsage: TokenUsageStats;
  observability: ProgressItem[];
  recentTools: ProgressItem[];
}) {
  return (
    <div className="space-y-4 p-4">
      <section className="rounded-lg border border-border-subtle/80 bg-bg-card/35 p-3">
        <div className="mb-3 flex items-center justify-between gap-2">
          <SectionTitle icon={<Sparkles size={13} />} label="执行态势" />
          <span className="text-[11px] text-text-muted">{stats.total} 个事件线索</span>
        </div>
        <div className="space-y-2">
          <HealthRow label="工具成功率" value={`${stats.successRate}%`} tone={stats.failed > 0 ? "warning" : "success"} />
          <HealthRow label="阻塞审批" value={stats.pending > 0 ? `${stats.pending} 个` : "无"} tone={stats.pending > 0 ? "warning" : "success"} />
          <HealthRow label="最新状态" value={statusLabel(run.status)} tone={run.status === "failed" ? "danger" : "muted"} />
        </div>
      </section>

      <section className="rounded-lg border border-border-subtle/80 bg-bg-card/35 p-3">
        <SectionTitle icon={<CircleDot size={13} />} label="Token 流量" />
        <div className="mt-3 grid grid-cols-2 gap-2">
          <TokenMetric icon={<Upload size={13} />} label="上传 token" value={tokenUsage.uploaded} />
          <TokenMetric icon={<Download size={13} />} label="下载 token" value={tokenUsage.downloaded} />
        </div>
        <div className="mt-3 space-y-2">
          <HealthRow label="reasoning" value={formatTokenCount(tokenUsage.reasoning)} tone="muted" />
          <HealthRow label="cache hit" value={formatTokenCount(tokenUsage.cacheHit)} tone="success" />
          <HealthRow label="cache miss" value={formatTokenCount(tokenUsage.cacheMiss)} tone="warning" />
        </div>
      </section>

      <section className="rounded-lg border border-border-subtle/80 bg-bg-card/35 p-3">
        <SectionTitle icon={<GitBranch size={13} />} label="分支与检查点" />
        <div className="mt-3 space-y-2.5">
          <InfoLine icon={<GitCommit size={13} />} label={branchDetail.name} />
          <InfoLine icon={<Clock3 size={13} />} label={branchDetail.checkpoint ?? "runtime ready"} />
          <InfoLine
            icon={<Layers3 size={13} />}
            label={branchDetail.hasChanges ? "存在待确认运行状态" : "无阻塞变更状态"}
          />
        </div>
      </section>

      <section>
        <SectionTitle icon={<History size={13} />} label="最近工具活动" />
        <div className="mt-2 space-y-2">
          {recentTools.length === 0 ? <EmptyLine text="工具调用会在这里形成实时执行流。" /> : null}
          {recentTools.slice(0, 5).map((item) => (
            <ProgressRow key={item.id} item={item} compact />
          ))}
        </div>
      </section>

      <section>
        <SectionTitle icon={<Boxes size={13} />} label="运行遥测" />
        <div className="mt-2 space-y-2">
          {observability.length === 0 ? <EmptyLine text="协议、缓存、恢复和压缩事件会在这里出现。" /> : null}
          {observability.slice(0, 5).map((item) => (
            <ProgressRow key={item.id} item={item} compact />
          ))}
        </div>
      </section>
    </div>
  );
}

function DiagnosticsTab({
  contextPressure,
  recoveryStatus,
  diagnostics,
}: {
  contextPressure: ContextPressureState;
  recoveryStatus: RecoveryStatusState | null;
  diagnostics: ProgressItem[];
}) {
  const groupedDiagnostics = useMemo(() => {
    const groups: Partial<Record<NonNullable<ProgressItem["category"]>, ProgressItem[]>> = {};
    for (const item of diagnostics) {
      const category = item.category ?? "other";
      groups[category] = [...(groups[category] ?? []), item];
    }
    return groups;
  }, [diagnostics]);

  return (
    <div className="space-y-4 p-4">
      <section className="rounded-lg border border-border-subtle/80 bg-bg-card/35 p-3">
        <div className="mb-3 flex items-center justify-between gap-3">
          <SectionTitle icon={<Layers3 size={13} />} label="上下文压力" />
          <span className={`rounded px-1.5 py-0.5 text-[10px] ${contextBadgeClass(contextPressure.status)}`}>
            {contextStatusLabel(contextPressure.status)}
          </span>
        </div>
        <div className="h-2 overflow-hidden rounded-full bg-bg-card-hover">
          <div
            className={`h-full rounded-full transition-[width] duration-300 ${contextBarClass(contextPressure.status)}`}
            style={{ width: `${contextPressurePercent(contextPressure)}%` }}
          />
        </div>
        <div className="mt-2 text-[12px] leading-5 text-text-secondary">{contextPressure.label}</div>
        <div className="mt-2 grid grid-cols-3 gap-2">
          <MiniMetric label="Prompt" value={formatNullableTokenCount(contextPressure.promptTokens)} />
          <MiniMetric label="上限" value={formatNullableTokenCount(contextPressure.maxTokens)} />
          <MiniMetric label="剩余" value={formatNullableTokenCount(contextPressure.remainingTokens)} />
        </div>
      </section>

      <section className="rounded-lg border border-border-subtle/80 bg-bg-card/35 p-3">
        <div className="mb-3 flex items-center justify-between gap-3">
          <SectionTitle icon={<History size={13} />} label="恢复状态" />
          {recoveryStatus ? (
            <span className={`rounded px-1.5 py-0.5 text-[10px] ${diagnosticBadgeClass(recoveryStatus.status)}`}>
              {progressStatusLabel(recoveryStatus.status)}
            </span>
          ) : null}
        </div>
        {recoveryStatus ? (
          <div className="space-y-2">
            <div className="text-[12px] leading-5 text-text-secondary">{recoveryStatus.label}</div>
            <code className="block truncate rounded-md bg-bg-card/70 px-2 py-1 text-[10px] text-text-muted">
              {recoveryStatus.eventType}
            </code>
          </div>
        ) : (
          <EmptyLine text="暂无恢复动作。出现重试、压缩、预算续写或循环收敛时会显示在这里。" />
        )}
      </section>

      <section>
        <SectionTitle icon={<Boxes size={13} />} label="诊断时间线" />
        <div className="mt-2 space-y-3">
          {diagnostics.length === 0 ? (
            <EmptyLine text="运行诊断会汇总上下文压缩、恢复、参数修复、cache 和子 Agent 事件。" />
          ) : null}
          {(["recovery", "context", "repair", "cache", "permission", "tool", "model", "subagent", "other"] as const).map(
            (category) => {
              const items = groupedDiagnostics[category] ?? [];
              if (items.length === 0) {
                return null;
              }
              return (
                <div key={category} className="space-y-2">
                  <div className="text-[10px] font-semibold uppercase tracking-wider text-text-muted">
                    {diagnosticCategoryLabel(category)}
                  </div>
                  {items.slice(0, 8).map((item) => (
                    <DiagnosticRow key={item.id} item={item} />
                  ))}
                </div>
              );
            },
          )}
        </div>
      </section>
    </div>
  );
}

function ApprovalsTab({
  hasPending,
  pendingPermissions,
  permissionDecisionInFlight,
  permissionDecisionErrors,
  pendingPlanApprovals,
  onDecisionPermission,
  onDecisionPlan,
  onApproveAllPermissions,
  onApproveAllPlans,
}: {
  hasPending: boolean;
  pendingPermissions: RightInspectorProps["pendingPermissions"];
  permissionDecisionInFlight: string | null;
  permissionDecisionErrors: Record<string, string>;
  pendingPlanApprovals: RightInspectorProps["pendingPlanApprovals"];
  onDecisionPermission: RightInspectorProps["onDecisionPermission"];
  onDecisionPlan: RightInspectorProps["onDecisionPlan"];
  onApproveAllPermissions: RightInspectorProps["onApproveAllPermissions"];
  onApproveAllPlans: RightInspectorProps["onApproveAllPlans"];
}) {
  const [permissionSuggestions, setPermissionSuggestions] = useState<Record<string, string>>({});
  return (
    <div className="space-y-4 p-4">
      <div className="rounded-lg border border-border-subtle/80 bg-bg-card/35 p-3">
        <div className="flex items-start gap-2">
          {hasPending ? (
            <ShieldAlert size={16} className="mt-0.5 shrink-0 text-warning" />
          ) : (
            <ShieldCheck size={16} className="mt-0.5 shrink-0 text-success" />
          )}
          <div className="min-w-0">
            <div className="text-[13px] font-medium text-text-primary">
              {hasPending ? "需要确认后继续" : "当前没有阻塞审批"}
            </div>
            <div className="mt-1 text-[11px] leading-5 text-text-muted">
              写文件、Shell、计划进入等高风险动作会停在这里，审批后 runtime 会带着已有证据继续。
            </div>
          </div>
        </div>
        {hasPending ? (
          <div className="mt-3 flex items-center gap-2">
            <button
              onClick={onApproveAllPermissions}
              disabled={pendingPermissions.length === 0}
              className="rounded-md bg-success/15 px-2.5 py-1.5 text-[11px] text-success transition-colors hover:bg-success/25 disabled:cursor-not-allowed disabled:opacity-40"
            >
              全部允许
            </button>
            <button
              onClick={onApproveAllPlans}
              disabled={pendingPlanApprovals.length === 0}
              className="rounded-md bg-accent/15 px-2.5 py-1.5 text-[11px] text-accent transition-colors hover:bg-accent/25 disabled:cursor-not-allowed disabled:opacity-40"
            >
              全部通过计划
            </button>
          </div>
        ) : null}
      </div>

      <div className="space-y-3">
        {pendingPermissions.map((permission) => (
          <div key={permission.permission_id} className="rounded-lg border border-warning/25 bg-warning/5 p-3">
            <div className="flex items-center gap-2 text-[12px] font-medium text-text-primary">
              <Wrench size={13} className="text-warning" />
              {permission.tool_id}
            </div>
            <div className="mt-1 text-[11px] text-text-muted">
              {permission.request_type} · {permission.permission_id}
            </div>
            {permission.args_preview || permission.path_preview ? (
              <div className="mt-2 space-y-1.5">
                <div className="flex items-center gap-2">
                  <span className={`rounded px-1.5 py-0.5 text-[10px] ${riskBadgeClass(permission.risk_level)}`}>
                    {riskLabel(permission.risk_level)}
                  </span>
                  {permission.path_preview ? (
                    <span className="min-w-0 truncate text-[11px] text-text-muted">{permission.path_preview}</span>
                  ) : null}
                </div>
                {permission.args_preview ? (
                  <code
                    className="block max-h-20 overflow-auto whitespace-pre-wrap break-words rounded-md border border-border-subtle/70 bg-bg-card/70 px-2 py-1.5 text-[11px] leading-5 text-text-secondary"
                    title={permission.args_preview}
                  >
                    {permission.args_preview}
                  </code>
                ) : null}
              </div>
            ) : null}
            {permissionDecisionInFlight === permission.permission_id ? (
              <div className="mt-2 flex items-center gap-2 text-[11px] text-warning">
                <Loader2 size={12} className="animate-spin" />
                正在提交并恢复工具执行
              </div>
            ) : null}
            {permissionDecisionErrors[permission.permission_id] ? (
              <div className="mt-2 text-[11px] leading-5 text-danger">
                {permissionDecisionErrors[permission.permission_id]}
              </div>
            ) : null}
            <textarea
              value={permissionSuggestions[permission.permission_id] ?? ""}
              onChange={(event) =>
                setPermissionSuggestions((prev) => ({
                  ...prev,
                  [permission.permission_id]: event.target.value.slice(0, 240),
                }))
              }
              disabled={permissionDecisionInFlight === permission.permission_id}
              rows={2}
              placeholder="拒绝时可给模型一个替代建议…"
              className="mt-3 w-full resize-none rounded-md border border-border-subtle/70 bg-bg-card/60 px-2 py-1.5 text-[11px] leading-5 text-text-secondary outline-none placeholder:text-text-muted disabled:cursor-wait disabled:opacity-60"
            />
            <div className="mt-3 grid grid-cols-2 gap-2">
              <ApprovalButton
                label="允许一次"
                tone="success"
                disabled={permissionDecisionInFlight === permission.permission_id}
                onClick={() => onDecisionPermission(permission.permission_id, "allow_once")}
              />
              <ApprovalButton
                label="本会话"
                tone="accent"
                disabled={permissionDecisionInFlight === permission.permission_id}
                onClick={() => onDecisionPermission(permission.permission_id, "allow_session")}
              />
              <ApprovalButton
                label="项目规则"
                tone="accent"
                disabled={permissionDecisionInFlight === permission.permission_id}
                onClick={() => onDecisionPermission(permission.permission_id, "allow_project_rule")}
              />
              <ApprovalButton
                label="带建议拒绝"
                tone="warning"
                disabled={permissionDecisionInFlight === permission.permission_id}
                onClick={() =>
                  onDecisionPermission(
                    permission.permission_id,
                    "modify",
                    permissionSuggestions[permission.permission_id] ?? "",
                  )
                }
              />
              <ApprovalButton
                label="拒绝"
                tone="danger"
                disabled={permissionDecisionInFlight === permission.permission_id}
                onClick={() => onDecisionPermission(permission.permission_id, "deny")}
              />
            </div>
          </div>
        ))}

        {pendingPlanApprovals.map((plan) => (
          <div key={plan.plan_approval_id} className="rounded-lg border border-accent/25 bg-accent/5 p-3">
            <div className="flex items-center gap-2 text-[12px] font-medium text-text-primary">
              <FileText size={13} className="text-accent" />
              计划审批
            </div>
            <div className="mt-1 break-all text-[11px] text-text-muted">{plan.plan_approval_id}</div>
            <div className="mt-2 rounded-md bg-bg-card/70 px-2 py-1.5 text-[11px] leading-5 text-text-secondary">
              {plan.goal || "等待确认计划内容"}
            </div>
            <div className="mt-3 grid grid-cols-2 gap-2">
              <ApprovalButton label="通过" tone="success" onClick={() => onDecisionPlan(plan.plan_approval_id, "approve")} />
              <ApprovalButton
                label="退回"
                tone="warning"
                onClick={() => onDecisionPlan(plan.plan_approval_id, "request_revision")}
              />
            </div>
          </div>
        ))}

        {!hasPending ? <EmptyLine text="没有待处理审批。高风险动作出现时会自动浮到这里。" /> : null}
      </div>
    </div>
  );
}

function ToolsTab({ tools }: { tools: ProgressItem[] }) {
  return (
    <div className="space-y-2 p-4">
      {tools.length === 0 ? <EmptyLine text="暂无工具调用。读文件、搜索、写入和命令都会在这里展开。" /> : null}
      {tools.map((item) => (
        <ProgressRow key={item.id} item={item} />
      ))}
    </div>
  );
}

function ArtifactsTab({
  artifacts,
  total,
  onOpenArtifact,
}: {
  artifacts: AgentRun["artifacts"];
  total: number;
  onOpenArtifact?: (path: string) => void;
}) {
  const [visibleCount, setVisibleCount] = useState(12);
  const visibleArtifacts = artifacts.slice(0, visibleCount);
  return (
    <div className="space-y-2 p-4">
      {artifacts.length === 0 ? <EmptyLine text="工具结果、报告、日志和补丁线索会汇总在这里。" /> : null}
      {visibleArtifacts.map((artifact) => (
        <button
          key={artifact.id}
          type="button"
          disabled={!artifact.path || !onOpenArtifact}
          onClick={() => artifact.path && onOpenArtifact?.(artifact.path)}
          className="w-full rounded-lg border border-border-subtle/80 bg-bg-card/35 px-3 py-2.5 text-left transition-colors hover:bg-bg-card/70"
          title={artifact.path ? "在 Finder 中显示" : "该产物没有本地路径"}
        >
          <div className="flex items-center gap-2 text-[12px] text-text-secondary">
            <FileText size={13} className="shrink-0 text-text-muted" />
            <span className="min-w-0 flex-1 truncate">{artifact.name}</span>
            <span className="rounded bg-bg-elevated px-1.5 py-0.5 text-[10px] text-text-muted">{artifact.type}</span>
          </div>
          {artifact.path ? <div className="mt-1 truncate text-[11px] text-text-muted">{artifact.path}</div> : null}
        </button>
      ))}
      {total > visibleArtifacts.length ? (
        <button
          type="button"
          onClick={() => setVisibleCount((value) => Math.min(total, value + 12))}
          className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-[12px] text-text-muted transition-colors hover:bg-bg-card/60"
        >
          <ChevronDown size={13} />
          再显示 {total - visibleArtifacts.length} 个
        </button>
      ) : null}
    </div>
  );
}

function ChangesTab({
  changes,
  total,
  branchDetail,
}: {
  changes: AgentRun["changes"];
  total: number;
  branchDetail: BranchDetail;
}) {
  const additions = changes.reduce((sum, change) => sum + change.additions, 0);
  const deletions = changes.reduce((sum, change) => sum + change.deletions, 0);
  return (
    <div className="space-y-3 p-4">
      <div className="rounded-lg border border-border-subtle/80 bg-bg-card/35 p-3">
        <SectionTitle icon={<GitBranch size={13} />} label="变更摘要" />
        <div className="mt-3 flex items-center gap-3 text-[12px]">
          <span className="text-diff-add">+{additions}</span>
          <span className="text-diff-remove">-{deletions}</span>
          <span className="text-text-muted">{total} 个文件线索</span>
        </div>
        <div className="mt-2 text-[11px] text-text-muted">{branchDetail.checkpoint}</div>
      </div>
      {changes.length === 0 ? <EmptyLine text="尚无文件变更线索。写入、补丁和生成文件会在这里出现。" /> : null}
      {changes.map((change) => (
        <div key={`${change.status}:${change.path}`} className="rounded-lg border border-border-subtle/80 bg-bg-card/35 px-3 py-2">
          <div className="flex items-center gap-2 text-[12px] text-text-secondary">
            <span className={`h-2 w-2 rounded-full ${changeStatusClass(change.status)}`} />
            <span className="min-w-0 flex-1 truncate">{change.path}</span>
          </div>
          <div className="mt-1 flex items-center gap-2 text-[11px]">
            <span className="text-diff-add">+{change.additions}</span>
            <span className="text-diff-remove">-{change.deletions}</span>
            <span className="text-text-muted">{change.status}</span>
          </div>
        </div>
      ))}
    </div>
  );
}

function ProgressRow({ item, compact = false }: { item: ProgressItem; compact?: boolean }) {
  const title = item.toolId ?? item.label;
  const detail = item.toolId && item.label !== item.toolId ? item.label : item.detail;
  return (
    <div className={`rounded-lg border border-border-subtle/80 bg-bg-card/35 ${compact ? "px-2.5 py-2" : "px-3 py-2.5"}`}>
      <div className="flex items-start gap-2.5">
        <StatusIcon status={item.status} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[12px] text-text-secondary">{title}</div>
          {detail ? <div className="mt-1 line-clamp-2 text-[11px] leading-5 text-text-muted">{detail}</div> : null}
        </div>
      </div>
    </div>
  );
}

function Metric({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone: "success" | "warning" | "danger" | "muted";
}) {
  const toneClass =
    tone === "success"
      ? "text-success"
      : tone === "warning"
        ? "text-warning"
        : tone === "danger"
          ? "text-danger"
          : "text-text-muted";
  return (
    <div className="rounded-lg border border-border-subtle/80 bg-bg-card/45 px-2 py-2">
      <div className={`text-[15px] font-semibold ${toneClass}`}>{value}</div>
      <div className="mt-0.5 truncate text-[10px] text-text-muted">{label}</div>
    </div>
  );
}

function TokenMetric({ icon, label, value }: { icon: ReactNode; label: string; value: number }) {
  return (
    <div className="rounded-lg border border-border-subtle/80 bg-bg-card/45 px-2.5 py-2">
      <div className="flex items-center gap-1.5 text-accent">
        {icon}
        <span className="text-[15px] font-semibold tabular-nums">{formatTokenCount(value)}</span>
      </div>
      <div className="mt-0.5 truncate text-[10px] text-text-muted">{label}</div>
    </div>
  );
}

function MiniMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border border-border-subtle/70 bg-bg-card/45 px-2 py-1.5">
      <div className="truncate text-[12px] font-medium text-text-secondary">{value}</div>
      <div className="mt-0.5 truncate text-[10px] text-text-muted">{label}</div>
    </div>
  );
}

function DiagnosticRow({ item }: { item: ProgressItem }) {
  return (
    <div className="rounded-lg border border-border-subtle/80 bg-bg-card/35 px-3 py-2.5">
      <div className="flex items-start gap-2.5">
        <StatusIcon status={item.status} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className={`rounded px-1.5 py-0.5 text-[10px] ${diagnosticBadgeClass(item.status)}`}>
              {progressStatusLabel(item.status)}
            </span>
            <span className="min-w-0 truncate text-[12px] text-text-secondary">{item.label}</span>
          </div>
          {item.detail ? <div className="mt-1 line-clamp-2 text-[11px] leading-5 text-text-muted">{item.detail}</div> : null}
          {item.eventType ? (
            <code className="mt-1 block truncate text-[10px] text-text-muted/80">{item.eventType}</code>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function SectionTitle({ icon, label }: { icon: ReactNode; label: string }) {
  return (
    <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-text-muted">
      {icon}
      {label}
    </div>
  );
}

function formatTokenCount(value: number): string {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 10_000) {
    return `${Math.round(value / 1000)}k`;
  }
  if (value >= 1000) {
    return `${(value / 1000).toFixed(1)}k`;
  }
  return String(value);
}

function formatNullableTokenCount(value: number | null): string {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? formatTokenCount(value) : "未知";
}

function HealthRow({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone: "success" | "warning" | "danger" | "muted";
}) {
  return (
    <div className="flex items-center justify-between gap-3 text-[12px]">
      <span className="text-text-muted">{label}</span>
      <span className={toneTextClass(tone)}>{value}</span>
    </div>
  );
}

function uniqueDiagnostics(items: ProgressItem[]): ProgressItem[] {
  const seen = new Set<string>();
  const result: ProgressItem[] = [];
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    const key = `${item.eventType ?? ""}:${item.category ?? ""}:${item.label}:${item.detail ?? ""}:${item.status}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    result.push(item);
  }
  return result.reverse();
}

function riskLabel(value?: string): string {
  if (value === "critical") return "极高风险";
  if (value === "high") return "高风险";
  if (value === "medium") return "中风险";
  return "低风险";
}

function riskBadgeClass(value?: string): string {
  if (value === "critical") return "bg-danger/20 text-danger";
  if (value === "high") return "bg-warning/20 text-warning";
  if (value === "medium") return "bg-accent/15 text-accent";
  return "bg-success/15 text-success";
}

function contextPressurePercent(state: ContextPressureState): number {
  if (!state.promptTokens || !state.maxTokens || state.maxTokens <= 0) {
    return 0;
  }
  return Math.min(100, Math.max(0, Math.round((state.promptTokens / state.maxTokens) * 100)));
}

function contextStatusLabel(status: ContextPressureState["status"]): string {
  if (status === "blocked") return "受阻";
  if (status === "compacting") return "压缩中";
  return "正常";
}

function contextBadgeClass(status: ContextPressureState["status"]): string {
  if (status === "blocked") return "bg-danger/15 text-danger";
  if (status === "compacting") return "bg-warning/15 text-warning";
  return "bg-success/15 text-success";
}

function contextBarClass(status: ContextPressureState["status"]): string {
  if (status === "blocked") return "bg-danger";
  if (status === "compacting") return "bg-warning";
  return "bg-accent";
}

function diagnosticBadgeClass(status: ProgressItem["status"]): string {
  if (status === "failed") return "bg-danger/15 text-danger";
  if (status === "running") return "bg-warning/15 text-warning";
  if (status === "done") return "bg-success/15 text-success";
  return "bg-bg-elevated text-text-muted";
}

function progressStatusLabel(status: ProgressItem["status"]): string {
  if (status === "failed") return "需关注";
  if (status === "running") return "进行中";
  if (status === "done") return "完成";
  return "等待";
}

function diagnosticCategoryLabel(category: NonNullable<ProgressItem["category"]>): string {
  if (category === "recovery") return "恢复与循环";
  if (category === "context") return "上下文";
  if (category === "repair") return "输入修复";
  if (category === "cache") return "缓存";
  if (category === "permission") return "审批恢复";
  if (category === "model") return "模型";
  if (category === "subagent") return "子 Agent";
  if (category === "tool") return "工具";
  return "其他";
}

function InfoLine({ icon, label }: { icon: ReactNode; label: string }) {
  return (
    <div className="flex items-start gap-2 text-[12px] text-text-secondary">
      <span className="mt-0.5 shrink-0 text-text-muted">{icon}</span>
      <span className="min-w-0 break-words">{label}</span>
    </div>
  );
}

function EmptyLine({ text }: { text: string }) {
  return (
    <div className="rounded-lg border border-dashed border-border-subtle/80 px-3 py-3 text-[12px] leading-5 text-text-muted">
      {text}
    </div>
  );
}

function StatusIcon({ status }: { status: ProgressItem["status"] }) {
  if (status === "done") {
    return (
      <div className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-success/15">
        <Check size={10} className="text-success" strokeWidth={2.5} />
      </div>
    );
  }
  if (status === "running") {
    return (
      <div className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-warning/15">
        <Loader2 size={10} className="animate-spin text-warning" />
      </div>
    );
  }
  if (status === "failed") {
    return (
      <div className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-danger/15">
        <AlertCircle size={10} className="text-danger" />
      </div>
    );
  }
  return <div className="mt-0.5 h-4 w-4 shrink-0 rounded-full border border-text-muted/25" />;
}

function ApprovalButton({
  label,
  tone,
  disabled,
  onClick,
}: {
  label: string;
  tone: "success" | "accent" | "warning" | "danger";
  disabled?: boolean;
  onClick: () => void;
}) {
  const className =
    tone === "success"
      ? "bg-success/15 text-success hover:bg-success/25"
      : tone === "accent"
        ? "bg-accent/15 text-accent hover:bg-accent/25"
        : tone === "warning"
          ? "bg-warning/15 text-warning hover:bg-warning/25"
          : "bg-danger/15 text-danger hover:bg-danger/25";
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={`rounded-md px-2 py-1.5 text-[11px] transition-colors disabled:cursor-wait disabled:opacity-50 ${className}`}
    >
      {label}
    </button>
  );
}

function buildStats(
  progressItems: ProgressItem[],
  pendingPermissions: number,
  pendingPlans: number,
  runStatus: AgentRun["status"],
) {
  const done = progressItems.filter((item) => item.status === "done").length;
  const rawRunning = progressItems.filter((item) => item.status === "running").length;
  const terminalRun = runStatus === "completed" || runStatus === "stopped" || runStatus === "failed";
  const running = terminalRun ? 0 : rawRunning;
  const failed = progressItems.filter((item) => item.status === "failed").length;
  const total = progressItems.length;
  const pending = pendingPermissions + pendingPlans;
  const successRate = done + failed === 0 ? 100 : Math.round((done / (done + failed)) * 100);
  const health =
    runStatus === "failed"
      ? "需关注"
      : pending > 0 || runStatus === "waiting_approval"
        ? "待确认"
        : runStatus === "running" || runStatus === "planning"
          ? "运行中"
          : "稳定";
  return { done, running, failed, total, pending, successRate, health };
}

function statusLabel(status: AgentRun["status"]): string {
  if (status === "running") return "执行中";
  if (status === "waiting_approval") return "等待审批";
  if (status === "failed") return "失败";
  if (status === "completed") return "已完成";
  if (status === "stopped") return "已停止";
  if (status === "planning") return "规划中";
  return "空闲";
}

function statusDotClass(status: AgentRun["status"]): string {
  if (status === "running") return "text-warning";
  if (status === "waiting_approval") return "text-accent";
  if (status === "failed") return "text-danger";
  if (status === "completed") return "text-success";
  return "text-text-muted";
}

function healthBadgeClass(health: string): string {
  if (health === "需关注") return "bg-danger/15 text-danger";
  if (health === "待确认") return "bg-warning/15 text-warning";
  if (health === "运行中") return "bg-accent/15 text-accent";
  return "bg-success/15 text-success";
}

function toneTextClass(tone: "success" | "warning" | "danger" | "muted"): string {
  if (tone === "success") return "text-success";
  if (tone === "warning") return "text-warning";
  if (tone === "danger") return "text-danger";
  return "text-text-secondary";
}

function changeStatusClass(status: AgentRun["changes"][number]["status"]): string {
  if (status === "created") return "bg-success";
  if (status === "deleted") return "bg-danger";
  return "bg-warning";
}
