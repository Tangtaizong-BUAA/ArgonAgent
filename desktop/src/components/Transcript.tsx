import { memo, useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  FileEdit,
  Loader2,
  RotateCcw,
  Wrench,
  Brain,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { AgentRun, TranscriptMessage } from "@/types";

const TRANSCRIPT_INITIAL_WINDOW_SIZE = 80;
const TRANSCRIPT_PAGE_SIZE = 80;

interface TranscriptProps {
  messages: TranscriptMessage[];
  currentRun: AgentRun;
  isStreaming?: boolean;
}

function normalizeEscapedTextForDisplay(value: string): string {
  if (!value) {
    return "";
  }
  let output = value;
  for (let index = 0; index < 4; index += 1) {
    const next = output
      .replace(/\\\\r\\\\n/g, "\n")
      .replace(/\\\\n/g, "\n")
      .replace(/\\\\t/g, "\t")
      .replace(/\\r\\n/g, "\n")
      .replace(/\\n/g, "\n")
      .replace(/\\t/g, "\t");
    if (next === output) {
      break;
    }
    output = next;
  }
  return output.replace(/\\+\n/g, "\n").replace(/\r\n/g, "\n");
}

function repairPseudoMarkdownNewlines(text: string): string {
  if (!text) {
    return "";
  }
  let output = text;
  output = output.replace(/(```[A-Za-z0-9_+.-]*)n(?=\S)/g, "$1\n");
  output = output.replace(/\b(import|from|def|class|return|assert|with|for|if|elif|else|try|except|finally)n(?=[A-Za-z_#@])/g, "$1\n");
  output = output.replace(/([。！？.!?；;：:）\]\}])nn(?=\S)/g, "$1\n\n");
  output = output.replace(/([。！？.!?；;：:）\]\}])n(?=(#{1,6}\s|---|[-*]\s|```|\d+\.\s))/g, "$1\n");
  output = output.replace(
    /(^|[^A-Za-z0-9_])nn(?=(#{1,6}\s|---|[-*]\s|```|\|(?:[^|]*\|)+|\d+\.\s))/g,
    "$1\n\n",
  );
  output = output.replace(/nn(?=\n)/g, "\n");
  return output.replace(/\n{3,}/g, "\n\n");
}

function findBalancedJsonObjectEnd(text: string, startIndex: number): number | null {
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = startIndex; index < text.length; index += 1) {
    const char = text[index];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (char === "\\") {
        escaped = true;
      } else if (char === '"') {
        inString = false;
      }
      continue;
    }
    if (char === '"') {
      inString = true;
      continue;
    }
    if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        return index + 1;
      }
    }
  }
  return null;
}

