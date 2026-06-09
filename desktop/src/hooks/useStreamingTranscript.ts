import { useCallback, useRef, type Dispatch, type SetStateAction } from "react";
import { nowIso } from "@/runtime/runStore";
import { isDuplicateAgentText } from "@/runtime/transcriptDedupe";
import {
  TOOL_MARKUP_END_PATTERNS,
  TOOL_MARKUP_START_PATTERNS,
  extractTrailingPotentialMarkup,
  findFirstPattern,
  normalizeStreamChunk,
  repairPseudoMarkdownNewlines,
  resolveFinalStreamingContent,
  stripLeadingToolCallsJson,
  stripLeakedToolMarkup,
  trimLeadingBlankLines,
} from "@/runtime/streamSanitizer";
import type { TranscriptMessage } from "@/types";

interface UseStreamingTranscriptOptions {
  setMessages: Dispatch<SetStateAction<TranscriptMessage[]>>;
  setIsStreaming: Dispatch<SetStateAction<boolean>>;
}

function makeMessageId(prefix: string): string {
  return `${prefix}_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
}

function compactAgentTextPrefix(value: string): string {
  return repairPseudoMarkdownNewlines(normalizeStreamChunk(value))
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 200);
}

function looksLikePseudoNewlineDamage(value: string): boolean {
  return (
    /```[A-Za-z0-9_+.-]*n(?=\S)/.test(value) ||
    /[。！？.!?；;：:）\]\}]n(?=(#{1,6}\s|---|[-*]\s|```|\d+\.\s|\S))/.test(value) ||
    /\b(import|from|def|class|return|assert|with|for|if|elif|else|try|except|finally)n(?=[A-Za-z_#@])/.test(value)
  );
}

function shouldReplaceWithAuthoritativeAgentText(previousContent: string, nextContent: string, wasStreaming: boolean): boolean {
  if (wasStreaming) {
    return true;
  }
  if (!looksLikePseudoNewlineDamage(previousContent)) {
    return false;
  }
  const previous = compactAgentTextPrefix(previousContent);
  const next = compactAgentTextPrefix(nextContent);
  if (previous.length < 40 || next.length < 40) {
    return false;
  }
  return previous.slice(0, 80) === next.slice(0, 80);
}

export function useStreamingTranscript({
  setMessages,
  setIsStreaming,
}: UseStreamingTranscriptOptions) {
  const activeStreamMessageIdRef = useRef<string | null>(null);
  const activeStreamBufferRef = useRef("");
  const streamFlushFrameRef = useRef<number | null>(null);
  const streamToolMarkupStateRef = useRef<{ insideMarkup: boolean; carry: string }>({
    insideMarkup: false,
    carry: "",
  });
  const callCompletedSettleTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const appendMessage = useCallback((message: Omit<TranscriptMessage, "id" | "timestamp">) => {
    setMessages((prev) => {
      if (message.role === "agent" && message.type === "text") {
        const cleanContent = repairPseudoMarkdownNewlines(normalizeStreamChunk(message.content));
        const lastAgentIndex = [...prev]
          .reverse()
          .findIndex((item) => item.role === "agent" && item.type === "text");
        const absoluteLastAgentIndex = lastAgentIndex >= 0 ? prev.length - 1 - lastAgentIndex : -1;
        const lastAgent = absoluteLastAgentIndex >= 0 ? prev[absoluteLastAgentIndex] : undefined;
        if (lastAgent?.role === "agent" && lastAgent.type === "text") {
          const shouldReplace = shouldReplaceWithAuthoritativeAgentText(
            lastAgent.content,
            cleanContent,
            lastAgent.metadata?.streaming === true,
          );
          if (shouldReplace || isDuplicateAgentText(lastAgent.content, cleanContent)) {
            if (lastAgent.content === cleanContent && !lastAgent.metadata?.streaming) {
              return prev;
            }
            return prev.map((item, index) =>
              index === absoluteLastAgentIndex
                ? {
                    ...item,
                    content: cleanContent,
                    metadata: undefined,
                    timestamp: item.timestamp,
                  }
                : item,
            );
          }
        }
        if (isDuplicateAgentText(lastAgent?.content, cleanContent)) {
          return prev;
        }
      }
      return [
        ...prev,
        {
          id: makeMessageId("msg"),
          timestamp: nowIso(),
          ...message,
          content:
            message.role === "agent" && message.type === "text"
              ? repairPseudoMarkdownNewlines(normalizeStreamChunk(message.content))
              : message.content,
        },
      ];
    });
  }, [setMessages]);

  const clearStreamingState = useCallback(() => {
    if (callCompletedSettleTimerRef.current) {
      clearTimeout(callCompletedSettleTimerRef.current);
      callCompletedSettleTimerRef.current = null;
    }
    if (streamFlushFrameRef.current !== null) {
      cancelAnimationFrame(streamFlushFrameRef.current);
      streamFlushFrameRef.current = null;
    }
    activeStreamMessageIdRef.current = null;
    activeStreamBufferRef.current = "";
    streamToolMarkupStateRef.current.insideMarkup = false;
    streamToolMarkupStateRef.current.carry = "";
    setIsStreaming(false);
  }, [setIsStreaming]);

  const discardActiveStreamingMessage = useCallback(() => {
    const activeId = activeStreamMessageIdRef.current;
    clearStreamingState();
    if (!activeId) {
      return;
    }
    setMessages((prev) => prev.filter((item) => item.id !== activeId));
  }, [clearStreamingState, setMessages]);

  const applyStreamChunk = useCallback((chunk: string) => {
    if (!chunk) {
      return;
    }
    const activeId = activeStreamMessageIdRef.current;
    if (!activeId) {
      const id = makeMessageId("stream");
      activeStreamMessageIdRef.current = id;
      activeStreamBufferRef.current = chunk;
      setMessages((prev) => [
        ...prev,
        {
          id,
          role: "agent",
          type: "text",
          content: chunk,
          metadata: { streaming: true },
          timestamp: nowIso(),
        },
      ]);
      setIsStreaming(true);
      return;
    }
    activeStreamBufferRef.current += chunk;
    if (streamFlushFrameRef.current === null) {
      streamFlushFrameRef.current = requestAnimationFrame(() => {
        streamFlushFrameRef.current = null;
        const targetId = activeStreamMessageIdRef.current;
        if (!targetId) {
          return;
        }
        const buffered = activeStreamBufferRef.current;
        setMessages((prev) =>
          prev.map((item) =>
            item.id === targetId && item.content !== buffered ? { ...item, content: buffered } : item,
          ),
        );
      });
    }
    setIsStreaming(true);
  }, [setIsStreaming, setMessages]);

  const sanitizeStreamChunk = useCallback((rawChunk: string) => {
    const state = streamToolMarkupStateRef.current;
    let text = normalizeStreamChunk(`${state.carry}${rawChunk}`);
    state.carry = "";
    if (!text) {
      return "";
    }
    let visible = "";
    while (text.length > 0) {
      if (state.insideMarkup) {
        const endHit = findFirstPattern(text, TOOL_MARKUP_END_PATTERNS);
        if (!endHit) {
          state.carry = text;
          return visible;
        }
        text = text.slice(endHit.index + endHit.value.length);
        state.insideMarkup = false;
        continue;
      }

      const startHit = findFirstPattern(text, TOOL_MARKUP_START_PATTERNS);
      if (!startHit) {
        visible += text;
        text = "";
        break;
      }
      visible += text.slice(0, startHit.index);
      text = text.slice(startHit.index);
      if (/^\{\s*"tool_calls"\s*:/i.test(text)) {
        const stripped = stripLeadingToolCallsJson(text);
        if (stripped.status === "complete") {
          text = stripped.text;
          continue;
        }
        state.carry = text;
        return visible;
      }
      state.insideMarkup = true;
    }
    const { stable, trailing } = extractTrailingPotentialMarkup(visible);
    state.carry = trailing;
    return trimLeadingBlankLines(repairPseudoMarkdownNewlines(stripLeakedToolMarkup(stable)));
  }, []);

  const commitStreamingMessage = useCallback((finalContent: string) => {
    const cleanFinalContent = repairPseudoMarkdownNewlines(sanitizeStreamChunk(finalContent)).trim();
    const activeId = activeStreamMessageIdRef.current;
    if (!activeId) {
      if (cleanFinalContent) {
        setMessages((prev) => {
          const lastAgentText = [...prev]
            .reverse()
            .find((item) => item.role === "agent" && item.type === "text")?.content;
          if (isDuplicateAgentText(lastAgentText, cleanFinalContent)) {
            return prev;
          }
          return [
            ...prev,
            {
              id: makeMessageId("msg"),
              role: "agent",
              type: "text",
              content: cleanFinalContent,
              timestamp: nowIso(),
            },
          ];
        });
      }
      clearStreamingState();
      return;
    }

    const streamed = activeStreamBufferRef.current.trim();
    setMessages((prev) => {
      const resolved = resolveFinalStreamingContent(streamed, cleanFinalContent);
      return prev.map((item) =>
        item.id === activeId
          ? {
              ...item,
              content: resolved || item.content,
              metadata: undefined,
            }
          : item,
      );
    });
    clearStreamingState();
  }, [clearStreamingState, sanitizeStreamChunk, setMessages]);

  return {
    appendMessage,
    clearStreamingState,
    discardActiveStreamingMessage,
    applyStreamChunk,
    sanitizeStreamChunk,
    commitStreamingMessage,
    callCompletedSettleTimerRef,
    streamToolMarkupStateRef,
  };
}
