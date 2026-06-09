import { useCallback, type Dispatch, type SetStateAction } from "react";
import {
  exportRuntimeEvents,
  getRuntimeSnapshot,
  interruptRuntimeSession,
  revealRuntimePath,
  startRuntimeSession,
  submitRuntimeUserMessage,
  type RuntimeStartSessionResult,
} from "@/runtime/localRuntimeClient";
import type { RuntimeBootstrap } from "@/types/runtime";
import type { RunStatus, TranscriptMessage } from "@/types";

interface RuntimeActionConfig {
  provider: "deepseek" | "qwen";
}

type AppendMessage = (message: Omit<TranscriptMessage, "id" | "timestamp">) => void;

interface SubmitPromptOptions {
  appendUser?: boolean;
  updateRunTitle?: boolean;
}

interface UseRuntimeSessionActionsOptions {
  autonomyMode: string;
  bootstrap: RuntimeBootstrap | null;
  config: RuntimeActionConfig;
  inputValue: string;
  messages: TranscriptMessage[];
  runTitle: string;
  selectedSourceWorkspaceRoot: string;
  sessionId: string | null;
  appendMessage: AppendMessage;
  ensureRuntimeSubscription: (bootstrap: RuntimeBootstrap, sessionId: string) => Promise<void>;
  onSessionStarted: (session: RuntimeStartSessionResult) => void;
  pollRuntimeEvents: (sessionId: string) => Promise<void>;
  setInputValue: Dispatch<SetStateAction<string>>;
  setIsStreaming: Dispatch<SetStateAction<boolean>>;
  setRunStatus: Dispatch<SetStateAction<RunStatus>>;
  setRunTitle: Dispatch<SetStateAction<string>>;
  setRuntimeError: Dispatch<SetStateAction<string | null>>;
}

const DEFAULT_CONTINUE_PROMPT = "Continue the current session using prior context.";
const CONTINUE_PROMPT_LABEL_ZH = "基于上一轮上下文继续";

async function withTimeout<T>(promise: Promise<T>, timeoutMs: number, label: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const timeoutPromise = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error(`${label} timeout after ${timeoutMs}ms`));
    }, timeoutMs);
  });
  try {
    return await Promise.race([promise, timeoutPromise]);
  } finally {
    if (timer) {
      clearTimeout(timer);
    }
  }
}

async function waitForRuntimeTurnReady(
  runtime: RuntimeBootstrap,
  sessionId: string,
  timeoutMs = 3_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const snapshot = await getRuntimeSnapshot(runtime, { session_id: sessionId });
    if (runtimeStateAllowsNewTurn(snapshot.state)) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
}

function runtimeStateAllowsNewTurn(state: unknown): boolean {
  const normalized = String(state ?? "").toLowerCase();
  return (
    normalized.includes("cancel") ||
    normalized.includes("stop") ||
    normalized.includes("complete") ||
    normalized.includes("fail") ||
    normalized.includes("idle") ||
    normalized.includes("waitingforuser") ||
    normalized.includes("waiting_for_user")
  );
}