function stripToolCallsJsonSegments(text: string): string {
  let output = "";
  let cursor = 0;
  while (cursor < text.length) {
    const match = /\{\s*"tool_calls"\s*:/i.exec(text.slice(cursor));
    if (!match || match.index < 0) {
      output += text.slice(cursor);
      break;
    }
    const start = cursor + match.index;
    output += text.slice(cursor, start);
    const end = findBalancedJsonObjectEnd(text, start);
    if (end === null) {
      output += text.slice(start);
      break;
    }
    cursor = end;
  }
  return output;
}

function stripLeakedToolMarkup(content: string): string {
  if (!content) {
    return "";
  }
  let output = normalizeEscapedTextForDisplay(content);

  // Remove complete DSML tool_calls blocks first (paired tags)
  output = output.replace(
    /<[^>\n]{0,120}dsml[^>\n]{0,220}tool_calls[^>]*>[\s\S]*?<\/[^>\n]{0,120}dsml[^>\n]{0,220}tool_calls\s*>/gi,
    "",
  );
  // Remove any remaining DSML open/close tags (invoke, parameter, tool_calls)
  output = output.replace(
    /<\/?[^>\n]{0,120}dsml[^>\n]{0,220}(invoke|parameter|tool_calls)[^>]*>/gi,
    "",
  );
  // Remove non-DSML parameter tags that wrap tool arguments
  output = output.replace(/<\/?parameter[^>\n]{0,200}>/gi, "");
  output = output.replace(/<\/?invoke[^>\n]{0,200}>/gi, "");
  // Remove bare JSON tool_calls blobs without dropping visible text that follows
  // in the same chunk/line.
  output = stripToolCallsJsonSegments(output);

  // Remove residual DSML attribute-value fragments that remain after tag stripping.
  // Pattern: lines whose only content looks like XML attribute residue, e.g.
  //   n="offset"string="false">0n="limit"string="false">200nn
  // These have `attr="value">content` structure with no opening `<`.
  output = output.replace(
    /^[^\n<]*\w[\w]*\s*=\s*"[^"\n]*"[^\n<]*>[^\n<]{0,800}$/gim,
    "",
  );

  output = output
    .split("\n")
    .filter((line) => {
      const lowered = line.toLowerCase();
      if (
        /(?:^|\s)n?=\"?(?:file_read|file\.read|read|list_directory|file\.list_directory|search|bash|execute_command)\"?>/i.test(
          line,
        )
      ) {
        return false;
      }
      if (
        lowered.includes("string=\"true\"") &&
        (lowered.includes("list_directory") ||
          lowered.includes("file_read") ||
          lowered.includes("tool_calls") ||
          lowered.includes("parameter"))
      ) {
        return false;
      }
      if (
        lowered.includes(".researchcode/argon_agent/workspaces/projects/") &&
        (lowered.includes("list_directory") || lowered.includes("file_read") || lowered.includes("tool_calls"))
      ) {
        return false;
      }
      if (!lowered.includes("dsml")) {
        return true;
      }
      return !(
        lowered.includes("tool_calls") ||
        lowered.includes("invoke name") ||
        lowered.includes("parameter name")
      );
    })
    .join("\n");
  return repairPseudoMarkdownNewlines(output.replace(/\n{3,}/g, "\n\n")).trim();
}

function prepareStreamingMarkdown(content: string, isStreaming: boolean): string {
  if (!isStreaming || !content) {
    return content;
  }
  const fenceCount = content.match(/```/g)?.length ?? 0;
  if (fenceCount % 2 === 1) {
    return `${content}\n\`\`\``;
  }
  return content;
}

function StreamingMarkdownCursor() {
  return (
    <span
      aria-hidden="true"
      className="ml-1 inline-block h-4 w-1 translate-y-0.5 animate-pulse rounded-sm bg-accent/80"
    />
  );
}

const MARKDOWN_COMPONENTS = {
  p: ({ children }: { children?: ReactNode }) => (
    <p className="mb-3 last:mb-0 break-words [overflow-wrap:anywhere]">{children}</p>
  ),
  h1: ({ children }: { children?: ReactNode }) => (
    <h1 className="mb-3 mt-1 text-[20px] font-semibold text-text-primary break-words [overflow-wrap:anywhere]">
      {children}
    </h1>
  ),
  h2: ({ children }: { children?: ReactNode }) => (
    <h2 className="mb-3 mt-1 text-[18px] font-semibold text-text-primary break-words [overflow-wrap:anywhere]">
      {children}
    </h2>
  ),
  h3: ({ children }: { children?: ReactNode }) => (
    <h3 className="mb-2 mt-1 text-[16px] font-semibold text-text-primary break-words [overflow-wrap:anywhere]">
      {children}
    </h3>
  ),
  ul: ({ children }: { children?: ReactNode }) => <ul className="mb-3 list-disc pl-6 space-y-1">{children}</ul>,
  ol: ({ children }: { children?: ReactNode }) => <ol className="mb-3 list-decimal pl-6 space-y-1">{children}</ol>,
  li: ({ children }: { children?: ReactNode }) => (
    <li className="text-text-secondary break-words [overflow-wrap:anywhere]">{children}</li>
  ),
  a: ({ href, children }: { href?: string; children?: ReactNode }) => (
    <a href={href} target="_blank" rel="noreferrer" className="text-accent hover:underline">
      {children}
    </a>
  ),
  code: ({ className, children }: { className?: string; children?: ReactNode }) =>
    className?.includes("language-") ? (
      <code
        className={`block overflow-x-auto rounded-lg border border-border-subtle bg-[#131316] p-3 text-[12px] leading-6 text-text-primary ${className}`}
      >
        {children}
      </code>
    ) : (
      <code className="px-1.5 py-0.5 rounded bg-bg-card border border-border-subtle text-[12px] text-text-primary break-all">
        {children}
      </code>
    ),
  pre: ({ children }: { children?: ReactNode }) => <pre className="mb-3">{children}</pre>,
  blockquote: ({ children }: { children?: ReactNode }) => (
    <blockquote className="mb-3 border-l-2 border-border-subtle pl-3 text-text-muted">{children}</blockquote>
  ),
  table: ({ children }: { children?: ReactNode }) => (
    <div className="mb-3 overflow-x-auto">
      <table className="min-w-full border-collapse text-[13px]">{children}</table>
    </div>
  ),
  thead: ({ children }: { children?: ReactNode }) => <thead className="bg-bg-card">{children}</thead>,
  th: ({ children }: { children?: ReactNode }) => (
    <th className="border border-border-subtle px-2 py-1 text-left text-text-primary">{children}</th>
  ),
  td: ({ children }: { children?: ReactNode }) => (
    <td className="border border-border-subtle px-2 py-1 text-text-secondary align-top">{children}</td>
  ),
  hr: () => <hr className="my-4 border-border-subtle" />,
};

