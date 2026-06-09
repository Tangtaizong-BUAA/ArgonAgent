import { Folder, FileText } from "lucide-react";
import { suggestedTasks } from "@/data/mock";

interface EmptyStateProps {
  projectName: string;
  projectPath?: string;
  reserveComposerSpace?: boolean;
  onPickProjectFolder?: () => void;
  onSelectSuggestedTask?: (task: string) => void;
}

export function EmptyState({
  projectName,
  projectPath,
  reserveComposerSpace = false,
  onPickProjectFolder,
  onSelectSuggestedTask,
}: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center justify-center h-full px-8">
      <h1 className="text-[28px] font-medium text-text-primary mb-10 tracking-tight">
        我们该在 {projectName} 中做什么？
      </h1>

      <div className="w-full max-w-[720px]">
        {reserveComposerSpace ? <div className="h-[164px] mb-4" aria-hidden="true" /> : null}

        <div className="flex items-center gap-2 text-[13px] text-text-muted mb-5 px-1">
          <Folder size={14} />
          <span className="truncate">{projectPath || projectName}</span>
          {onPickProjectFolder ? (
            <button
              type="button"
              onClick={onPickProjectFolder}
              className="ml-2 rounded-md border border-border-subtle px-2 py-0.5 text-[11px] text-text-secondary hover:bg-bg-card-hover transition-colors"
            >
              选择项目文件夹
            </button>
          ) : null}
        </div>

        <div className="space-y-0.5">
          {suggestedTasks.map((task, i) => (
            <button
              key={i}
              type="button"
              onClick={() => onSelectSuggestedTask?.(task)}
              className="w-full flex items-center gap-3 px-3 py-2.5 rounded-xl text-[13px] text-text-secondary hover:bg-bg-card hover:text-text-primary transition-colors text-left"
            >
              <FileText size={14} className="text-text-muted shrink-0" />
              <span className="truncate">{task}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
