import { useState, type ReactNode } from "react";
import { AlertCircle, CheckCircle2, Download, FileText, GitCommit, Loader2, ShieldAlert, Upload, Wrench } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { PendingPlanApprovalState } from "@/hooks/usePermissionFlow";
import type { ProgressItem, RunStatus, TokenUsageStats } from "@/types";

export interface ContextPressureState {
  promptTokens: number | null;
  maxTokens: number | null;
  remainingTokens: number | null;
  status: "normal" | "compacting" | "blocked";
  label: string;
}

export interface RecoveryStatusState {
  label: string;
  status: ProgressItem["status"];
  eventType: string;
}

export function PlanApprovalBanner({
  plan,
  queueCount,
  inFlight,
  error,
  onApprove,
  onRequestRevision,
}: {
  plan: PendingPlanApprovalState;
  queueCount: number;
  inFlight: boolean;
  error?: string;
  onApprove: () => void;
  onRequestRevision: (feedback: string) => void;
}) {
  const [revisionFeedback, setRevisionFeedback] = useState("");
  const preview = (plan.plan_preview || plan.goal || "计划内容等待 runtime 同步").trim();
  return (
    <div
      role="dialog"
      aria-modal="false"
      aria-label="计划审批"
      className="absolute left-1/2 bottom-[132px] z-50 w-[min(760px,calc(100%-36px))] -translate-x-1/2 rounded-lg border border-warning/45 bg-bg-panel/95 shadow-2xl backdrop-blur pointer-events-auto"
    >
      <div className="flex items-start gap-3 px-4 py-3">
        <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-warning/15 text-warning">
          <FileText size={17} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div className="text-[13px] font-semibold text-text-primary">计划等待审批</div>
              <div className="mt-0.5 text-[11px] text-text-muted">
                {plan.plan_approval_id}
                {queueCount > 1 ? ` · 队列中还有 ${queueCount - 1} 个` : ""}
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <button
                type="button"
                aria-label="退回计划并请求修订"
                disabled={inFlight}
                onClick={() => onRequestRevision(revisionFeedback)}
                className="rounded-md border border-warning/30 px-3 py-1.5 text-[12px] text-warning transition-colors hover:bg-warning/10 disabled:cursor-wait disabled:opacity-50"
              >
                退回修订
              </button>
              <button
                type="button"
                aria-label="批准计划并继续执行"
                disabled={inFlight}
                onClick={onApprove}
                className="inline-flex items-center gap-1.5 rounded-md bg-success/20 px-3 py-1.5 text-[12px] text-success transition-colors hover:bg-success/30 disabled:cursor-wait disabled:opacity-50"
              >
                {inFlight ? <Loader2 size={13} className="animate-spin" /> : <CheckCircle2 size={13} />}
                批准并继续
              </button>
            </div>
          </div>
          <div className="mt-3 max-h-[min(42vh,260px)] overflow-auto rounded-md border border-border-subtle/80 bg-bg-card/70 px-3 py-2 text-[12px] leading-5 text-text-secondary">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{preview}</ReactMarkdown>
          </div>
          <textarea
            value={revisionFeedback}
            onChange={(event) => setRevisionFeedback(event.target.value.slice(0, 240))}
            disabled={inFlight}
            rows={2}
            placeholder="退回时可写明要改哪里…"
            className="mt-2 w-full resize-none rounded-md border border-border-subtle/80 bg-bg-card/70 px-3 py-2 text-[12px] leading-5 text-text-secondary outline-none placeholder:text-text-muted disabled:cursor-wait disabled:opacity-60"
          />
          {error ? (
            <div className="mt-2 flex items-start gap-2 text-[11px] leading-5 text-danger">
              <AlertCircle size={13} className="mt-0.5 shrink-0" />
              <span className="min-w-0">{error}</span>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

export function RunDashboardStrip({
  status,
  running,
  done,
  failed,
  toolCount,
  pending,
  additions,
  deletions,
  tokenUsage,
  lastLabel,
  runtimeError,
  contextPressure,
  recoveryStatus,
}: {
  status: RunStatus;
  running: number;
  done: number;
  failed: number;
  toolCount: number;
  pending: number;
  additions: number;
  deletions: number;
  tokenUsage: TokenUsageStats;
  lastLabel: string;
  runtimeError: string | null;
  contextPressure: ContextPressureState;
  recoveryStatus: RecoveryStatusState | null;
}) {
  const statusTone =
    status === "failed"
      ? "danger"
      : status === "waiting_approval"
        ? "warning"
        : status === "running"
          ? "accent"
          : "success";
  return (
    <div className="sticky top-0 z-10 border-b border-border-subtle/70 bg-bg-app/88 px-8 py-3 backdrop-blur-xl">
      <div className="mx-auto flex max-w-[980px] items-center gap-3">
        <div className="min-w-0 flex-1 rounded-lg border border-border-subtle/80 bg-bg-card/45 px-3 py-2">
          <div className="flex items-center gap-2 text-[12px] text-text-secondary">
            {status === "running" ? (
              <Loader2 size={13} className="animate-spin text-warning" />
            ) : status === "failed" ? (
              <AlertCircle size={13} className="text-danger" />
            ) : status === "waiting_approval" ? (
              <ShieldAlert size={13} className="text-warning" />
            ) : (
              <CheckCircle2 size={13} className="text-success" />
            )}
            <span className={`shrink-0 ${dashboardToneText(statusTone)}`}>{dashboardStatus(status)}</span>
            <span className="min-w-0 truncate text-text-muted">{lastLabel}</span>
          </div>
          <ContextPressureBar state={contextPressure} />
          {runtimeError ? <div className="mt-1 truncate text-[11px] text-danger">{runtimeError}</div> : null}
        </div>

        <DashboardMetric icon={<Wrench size={13} />} label="工具" value={toolCount} />
        <DashboardMetric icon={<Loader2 size={13} />} label="运行" value={running} spin={running > 0} />
        <DashboardMetric icon={<CheckCircle2 size={13} />} label="完成" value={done} tone="success" />
        <DashboardMetric icon={<ShieldAlert size={13} />} label="审批" value={pending} tone={pending > 0 ? "warning" : "muted"} />
        <DashboardMetric icon={<Upload size={13} />} label="上传" value={formatTokenCount(tokenUsage.uploaded)} tone="accent" />
        <DashboardMetric icon={<Download size={13} />} label="下载" value={formatTokenCount(tokenUsage.downloaded)} tone="accent" />
        <div className="hidden rounded-lg border border-border-subtle/80 bg-bg-card/45 px-3 py-2 text-[12px] md:block">
          <div className="flex items-center gap-2 text-text-muted">
            <GitCommit size={13} />
            <span className="text-diff-add">+{additions}</span>
            <span className="text-diff-remove">-{deletions}</span>
          </div>
        </div>
        {failed > 0 ? (
          <div className="rounded-lg border border-danger/25 bg-danger/10 px-3 py-2 text-[12px] text-danger">
            {failed} 失败
          </div>
        ) : null}
        {recoveryStatus ? <RecoveryStatusChip recovery={recoveryStatus} /> : null}
      </div>
    </div>
  );
}

function ContextPressureBar({ state }: { state: ContextPressureState }) {
  const ratio = contextPressureRatio(state);
  const width = ratio === null ? 0 : Math.round(ratio * 100);
  const toneClass =
    state.status === "blocked"
      ? "bg-danger"
      : state.status === "compacting"
        ? "bg-warning"
        : ratio !== null && ratio > 0.8
          ? "bg-warning"
          : "bg-accent";
  const textTone =
    state.status === "blocked"
      ? "text-danger"
      : state.status === "compacting"
        ? "text-warning"
        : "text-text-muted";
  return (
    <div className="mt-2">
      <div className="h-1.5 overflow-hidden rounded-full bg-bg-card-hover">
        <div
          className={`h-full rounded-full transition-[width] duration-300 ${toneClass}`}
          style={{ width: `${width}%` }}
        />
      </div>
      <div className={`mt-1 flex items-center justify-between gap-2 text-[10px] ${textTone}`}>
        <span className="min-w-0 truncate">{state.label}</span>
        {state.remainingTokens ? (
          <span className="shrink-0">剩余 {formatTokenCount(state.remainingTokens)}</span>
        ) : null}
      </div>
    </div>
  );
}

function RecoveryStatusChip({ recovery }: { recovery: RecoveryStatusState }) {
  const tone =
    recovery.status === "failed"
      ? "border-danger/25 bg-danger/10 text-danger"
      : recovery.status === "done"
        ? "border-success/25 bg-success/10 text-success"
        : "border-warning/25 bg-warning/10 text-warning";
  return (
    <div
      className={`hidden max-w-[220px] rounded-lg border px-3 py-2 text-[12px] lg:block ${tone}`}
      title={recovery.eventType}
    >
      <div className="flex items-center gap-2">
        {recovery.status === "running" ? <Loader2 size={13} className="animate-spin" /> : <AlertCircle size={13} />}
        <span className="truncate">{recovery.label}</span>
      </div>
      <div className="mt-0.5 truncate text-[10px] opacity-70">{recovery.eventType}</div>
    </div>
  );
}

function DashboardMetric({
  icon,
  label,
  value,
  tone = "muted",
  spin = false,
}: {
  icon: ReactNode;
  label: string;
  value: number | string;
  tone?: "success" | "warning" | "accent" | "muted";
  spin?: boolean;
}) {
  const toneClass =
    tone === "success"
      ? "text-success"
      : tone === "warning"
        ? "text-warning"
        : tone === "accent"
          ? "text-accent"
          : "text-text-secondary";
  return (
    <div className="hidden rounded-lg border border-border-subtle/80 bg-bg-card/45 px-3 py-2 text-[12px] sm:block">
      <div className={`flex items-center gap-2 ${toneClass}`}>
        <span className={spin ? "animate-spin" : ""}>{icon}</span>
        <span>{value}</span>
      </div>
      <div className="mt-0.5 text-[10px] text-text-muted">{label}</div>
    </div>
  );
}

function contextPressureRatio(state: ContextPressureState): number | null {
  if (!state.promptTokens || !state.maxTokens) {
    return null;
  }
  return Math.min(1, Math.max(0, state.promptTokens / state.maxTokens));
}

export function formatTokenCount(value: number): string {
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

function dashboardStatus(status: RunStatus): string {
  if (status === "running") return "执行中";
  if (status === "waiting_approval") return "等待审批";
  if (status === "failed") return "失败";
  if (status === "completed") return "已完成";
  if (status === "stopped") return "已停止";
  return "准备就绪";
}

function dashboardToneText(tone: "danger" | "warning" | "accent" | "success"): string {
  if (tone === "danger") return "text-danger";
  if (tone === "warning") return "text-warning";
  if (tone === "accent") return "text-accent";
  return "text-success";
}