interface ToolActivityEvent {
  id: string;
  toolId: string;
  toolCallId?: string;
  phase: "requested" | "completed" | "failed";
  content: string;
  timestamp: string;
}

interface ToolActivityGroup {
  id: string;
  events: ToolActivityEvent[];
}

type TranscriptRenderItem =
  | { kind: "message"; message: TranscriptMessage }
  | { kind: "tool_group"; group: ToolActivityGroup };

export function Transcript({ messages, currentRun, isStreaming = false }: TranscriptProps) {
  const [visibleLimit, setVisibleLimit] = useState(TRANSCRIPT_INITIAL_WINDOW_SIZE);
  const allItems = useMemo(() => buildTranscriptItems(messages), [messages]);
  const hiddenCount = Math.max(0, allItems.length - visibleLimit);
  const items = useMemo(() => allItems.slice(-visibleLimit), [allItems, visibleLimit]);

  return (
    <div className="flex flex-col px-8 py-5 max-w-[900px] mx-auto">
      {hiddenCount > 0 && (
        <div className="mb-4 flex justify-center">
          <button
            type="button"
            onClick={() => setVisibleLimit((current) => Math.min(allItems.length, current + TRANSCRIPT_PAGE_SIZE))}
            className="rounded-md border border-border-subtle bg-bg-card px-3 py-1.5 text-[12px] text-text-muted transition-colors hover:bg-bg-card-hover hover:text-text-secondary"
          >
            显示较早的 {hiddenCount} 条记录
          </button>
        </div>
      )}
      {items.map((item) => {
        if (item.kind === "message") {
          return <MessageBubble key={item.message.id} message={item.message} />;
        }
        return <ToolActivityBubble key={item.group.id} group={item.group} />;
      })}

      {isStreaming && (
        <div className="mb-4 flex items-center gap-2 text-[12px] text-text-muted">
          <Loader2 size={12} className="animate-spin" />
          <span>模型正在流式输出</span>
        </div>
      )}

      {currentRun.changes.length > 0 && (
        <div className="mt-2 mb-3">
          <ChangeSummaryBar changes={currentRun.changes} />
        </div>
      )}
    </div>
  );
}