export function useRuntimeSessionActions({
  autonomyMode,
  bootstrap,
  config,
  inputValue,
  messages,
  runTitle,
  selectedSourceWorkspaceRoot,
  sessionId,
  appendMessage,
  ensureRuntimeSubscription,
  onSessionStarted,
  pollRuntimeEvents,
  setInputValue,
  setIsStreaming,
  setRunStatus,
  setRunTitle,
  setRuntimeError,
}: UseRuntimeSessionActionsOptions) {
  const ensureSession = useCallback(async () => {
    if (!bootstrap) {
      throw new Error("Runtime bootstrap not ready");
    }
    if (sessionId) {
      return sessionId;
    }
    const session = await startRuntimeSession(bootstrap, {
      workspace: selectedSourceWorkspaceRoot || bootstrap.workspaceRoot,
      model_mode: config.provider,
      autonomy_mode: autonomyMode,
    });
    onSessionStarted(session);
    return session.session_id;
  }, [autonomyMode, bootstrap, config.provider, onSessionStarted, selectedSourceWorkspaceRoot, sessionId]);

  const submitPrompt = useCallback(async (
    rawText: string,
    options?: SubmitPromptOptions,
  ): Promise<boolean> => {
    const text = rawText.trim();
    if (!text) {
      return false;
    }
    const appendUser = options?.appendUser ?? true;
    const updateRunTitle = options?.updateRunTitle ?? false;
    if (appendUser) {
      appendMessage({
        role: "user",
        type: "text",
        content: text,
      });
    }
    if (updateRunTitle && runTitle === "新会话") {
      setRunTitle(text.slice(0, 40));
    }
    setRunStatus("running");
    const runtime = bootstrap;
    if (!runtime) {
      appendMessage({
        role: "system",
        type: "text",
        content: "Runtime 未就绪，请稍后重试。",
      });
      setRunStatus("failed");
      return false;
    }
    try {
      const currentSessionId = await ensureSession();
      await ensureRuntimeSubscription(runtime, currentSessionId);
      const result = await withTimeout(
        submitRuntimeUserMessage(runtime, {
          session_id: currentSessionId,
          text,
        }),
        10_000,
        "runtime.submit_user_message",
      );
      if (!result.ok && result.error_code) {
        if (result.error_code === "runtime_turn_in_progress") {
          await pollRuntimeEvents(currentSessionId);
          const snapshot = await getRuntimeSnapshot(runtime, { session_id: currentSessionId });
          if (runtimeStateAllowsNewTurn(snapshot.state)) {
            const retryResult = await withTimeout(
              submitRuntimeUserMessage(runtime, {
                session_id: currentSessionId,
                text,
              }),
              10_000,
              "runtime.submit_user_message.followup_after_stale_busy",
            );
            if (retryResult.ok) {
              await pollRuntimeEvents(currentSessionId);
              return true;
            }
          }
          appendMessage({
            role: "system",
            type: "text",
            content: "上一轮仍在执行，正在中断并提交你的跟进。",
          });
          try {
            const interruptResult = await withTimeout(
              interruptRuntimeSession(runtime, { session_id: currentSessionId }),
              5_000,
              "runtime.interrupt_session",
            );
            if (interruptResult.ok) {
              setRunStatus("stopped");
              setIsStreaming(false);
              await waitForRuntimeTurnReady(runtime, currentSessionId);
              setRunStatus("running");
              const retryResult = await withTimeout(
                submitRuntimeUserMessage(runtime, {
                  session_id: currentSessionId,
                  text,
                }),
                10_000,
                "runtime.submit_user_message.followup",
              );
              if (retryResult.ok) {
                await pollRuntimeEvents(currentSessionId);
                return true;
              }
              appendMessage({
                role: "system",
                type: "text",
                content: `跟进提交失败: ${retryResult.error_code ?? "unknown"}`,
              });
              return false;
            }
          } catch (interruptError) {
            const interruptMessage = String((interruptError as Error).message ?? interruptError);
            appendMessage({
              role: "system",
              type: "text",
              content: `中断并提交跟进失败: ${interruptMessage}`,
            });
            return false;
          }
        }
        const errorMessage =
          result.error_code === "runtime_turn_in_progress"
            ? "上一轮仍在收尾，已刷新运行状态；如果已经结束，请再发送一次。"
            : `调用失败: ${result.error_code}`;
        appendMessage({
          role: "system",
          type: "text",
          content: errorMessage,
        });
        return false;
      }
      await pollRuntimeEvents(currentSessionId);
      return true;
    } catch (error) {
      const message = String((error as Error).message ?? error);
      setRuntimeError(message);
      appendMessage({
        role: "system",
        type: "text",
        content: `提交消息失败: ${message}`,
      });
      setRunStatus("failed");
      return false;
    }
  }, [
    appendMessage,
    bootstrap,
    ensureRuntimeSubscription,
    ensureSession,
    pollRuntimeEvents,
    runTitle,
    setIsStreaming,
    setRunStatus,
    setRunTitle,
    setRuntimeError,
  ]);

  const handleSubmit = useCallback(async () => {
    const text = inputValue.trim();
    if (!text) {
      return;
    }
    const ok = await submitPrompt(text, { appendUser: true, updateRunTitle: true });
    if (ok) {
      setInputValue("");
    } else {
      setInputValue(text);
    }
  }, [inputValue, setInputValue, submitPrompt]);

  const handleStopRun = useCallback(async () => {
    if (!bootstrap || !sessionId) {
      return;
    }
    try {
      const result = await interruptRuntimeSession(bootstrap, { session_id: sessionId });
      if (result.ok) {
        setRunStatus("stopped");
        setIsStreaming(false);
        appendMessage({
          role: "system",
          type: "text",
          content: "已请求中断当前轮。",
        });
        await pollRuntimeEvents(sessionId);
      }
    } catch (error) {
      const message = String((error as Error).message ?? error);
      setRuntimeError(message);
      appendMessage({
        role: "system",
        type: "text",
        content: `中断失败: ${message}`,
      });
    }
  }, [appendMessage, bootstrap, pollRuntimeEvents, sessionId, setIsStreaming, setRunStatus, setRuntimeError]);

  const handleContinueRun = useCallback(async () => {
    if (!bootstrap) {
      return;
    }
    appendMessage({
      role: "system",
      type: "text",
      content: CONTINUE_PROMPT_LABEL_ZH,
    });
    await submitPrompt(DEFAULT_CONTINUE_PROMPT, {
      appendUser: false,
      updateRunTitle: false,
    });
  }, [appendMessage, bootstrap, submitPrompt]);

  const handleRetryLast = useCallback(async () => {
    const lastUserPrompt = [...messages]
      .reverse()
      .find((item) => item.role === "user" && item.type === "text")?.content;
    if (!lastUserPrompt) {
      appendMessage({
        role: "system",
        type: "text",
        content: "没有可重试的上一条用户指令。",
      });
      return;
    }
    await submitPrompt(lastUserPrompt, {
      appendUser: true,
      updateRunTitle: false,
    });
  }, [appendMessage, messages, submitPrompt]);

  const handleExportEvents = useCallback(async () => {
    if (!bootstrap || !sessionId) {
      return;
    }
    try {
      const exported = await withTimeout(
        exportRuntimeEvents(bootstrap, { session_id: sessionId }),
        10_000,
        "runtime.export_events",
      );
      appendMessage({
        role: "system",
        type: "text",
        content: exported.path ? `事件已导出: ${exported.path}` : "事件已导出。",
      });
    } catch (error) {
      const message = String((error as Error).message ?? error);
      setRuntimeError(message);
      appendMessage({
        role: "system",
        type: "text",
        content: `导出事件失败: ${message}`,
      });
    }
  }, [appendMessage, bootstrap, sessionId, setRuntimeError]);

  const handleOpenArtifact = useCallback(async (path: string) => {
    try {
      const ok = await revealRuntimePath(path);
      if (!ok) {
        throw new Error("当前运行环境不能打开本地路径");
      }
    } catch (error) {
      const message = String((error as Error).message ?? error);
      setRuntimeError(message);
      appendMessage({
        role: "system",
        type: "text",
        content: `打开产物失败: ${message}`,
      });
    }
  }, [appendMessage, setRuntimeError]);

  return {
    handleSubmit,
    handleStopRun,
    handleContinueRun,
    handleRetryLast,
    handleExportEvents,
    handleOpenArtifact,
  };
}
