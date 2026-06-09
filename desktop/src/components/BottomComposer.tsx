import { useEffect, useMemo, useRef, useState } from "react";
import { Plus, Sparkles, ArrowUp, Check, ChevronDown, Square } from "lucide-react";
import type { RunStatus } from "@/types";

export type DeepSeekModelId = "deepseek-v4-flash" | "deepseek-v4-pro";

interface BottomComposerProps {
  projectName: string;
  runStatus: RunStatus;
  provider: "deepseek" | "qwen";
  modelId?: string;
  layoutMode: "center" | "bottom";
  inputValue: string;
  onInputChange: (value: string) => void;
  commandSuggestions?: string[];
  onInsertCommand?: (command: string) => void;
  onSubmit: () => void;
  onStop?: () => void;
  onModelChange?: (modelId: DeepSeekModelId) => void;
  modelSwitchHint?: string | null;
  disabled?: boolean;
  focusNonce?: number;
}

interface ModelOption {
  id: DeepSeekModelId;
  label: string;
}

const DEEPSEEK_MODEL_OPTIONS: ModelOption[] = [
  { id: "deepseek-v4-flash", label: "DeepSeek-V4-Flash" },
  { id: "deepseek-v4-pro", label: "DeepSeek-V4-Pro" },
];

function labelForStatus(status: RunStatus): string {
  if (status === "running") return "执行中";
  if (status === "waiting_approval") return "等待审批";
  if (status === "failed") return "执行失败";
  if (status === "completed") return "已完成";
  return "自动审查";
}

function normalizeModelId(modelId?: string): DeepSeekModelId {
  if (modelId === "deepseek-v4-pro") {
    return "deepseek-v4-pro";
  }
  return "deepseek-v4-flash";
}

