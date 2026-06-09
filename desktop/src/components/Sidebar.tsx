import {
  MessageSquarePlus,
  Search,
  Puzzle,
  Bot,
  Settings,
  LogOut,
  Plus,
  ChevronDown,
  ChevronRight,
  FolderOpen,
} from "lucide-react";
import React from "react";
import type { Project, AgentRun } from "@/types";

interface SidebarProps {
  projects: Project[];
  selectedProject: Project;
  currentRun: AgentRun | null;
  onSelectProject: (project: Project) => void;
  onSelectRun: (run: AgentRun) => void;
  onNewRun: () => void;
  onPickProjectFolder: () => void;
  onLogout: () => void;
}

function formatTimeAgo(dateStr: string): string {
  const d = new Date(dateStr);
  const now = new Date();
  const diff = now.getTime() - d.getTime();
  const minutes = Math.floor(diff / 60000);
  const hours = Math.floor(diff / 3600000);
  const days = Math.floor(diff / 86400000);
  if (days > 0) return `${days} 天`;
  if (hours > 0) return `${hours} 小时`;
  if (minutes > 0) return `${minutes} 分钟`;
  return "刚刚";
}

export function Sidebar({
  projects,
  selectedProject,
  currentRun,
  onSelectProject,
  onSelectRun,
  onNewRun,
  onPickProjectFolder,
  onLogout,
}: SidebarProps) {
  const [expandedProjects, setExpandedProjects] = React.useState<Set<string>>(
    () => new Set([selectedProject.id])
  );
  const [settingsMenuOpen, setSettingsMenuOpen] = React.useState(false);
  const [activePanel, setActivePanel] = React.useState<"search" | "plugins" | "automation" | null>(null);
  const [searchQuery, setSearchQuery] = React.useState("");
  const settingsMenuRef = React.useRef<HTMLDivElement | null>(null);
  const visibleProjects = React.useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    if (!query) {
      return projects;
    }
    return projects
      .map((project) => ({
        ...project,
        runs: project.runs.filter((run) =>
          `${project.name} ${project.path} ${run.title} ${run.branch ?? ""}`.toLowerCase().includes(query),
        ),
      }))
      .filter((project) => project.runs.length > 0 || project.name.toLowerCase().includes(query));
  }, [projects, searchQuery]);

  React.useEffect(() => {
    const onDocClick = (event: MouseEvent) => {
      if (!settingsMenuRef.current) {
        return;
      }
      if (!settingsMenuRef.current.contains(event.target as Node)) {
        setSettingsMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", onDocClick);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
    };
  }, []);

  const toggleProject = (projectId: string) => {
    setExpandedProjects((prev) => {
      const next = new Set(prev);
      if (next.has(projectId)) {
        next.delete(projectId);
      } else {
        next.add(projectId);
      }
      return next;
    });
  };

  return (
    <aside className="w-[260px] flex flex-col bg-[#1e1e20] border-r border-border-subtle shrink-0">
      {/* macOS traffic light placeholder area */}
      <div className="h-10 flex items-center px-4" style={{ WebkitAppRegion: "drag" } as any}>
        <span className="text-xs text-text-muted font-medium">Argon Agent</span>
      </div>

      {/* Global Nav */}
      <nav className="px-2 pb-2">
        <SidebarItem icon={<MessageSquarePlus size={16} />} label="新对话" onClick={onNewRun} />
        <SidebarItem
          icon={<Search size={16} />}
          label="搜索"
          active={activePanel === "search"}
          onClick={() => setActivePanel((value) => (value === "search" ? null : "search"))}
        />
        <SidebarItem
          icon={<Puzzle size={16} />}
          label="插件"
          active={activePanel === "plugins"}
          onClick={() => setActivePanel((value) => (value === "plugins" ? null : "plugins"))}
        />
        <SidebarItem
          icon={<Bot size={16} />}
          label="自动化"
          active={activePanel === "automation"}
          onClick={() => setActivePanel((value) => (value === "automation" ? null : "automation"))}
        />
      </nav>

      {activePanel ? (
        <div className="mx-2 mb-2 rounded-lg border border-border-subtle bg-bg-card/70 p-2">
          {activePanel === "search" ? (
            <div>
              <label className="mb-1 block text-[11px] font-medium text-text-muted">搜索会话</label>
              <input
                value={searchQuery}
                onChange={(event) => setSearchQuery(event.target.value)}
                placeholder="标题、分支、路径"
                className="w-full rounded-md border border-border-subtle bg-[#18181b] px-2 py-1.5 text-[12px] text-text-secondary outline-none placeholder:text-text-muted focus:border-accent"
              />
            </div>
          ) : activePanel === "plugins" ? (
            <FeatureList
              items={[
                ["Runtime Bridge", "Tauri 事件流和本地命令桥接已启用"],
                ["DeepSeek/Qwen Native", "原生 profile、parser 和工具契约"],
                ["Event Export", "可导出 JSONL 供 GUI 回放和问题复现"],
              ]}
            />
          ) : (
            <FeatureList
              items={[
                ["继续当前任务", "顶部播放按钮会使用上下文续跑"],
                ["审批恢复", "允许后自动恢复挂起工具"],
                ["事件镜像", "会话摘要会写入本地 workspace 记录"],
              ]}
            />
          )}
        </div>
      ) : null}

      {/* Divider */}
      <div className="mx-3 h-px bg-border-subtle my-1" />

      {/* Projects */}
      <div className="flex-1 overflow-y-auto px-2 py-1">
        <div className="px-2 py-1.5 flex items-center justify-between gap-2">
          <span className="text-[11px] font-semibold text-text-muted uppercase tracking-wider">
            项目
          </span>
          <button
            type="button"
            onClick={onPickProjectFolder}
            className="inline-flex items-center gap-1 rounded-md border border-border-subtle px-1.5 py-0.5 text-[11px] text-text-muted hover:text-text-secondary hover:bg-bg-card-hover transition-colors"
            title="选择项目文件夹"
          >
            <Plus size={11} />
            <span>文件夹</span>
          </button>
        </div>
        {visibleProjects.map((project) => {
          const isExpanded = expandedProjects.has(project.id);
          const isSelected = selectedProject.id === project.id;
          return (
            <div key={project.id}>
              <div
                className={`w-full flex items-center gap-1.5 px-2 py-1.5 rounded-lg text-[13px] transition-colors ${
                  isSelected
                    ? "bg-bg-card text-text-primary"
                    : "text-text-secondary hover:bg-bg-card-hover hover:text-text-primary"
                } group`}
              >
                <button
                  type="button"
                  onClick={() => {
                    onSelectProject(project);
                    toggleProject(project.id);
                  }}
                  className="min-w-0 flex flex-1 items-center gap-1.5 text-left"
                >
                  {isExpanded ? (
                    <ChevronDown size={14} className="text-text-muted shrink-0" />
                  ) : (
                    <ChevronRight size={14} className="text-text-muted shrink-0" />
                  )}
                  <FolderOpen size={14} className="shrink-0 text-text-muted" />
                  <span className="truncate flex-1">{project.name}</span>
                </button>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    onSelectProject(project);
                    onNewRun();
                  }}
                  className="opacity-0 group-hover:opacity-100 hover:text-accent shrink-0"
                  title="新建对话"
                >
                  <MessageSquarePlus size={14} />
                </button>
              </div>

              {isExpanded && (
                <div className="ml-5 border-l border-border-subtle pl-2 mt-0.5 space-y-0.5">
                  {project.runs.map((run) => {
                    const isRunActive = currentRun?.id === run.id;
                    return (
                      <button
                        key={run.id}
                        onClick={() => onSelectRun(run)}
                        className={`w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-[13px] transition-colors ${
                          isRunActive
                            ? "bg-bg-card text-text-primary"
                            : "text-text-secondary hover:bg-bg-card-hover hover:text-text-primary"
                        }`}
                      >
                        <span className="truncate flex-1 text-left">{run.title}</span>
                        <span className="text-[11px] text-text-muted shrink-0">
                          {formatTimeAgo(run.updatedAt)}
                        </span>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
        {visibleProjects.length === 0 ? (
          <div className="px-2 py-4 text-[12px] text-text-muted">没有匹配的会话</div>
        ) : null}
      </div>

      {/* Bottom Settings */}
      <div className="p-2 border-t border-border-subtle relative" ref={settingsMenuRef}>
        <SidebarItem
          icon={<Settings size={16} />}
          label="设置"
          onClick={() => setSettingsMenuOpen((value) => !value)}
        />
        {settingsMenuOpen ? (
          <div className="absolute left-2 right-2 bottom-[44px] rounded-xl border border-border-subtle bg-[#202024]/95 backdrop-blur-md shadow-2xl p-1 z-20">
            <button
              type="button"
              onClick={() => {
                setSettingsMenuOpen(false);
                if (window.confirm("确定要退出登录并清除本地配置吗？")) {
                  onLogout();
                }
              }}
              className="w-full flex items-center gap-2 px-2.5 py-2 rounded-lg text-[13px] text-text-secondary hover:bg-bg-card-hover hover:text-text-primary transition-colors"
            >
              <LogOut size={14} />
              <span>退出登录</span>
            </button>
          </div>
        ) : null}
      </div>
    </aside>
  );
}

function SidebarItem({
  icon,
  label,
  active = false,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  active?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-[13px] transition-colors ${
        active
          ? "bg-bg-card text-text-primary"
          : "text-text-secondary hover:bg-bg-card-hover hover:text-text-primary"
      }`}
    >
      <span className="text-text-muted">{icon}</span>
      <span className="text-left">{label}</span>
    </button>
  );
}

function FeatureList({ items }: { items: Array<[string, string]> }) {
  return (
    <div className="space-y-2">
      {items.map(([title, detail]) => (
        <div key={title} className="rounded-md bg-[#18181b] px-2 py-1.5">
          <div className="text-[12px] text-text-secondary">{title}</div>
          <div className="mt-0.5 text-[11px] leading-4 text-text-muted">{detail}</div>
        </div>
      ))}
    </div>
  );
}