function buildTranscriptItems(messages: TranscriptMessage[]): TranscriptRenderItem[] {
  const items: TranscriptRenderItem[] = [];
  let bufferedToolMessages: TranscriptMessage[] = [];

  const flushToolBuffer = () => {
    if (bufferedToolMessages.length === 0) {
      return;
    }
    const seen = new Set<string>();
    const events = bufferedToolMessages
      .map(toToolActivityEvent)
      .filter((event) => {
        const key = `${event.toolCallId ?? "none"}|${event.phase}|${event.toolId}`;
        if (seen.has(key)) {
          return false;
        }
        seen.add(key);
        return true;
      });
    const firstId = bufferedToolMessages[0]?.id ?? `tool_group_${Date.now()}`;
    items.push({
      kind: "tool_group",
      group: {
        id: `tool_group_${firstId}`,
        events,
      },
    });
    bufferedToolMessages = [];
  };

  for (const message of messages) {
    if (message.type === "tool_call" && message.role === "agent") {
      bufferedToolMessages.push(message);
      continue;
    }
    flushToolBuffer();
    items.push({ kind: "message", message });
  }
  flushToolBuffer();
  return items;
}

function toToolActivityEvent(message: TranscriptMessage): ToolActivityEvent {
  const metadata = (message.metadata ?? {}) as Record<string, unknown>;
  const metadataToolId = String(metadata.tool_id ?? "").trim();
  const metadataToolCallId = String(metadata.tool_call_id ?? "").trim();
  const metadataPhaseRaw = String(metadata.tool_phase ?? "").trim().toLowerCase();
  if (metadataToolId && (metadataPhaseRaw === "requested" || metadataPhaseRaw === "completed" || metadataPhaseRaw === "failed")) {
    return {
      id: message.id,
      toolId: metadataToolId,
      toolCallId: metadataToolCallId || undefined,
      phase: metadataPhaseRaw as "requested" | "completed" | "failed",
      content: message.content,
      timestamp: message.timestamp,
    };
  }
  const content = message.content.trim();
  const requestedMatch = /^调用工具:\s*(.+)$/u.exec(content);
  if (requestedMatch) {
    return {
      id: message.id,
      toolId: requestedMatch[1].trim(),
      toolCallId: metadataToolCallId || undefined,
      phase: "requested",
      content: message.content,
      timestamp: message.timestamp,
    };
  }
  const completedMatch = /^工具完成:\s*(.+)$/u.exec(content);
  if (completedMatch) {
    return {
      id: message.id,
      toolId: completedMatch[1].trim(),
      toolCallId: metadataToolCallId || undefined,
      phase: "completed",
      content: message.content,
      timestamp: message.timestamp,
    };
  }
  const failedMatch = /^工具失败:\s*(.+)$/u.exec(content);
  if (failedMatch) {
    return {
      id: message.id,
      toolId: failedMatch[1].trim(),
      toolCallId: metadataToolCallId || undefined,
      phase: "failed",
      content: message.content,
      timestamp: message.timestamp,
    };
  }
  return {
    id: message.id,
    toolId: "unknown",
    toolCallId: metadataToolCallId || undefined,
    phase: content.includes("失败") ? "failed" : content.includes("完成") ? "completed" : "requested",
    content: message.content,
    timestamp: message.timestamp,
  };
}

function toolVerb(toolId: string): string {
  const value = toolId.toLowerCase();
  if (value.includes("file.read")) return "读取";
  if (value.includes("search.ripgrep")) return "搜索";
  if (value.includes("file.list")) return "浏览";
  if (
    value.includes("file.edit") ||
    value.includes("file.write") ||
    value.includes("file.multi_edit") ||
    value.includes("patch.apply")
  ) {
    return "编辑";
  }
  if (value.includes("git.")) return "检查";
  return "执行";
}

function classifyToolForSummary(toolId: string): "edit" | "explore" | "search" | "command" | "other" {
  const value = toolId.toLowerCase();
  if (
    value.includes("file.edit") ||
    value.includes("file.write") ||
    value.includes("file.multi_edit") ||
    value.includes("patch.apply")
  ) {
    return "edit";
  }
  if (
    value.includes("file.read") ||
    value.includes("file.list") ||
    value.includes("repo.map") ||
    value.includes("lsp.diagnostics")
  ) {
    return "explore";
  }
  if (value.includes("search.ripgrep")) {
    return "search";
  }
  if (value.includes("shell.command") || value.startsWith("git.")) {
    return "command";
  }
  return "other";
}