export function BottomComposer({
  projectName,
  runStatus,
  provider,
  modelId,
  layoutMode,
  inputValue,
  onInputChange,
  commandSuggestions = [],
  onInsertCommand,
  onSubmit,
  onStop,
  onModelChange,
  modelSwitchHint,
  disabled = false,
  focusNonce = 0,
}: BottomComposerProps) {
  const [selectedCommandIndex, setSelectedCommandIndex] = useState(0);
  const [isModelMenuOpen, setIsModelMenuOpen] = useState(false);
  const modelMenuRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const deepseekModelId = normalizeModelId(modelId);
  const modelLabel =
    provider === "deepseek"
      ? DEEPSEEK_MODEL_OPTIONS.find((item) => item.id === deepseekModelId)?.label ?? "DeepSeek-V4-Flash"
      : "Qwen 3.6-27B";
  const modeLabel = labelForStatus(runStatus);
  const normalizedInput = inputValue.trimStart();
  const commandMode = normalizedInput.startsWith("/");
  const commandQuery = commandMode ? normalizedInput.slice(1).trim().toLowerCase() : "";
  const commandHasArgs = commandMode && /\s/.test(normalizedInput.slice(1).trim());

  const filteredCommands = useMemo(() => {
    if (!commandMode) {
      return [];
    }
    const source = commandSuggestions
      .map((item) => String(item).trim())
      .filter((item) => item.startsWith("/"));
    if (!commandQuery) {
      return source.slice(0, 8);
    }
    return source.filter((item) => item.toLowerCase().includes(commandQuery)).slice(0, 8);
  }, [commandMode, commandQuery, commandSuggestions]);

  useEffect(() => {
    setSelectedCommandIndex(0);
  }, [commandQuery, commandMode]);

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (!modelMenuRef.current) {
        return;
      }
      if (!modelMenuRef.current.contains(event.target as Node)) {
        setIsModelMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, []);

  useEffect(() => {
    if (focusNonce > 0) {
      textareaRef.current?.focus();
    }
  }, [focusNonce]);

  const applyCommand = (command: string) => {
    if (onInsertCommand) {
      onInsertCommand(command);
      return;
    }
    onInputChange(command.endsWith(" ") ? command : `${command} `);
  };

  const insertToken = (token: string) => {
    const prefix = inputValue.trim().length > 0 && !inputValue.endsWith(" ") ? " " : "";
    onInputChange(`${inputValue}${prefix}${token}`);
  };

  const activeCommand = filteredCommands[selectedCommandIndex] ?? filteredCommands[0];
  const isCenterLayout = layoutMode === "center";
  const quickCommands = commandSuggestions
    .filter((command) => ["/plan", "/review", "/status"].includes(command))
    .slice(0, 3);
  const isRunning = runStatus === "running";

  return (
    <div className={`w-full ${isCenterLayout ? "" : "pb-4 pt-2"}`}>
      <div className="max-w-[860px] mx-auto px-6">
        <div className="bg-bg-card border border-border-subtle rounded-2xl p-1 shadow-[0_10px_40px_rgba(0,0,0,0.24)]">
          <textarea
            ref={textareaRef}
            rows={1}
            placeholder={
              disabled
                ? "等待当前审批处理完成…"
                : isCenterLayout
                  ? "描述任务，或输入 @ 添加文件、日志、上下文…"
                  : "继续要求或追问…"
            }
            value={inputValue}
            disabled={disabled}
            onChange={(event) => onInputChange(event.target.value)}
            onKeyDown={(event) => {
              if (disabled) {
                event.preventDefault();
                return;
              }
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                event.preventDefault();
                onSubmit();
                return;
              }
              if (commandMode && filteredCommands.length > 0) {
                if (event.key === "ArrowDown") {
                  event.preventDefault();
                  setSelectedCommandIndex((value) => (value + 1) % filteredCommands.length);
                  return;
                }
                if (event.key === "ArrowUp") {
                  event.preventDefault();
                  setSelectedCommandIndex((value) => (value - 1 + filteredCommands.length) % filteredCommands.length);
                  return;
                }
                if (event.key === "Tab") {
                  event.preventDefault();
                  applyCommand(activeCommand);
                  return;
                }
                if (event.key === "Enter" && !event.shiftKey && !commandHasArgs) {
                  event.preventDefault();
                  applyCommand(activeCommand);
                  return;
                }
              }
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                onSubmit();
              }
            }}
            className="w-full bg-transparent text-[14px] text-text-primary placeholder:text-text-muted px-3 py-2.5 resize-none outline-none leading-relaxed disabled:cursor-not-allowed disabled:text-text-muted"
            style={{ minHeight: "24px", maxHeight: "120px" }}
          />

          {commandMode && filteredCommands.length > 0 && (
            <div className="px-2 pb-2">
              <div className="rounded-xl border border-border-subtle bg-[#17171a] overflow-hidden">
                {filteredCommands.map((command, index) => (
                  <button
                    key={command}
                    type="button"
                    disabled={disabled}
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => applyCommand(command)}
                    className={`w-full flex items-center justify-between gap-3 px-3 py-2 text-left text-[12px] transition-colors ${
                      index === selectedCommandIndex
                        ? "bg-bg-card text-text-primary"
                        : "text-text-secondary hover:bg-bg-card-hover"
                    }`}
                  >
                    <span className="truncate">{command}</span>
                    <span className="text-[11px] text-text-muted shrink-0">CLI</span>
                  </button>
                ))}
              </div>
            </div>
          )}

          {!commandMode && quickCommands.length > 0 && inputValue.trim().length === 0 && (
            <div className="px-2 pb-2">
              <div className="flex flex-wrap items-center gap-1.5">
                {quickCommands.map((command) => (
                  <button
                    key={command}
                    type="button"
                    disabled={disabled}
                    onClick={() => applyCommand(command)}
                    className="rounded-lg border border-border-subtle bg-[#17171a] px-2 py-1 text-[11px] text-text-muted transition-colors hover:border-accent/40 hover:text-text-secondary disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {command}
                  </button>
                ))}
              </div>
            </div>
          )}

          <div className="flex items-center justify-between px-2 pb-1.5 relative">
            <div className="flex items-center gap-1">
              <button
                type="button"
                onClick={() => insertToken("@")}
                disabled={disabled}
                className="p-1.5 rounded-lg text-text-muted hover:text-text-secondary hover:bg-bg-card-hover transition-colors disabled:cursor-not-allowed disabled:opacity-50"
                title="插入上下文引用"
              >
                <Plus size={16} />
              </button>
              <button
                type="button"
                onClick={() => applyCommand(runStatus === "planning" ? "/execute" : "/plan")}
                disabled={disabled}
                className="flex items-center gap-1 px-2 py-1 rounded-lg text-[12px] text-accent hover:bg-bg-card-hover transition-colors disabled:cursor-not-allowed disabled:opacity-50"
                title="插入计划/执行命令"
              >
                <Sparkles size={12} />
                <span>{modeLabel}</span>
                <ChevronDown size={10} />
              </button>
            </div>

            <div className="flex items-center gap-1">
              <div className="relative" ref={modelMenuRef}>
                <button
                  disabled={disabled}
                  onClick={() => {
                    if (disabled) {
                      return;
                    }
                    if (provider !== "deepseek" || !onModelChange) {
                      return;
                    }
                    setIsModelMenuOpen((value) => !value);
                  }}
                  className={`flex items-center gap-1 px-2 py-1 rounded-lg text-[12px] transition-colors ${
                    provider === "deepseek"
                      ? "text-text-secondary hover:bg-bg-card-hover"
                      : "text-text-muted cursor-default"
                  }`}
                >
                  <span>{modelLabel}</span>
                  {provider === "deepseek" && <ChevronDown size={10} />}
                </button>

                {provider === "deepseek" && onModelChange && isModelMenuOpen && (
                  <div className="absolute right-0 bottom-[calc(100%+8px)] min-w-[208px] rounded-xl border border-border-subtle bg-[#18181b]/95 backdrop-blur-md shadow-2xl overflow-hidden z-20">
                    {DEEPSEEK_MODEL_OPTIONS.map((option) => {
                      const selected = option.id === deepseekModelId;
                      return (
                        <button
                          key={option.id}
                          onClick={() => {
                            onModelChange(option.id);
                            setIsModelMenuOpen(false);
                          }}
                          className={`w-full px-3 py-2 text-left text-[12px] flex items-center justify-between gap-2 transition-colors ${
                            selected
                              ? "bg-bg-card text-text-primary"
                              : "text-text-secondary hover:bg-bg-card-hover"
                          }`}
                        >
                          <span>{option.label}</span>
                          {selected ? <Check size={12} className="text-accent" /> : null}
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>

              {isRunning ? (
                <button
                  type="button"
                  onClick={onStop}
                  disabled={!onStop}
                  className="w-7 h-7 rounded-full bg-danger flex items-center justify-center text-white hover:bg-danger/90 transition-colors ml-1 disabled:opacity-50 disabled:cursor-not-allowed"
                  title="中断当前轮"
                >
                  <Square size={13} strokeWidth={2.5} />
                </button>
              ) : (
                <button
                  onClick={onSubmit}
                  disabled={disabled || inputValue.trim().length === 0}
                  className="w-7 h-7 rounded-full bg-accent flex items-center justify-center text-white hover:bg-accent/90 transition-colors ml-1 disabled:opacity-50 disabled:cursor-not-allowed"
                  title="发送"
                >
                  <ArrowUp size={14} strokeWidth={2.5} />
                </button>
              )}
            </div>
          </div>

          {modelSwitchHint ? (
            <div className="px-3 pb-2 text-[11px] text-text-muted">{modelSwitchHint}</div>
          ) : null}
        </div>

        {!isCenterLayout && (
          <div className="flex items-center justify-between mt-2 px-1">
            <div className="flex items-center gap-1.5 text-[11px] text-text-muted">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><line x1="12" y1="8" x2="12" y2="16"/><line x1="8" y1="12" x2="16" y2="12"/></svg>
              本地模式
            </div>
            <div className="flex items-center gap-1.5 text-[11px] text-text-muted">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><line x1="6" y1="3" x2="6" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/></svg>
              {projectName}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
