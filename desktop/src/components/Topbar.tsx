import { Download, PanelRight, Play, RotateCcw, GitBranch } from "lucide-react";
import type { AgentRun, RunStatus } from "@/types";

export type AutonomyModeId = "conservative" | "fast_auto" | "manual_review";

interface TopbarProps {
  currentRun: AgentRun | null;
  runStatus: RunStatus;
  isRightPanelOpen: boolean;
  onToggleRightPanel: () => void;
  onContinue: () => void;
  onRetry: () => void;
  onExport: () => void;
  autonomyMode?: AutonomyModeId;
  onAutonomyModeChange?: (mode: AutonomyModeId) => void;
}

function statusLabel(status: RunStatus): string {
  if (status === "running") return "执行中";
  if (status === "waiting_approval") return "等待审批";
  if (status === "failed") return "失败";
  if (status === "completed") return "已完成";
  if (status === "stopped") return "已停止";
  return "空闲";
}

export function Topbar({
  currentRun,
  runStatus,
  isRightPanelOpen,
  onToggleRightPanel,
  onContinue,
  onRetry,
  onExport,
  autonomyMode = "conservative",
  onAutonomyModeChange,
}: TopbarProps) {
  const totalAdditions = currentRun?.changes.reduce((s, c) => s + c.additions, 0) ?? 0;
  const totalDeletions = currentRun?.changes.reduce((s, c) => s + c.deletions, 0) ?? 0;
  const canOperate = Boolean(currentRun);
  const canSwitchAutonomyMode = Boolean(onAutonomyModeChange);

  return (
      <header
        className="h-10 flex items-center justify-between px-4 border-b border-border-subtle bg-bg-app shrink-0"
        style={{ WebkitAppRegion: "drag" } as any}
      >
      <div className="flex items-center" style={{ WebkitAppRegion: "no-drag" } as any}>
        {currentRun ? (
          <div className="flex items-center gap-3">
            <span className="text-[13px] text-text-secondary truncate max-w-[380px]">
              {currentRun.title}
            </span>
            {currentRun.changes.length > 0 && (
              <span className="flex items-center gap-1.5 text-[12px]">
                <span className="text-diff-add font-medium">+{totalAdditions}</span>
                <span className="text-diff-remove font-medium">-{totalDeletions}</span>
              </span>
            )}
            {currentRun.branch && (
              <span className="flex items-center gap-1 text-[11px] text-text-muted bg-bg-card px-1.5 py-0.5 rounded-md">
                <GitBranch size={11} />
                {currentRun.branch}
              </span>
            )}
            <span className="text-[11px] text-text-muted bg-bg-card px-1.5 py-0.5 rounded-md">
              {statusLabel(runStatus)}
            </span>
          </div>
        ) : (
          <span className="text-[13px] text-text-muted">准备开始新任务</span>
        )}
      </div>

      <div className="flex items-center gap-1.5" style={{ WebkitAppRegion: "no-drag" } as any}>
        <select
          value={autonomyMode}
          disabled={!canSwitchAutonomyMode}
          onChange={(event) => onAutonomyModeChange?.(event.target.value as AutonomyModeId)}
          className="h-7 rounded-md border border-border-subtle bg-bg-card px-2 text-[11px] text-text-muted outline-none transition-colors hover:text-text-secondary disabled:cursor-not-allowed disabled:opacity-40"
          title="切换自主执行模式"
        >
          <option value="conservative">保守</option>
          <option value="fast_auto">快速自动</option>
          <option value="manual_review">手动审查</option>
        </select>
        <button
          onClick={onContinue}
          disabled={!canOperate}
          className="p-1.5 rounded-md text-text-muted hover:text-accent hover:bg-bg-card transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          title="继续执行"
        >
          <Play size={14} />
        </button>
        <button
          onClick={onRetry}
          disabled={!canOperate}
          className="p-1.5 rounded-md text-text-muted hover:text-text-secondary hover:bg-bg-card transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          title="重试上一条用户指令"
        >
          <RotateCcw size={14} />
        </button>
        <button
          onClick={onExport}
          disabled={!canOperate}
          className="p-1.5 rounded-md text-text-muted hover:text-text-secondary hover:bg-bg-card transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          title="导出会话事件"
        >
          <Download size={14} />
        </button>
        <button
          onClick={onToggleRightPanel}
          className={`p-1.5 rounded-md transition-colors ${
            isRightPanelOpen
              ? "text-accent bg-bg-card"
              : "text-text-muted hover:text-text-secondary hover:bg-bg-card"
          }`}
          title="切换右侧面板"
        >
          <PanelRight size={14} />
        </button>
      </div>
    </header>
  );
}