const ToolActivityBubble = memo(function ToolActivityBubble({ group }: { group: ToolActivityGroup }) {
  const [expanded, setExpanded] = useState(false);

  const stats = useMemo(() => {
    let requested = 0;
    let completed = 0;
    let failed = 0;
    const pendingToolCallIds: string[] = [];
    const pendingToolNames: string[] = [];
    const summary = {
      edit: 0,
      explore: 0,
      search: 0,
      command: 0,
      other: 0,
    };

    for (const event of group.events) {
      if (event.phase === "requested") {
        requested += 1;
        if (event.toolCallId) {
          pendingToolCallIds.push(event.toolCallId);
        }
        pendingToolNames.push(event.toolId);
        continue;
      }
      if (event.phase === "completed") {
        completed += 1;
      } else if (event.phase === "failed") {
        failed += 1;
      }
      summary[classifyToolForSummary(event.toolId)] += 1;
      if (event.toolCallId) {
        const idIndex = pendingToolCallIds.indexOf(event.toolCallId);
        if (idIndex >= 0) {
          pendingToolCallIds.splice(idIndex, 1);
          pendingToolNames.splice(idIndex, 1);
          continue;
        }
      }
      const exact = pendingToolNames.indexOf(event.toolId);
      if (exact >= 0) {
        pendingToolNames.splice(exact, 1);
      } else if (pendingToolNames.length > 0) {
        pendingToolNames.shift();
      }
    }

    const running = pendingToolNames.length;
    const currentTool = pendingToolNames[pendingToolNames.length - 1] ?? group.events[group.events.length - 1]?.toolId ?? "tool";
    return { requested, completed, failed, running, currentTool, summary };
  }, [group.events]);

  const isRunning = stats.running > 0;
  const header = isRunning
    ? `正在${toolVerb(stats.currentTool)} ${stats.currentTool}`
    : `已完成 ${stats.completed} 个任务${stats.failed > 0 ? `，失败 ${stats.failed}` : ""}`;
  const detail = isRunning
    ? `完成 ${stats.completed}/${Math.max(stats.requested, stats.completed + stats.running + stats.failed)}`
    : `共 ${Math.max(stats.requested, stats.completed + stats.failed)} 次工具调用`;
  const summaryParts: string[] = [];
  if (stats.summary.edit > 0) {
    summaryParts.push(`已编辑 ${stats.summary.edit} 个文件`);
  }
  if (stats.summary.explore > 0) {
    summaryParts.push(`已探索 ${stats.summary.explore} 个文件`);
  }
  if (stats.summary.search > 0) {
    summaryParts.push(`${stats.summary.search} 次搜索`);
  }
  if (stats.summary.command > 0) {
    summaryParts.push(`${stats.summary.command} 次命令`);
  }
  const activitySummary = summaryParts.join("，");

  return (
    <div className="mb-4">
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        className="w-full max-w-[720px] rounded-xl border border-border-subtle bg-bg-card px-4 py-3 text-left hover:bg-bg-card-hover transition-colors"
      >
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2 min-w-0">
            {isRunning ? (
              <Loader2 size={14} className="text-warning animate-spin shrink-0" />
            ) : stats.failed > 0 ? (
              <AlertCircle size={14} className="text-danger shrink-0" />
            ) : (
              <CheckCircle2 size={14} className="text-success shrink-0" />
            )}
            <div className="min-w-0">
              <div className="text-[14px] text-text-primary truncate">{header}</div>
              {activitySummary && (
                <div className="text-[12px] text-text-muted truncate mt-0.5">{activitySummary}</div>
              )}
            </div>
          </div>
          <div className="flex items-center gap-1 text-[12px] text-text-muted shrink-0">
            <span>{detail}</span>
            {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </div>
        </div>
      </button>

      {expanded && (
        <div className="mt-2 ml-1 rounded-lg border border-border-subtle/70 bg-[#17171a] px-3 py-2 space-y-1.5">
          {group.events.map((event) => (
            <div key={event.id} className="flex items-center gap-2 text-[12px] text-text-muted">
              {event.phase === "requested" ? (
                <Wrench size={12} className="text-warning shrink-0" />
              ) : event.phase === "completed" ? (
                <CheckCircle2 size={12} className="text-success shrink-0" />
              ) : (
                <AlertCircle size={12} className="text-danger shrink-0" />
              )}
              <span className="truncate">{event.content}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
});

const MessageBubble = memo(function MessageBubble({ message }: { message: TranscriptMessage }) {
  if (message.role === "user") {
    return (
      <div className="flex justify-end mb-5">
        <div className="bg-bg-card border border-border-subtle rounded-2xl rounded-tr-sm px-4 py-2.5 max-w-[80%]">
          <p className="text-[14px] text-text-primary leading-relaxed">{message.content}</p>
        </div>
      </div>
    );
  }

  if (message.role === "system" && message.type === "approval") {
    return (
      <div className="flex items-center gap-2 mb-4 px-1">
        <CheckCircle2 size={14} className="text-success shrink-0" />
        <span className="text-[12px] text-text-muted">{message.content}</span>
      </div>
    );
  }

  if (message.role === "system") {
    return (
      <div className="mb-4">
        <div className="inline-flex items-center gap-2 px-3 py-1.5 rounded-lg bg-bg-card border border-border-subtle text-[12px] text-text-muted">
          <RotateCcw size={12} />
          <span>{message.content}</span>
        </div>
      </div>
    );
  }

  if (message.type === "thinking") {
    const meta = (message.metadata ?? {}) as Record<string, unknown>;
    const streaming = meta.streaming !== false;
    const durationMs = Number(meta.duration_ms ?? 0);
    const tokenEstimate = Number(meta.token_estimate ?? 0);
    return <ThinkingBlock content={message.content} streaming={streaming} durationMs={durationMs} tokenEstimate={tokenEstimate} />;
  }

  if (message.type === "file_edit") {
    return (
      <div className="mb-3">
        <div className="inline-flex items-center gap-2 px-3 py-1.5 rounded-lg bg-bg-card border border-border-subtle text-[12px] text-text-muted hover:bg-bg-card-hover transition-colors cursor-default">
          <FileEdit size={12} />
          <span>{message.content}</span>
        </div>
      </div>
    );
  }

  // Default agent text
  const safeContent =
    message.role === "agent"
      ? stripLeakedToolMarkup(message.content)
      : repairPseudoMarkdownNewlines(normalizeEscapedTextForDisplay(message.content));
  const isLiveStream = message.role === "agent" && message.metadata?.streaming === true;
  if (message.role === "agent" && !safeContent.trim()) {
    return null;
  }
  return (
    <div className="mb-5">
      <div className="text-[14px] text-text-secondary leading-[1.75] break-words [overflow-wrap:anywhere]">
        {message.role === "agent" ? (
          <>
            <ReactMarkdown remarkPlugins={[remarkGfm]} components={MARKDOWN_COMPONENTS}>
              {prepareStreamingMarkdown(safeContent, isLiveStream)}
            </ReactMarkdown>
            {isLiveStream && <StreamingMarkdownCursor />}
          </>
        ) : (
          <div className="whitespace-pre-wrap">{safeContent}</div>
        )}
      </div>
    </div>
  );
});

function ChangeSummaryBar({
  changes,
}: {
  changes: { path: string; additions: number; deletions: number; status: string }[];
}) {
  const totalAdditions = changes.reduce((s, c) => s + c.additions, 0);
  const totalDeletions = changes.reduce((s, c) => s + c.deletions, 0);

  return (
    <div className="bg-bg-card border border-border-subtle rounded-xl overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2.5 border-b border-border-subtle">
        <div className="flex items-center gap-2.5 text-[13px]">
          <span className="text-text-secondary">{changes.length} 个文件已更改</span>
          <span className="text-diff-add font-medium">+{totalAdditions}</span>
          <span className="text-diff-remove font-medium">-{totalDeletions}</span>
        </div>
      </div>
      {/* File list */}
      <div className="px-4 py-2 space-y-1">
        {changes.map((change, i) => (
          <div key={i} className="flex items-center justify-between text-[12px]">
            <span className="text-text-secondary truncate max-w-[420px] font-mono">{change.path}</span>
            <span className="flex items-center gap-2.5 shrink-0 text-text-muted tabular-nums">
              <span className="text-diff-add">+{change.additions}</span>
              <span className="text-diff-remove">-{change.deletions}</span>
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── ThinkingBlock ─────────────────────────────────────────────────────────────
// Mirrors DeepSeek-TUI render_thinking():
//   • "…" opener + status label in header
//   • Dashed rail (╎) prefix on each body line (italic, warm tint)
//   • Streaming cursor (▎) on the last body line while generating
//   • Collapsible body — collapsed after stream completes (Ctrl+O to expand)
//   • Duration and token count in completed header

const THINKING_OPENER = "…";
const THINKING_RAIL = "╎ ";
const THINKING_CURSOR = "▎";
const THINKING_SUMMARY_LINE_LIMIT = 3;

interface ThinkingBlockProps {
  content: string;
  streaming: boolean;
  durationMs: number;
  tokenEstimate: number;
}

function ThinkingBlock({ content, streaming, durationMs, tokenEstimate }: ThinkingBlockProps) {
  const [expanded, setExpanded] = useState(false);
  const showFull = expanded || streaming;
  const lines = content.split("\n").filter((l) => l.trim() !== "");
  const previewLines = showFull ? lines : lines.slice(0, THINKING_SUMMARY_LINE_LIMIT);
  const truncated = !showFull && lines.length > THINKING_SUMMARY_LINE_LIMIT;

  return (
    <div className="mb-4 max-w-[720px]">
      {/* Header */}
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="flex items-center gap-2 mb-1 group"
        title={streaming ? "思维进行中…" : expanded ? "折叠思维过程" : "展开思维过程"}
      >
        {streaming ? (
          <Loader2 size={13} className="text-violet-400 animate-spin shrink-0" />
        ) : (
          <Brain size={13} className="text-violet-400 shrink-0" />
        )}
        <span className="text-[12px] text-violet-400 font-medium">
          {THINKING_OPENER} thinking
        </span>
        <span className="text-[12px] text-text-muted ml-1">
          {streaming
            ? "reasoning in progress..."
            : `done · ${(durationMs / 1000).toFixed(1)}s · ~${tokenEstimate} tokens`}
        </span>
        {!streaming && (
          <span className="text-[11px] text-text-muted opacity-60 group-hover:opacity-100 ml-1">
            {expanded ? "▾" : "▸"}
          </span>
        )}
      </button>

      {/* Body — dashed rail lines */}
      {(streaming || content.trim()) && (
        <div className="pl-1 space-y-0">
          {streaming && previewLines.length === 0 ? (
            <div className="flex items-start gap-1">
              <span className="text-violet-600/60 text-[13px] font-mono shrink-0 select-none">
                {THINKING_RAIL}
              </span>
              <span className="text-[12px] text-text-muted italic">
                reasoning in progress... {THINKING_CURSOR}
              </span>
            </div>
          ) : (
            previewLines.map((line, idx) => {
              const isLast = idx === previewLines.length - 1;
              return (
                <div key={idx} className="flex items-start gap-1">
                  <span className="text-violet-600/60 text-[13px] font-mono shrink-0 select-none leading-5">
                    {THINKING_RAIL}
                  </span>
                  <span className="text-[12px] text-text-muted italic leading-5 break-words [overflow-wrap:anywhere]">
                    {line}
                    {streaming && isLast && (
                      <span className="text-violet-400 not-italic ml-0.5">{THINKING_CURSOR}</span>
                    )}
                  </span>
                </div>
              );
            })
          )}
          {truncated && (
            <div className="flex items-start gap-1">
              <span className="text-violet-600/60 text-[13px] font-mono shrink-0 select-none leading-5">
                {THINKING_RAIL}
              </span>
              <button
                type="button"
                onClick={() => setExpanded(true)}
                className="text-[11px] text-text-muted italic hover:text-violet-400 transition-colors leading-5"
              >
                thinking collapsed · click to expand full reasoning
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
