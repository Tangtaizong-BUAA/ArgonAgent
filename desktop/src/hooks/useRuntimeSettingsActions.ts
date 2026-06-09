import { useCallback, type Dispatch, type SetStateAction } from "react";
import {
  configureRuntimeProvider,
  setRuntimeAutonomyMode,
} from "@/runtime/localRuntimeClient";
import { autonomyModeLabel } from "@/runtime/runtimeEventViewModel";
import type { RuntimeBootstrap } from "@/types/runtime";
import type { RunStatus, TranscriptMessage } from "@/types";
import type { DeepSeekModelId } from "@/components/BottomComposer";
import type { AutonomyModeId } from "@/components/Topbar";

interface RuntimeSettingsConfig {
  provider: "deepseek" | "qwen";
  apiKey?: string;
  baseUrl?: string;
}

type AppendMessage = (message: Omit<TranscriptMessage, "id" | "timestamp">) => void;

interface UseRuntimeSettingsActionsOptions {
  activeModelId: DeepSeekModelId;
  bootstrap: RuntimeBootstrap | null;
  config: RuntimeSettingsConfig;
  runStatus: RunStatus;
  sessionId: string | null;
  appendMessage: AppendMessage;
  pollRuntimeEvents: (sessionId: string) => Promise<void>;
  setActiveModelId: Dispatch<SetStateAction<DeepSeekModelId>>;
  setAutonomyMode: Dispatch<SetStateAction<AutonomyModeId>>;
  setModelSwitchHint: Dispatch<SetStateAction<string | null>>;
  setRuntimeError: Dispatch<SetStateAction<string | null>>;
}

export function useRuntimeSettingsActions({
  activeModelId,
  bootstrap,
  config,
  runStatus,
  sessionId,
  appendMessage,
  pollRuntimeEvents,
  setActiveModelId,
  setAutonomyMode,
  setModelSwitchHint,
  setRuntimeError,
}: UseRuntimeSettingsActionsOptions) {
  const handleModelChange = useCallback(async (nextModelId: DeepSeekModelId) => {
    if (config.provider !== "deepseek") {
      return;
    }
    if (nextModelId === activeModelId) {
      return;
    }
    setActiveModelId(nextModelId);
    setModelSwitchHint(
      runStatus === "running"
        ? `模型已切换为 ${nextModelId}，当前轮结束后按新模型执行。`
        : `模型已切换为 ${nextModelId}，下一轮立即生效。`,
    );
    try {
      await configureRuntimeProvider({
        provider: config.provider,
        apiKey: config.apiKey,
        baseUrl: config.baseUrl,
        modelId: nextModelId,
      });
      const saved = localStorage.getItem("deepcode_config");
      if (saved) {
        const parsed = JSON.parse(saved) as RuntimeSettingsConfig & { modelId?: string };
        localStorage.setItem(
          "deepcode_config",
          JSON.stringify({
            ...parsed,
            modelId: nextModelId,
          }),
        );
      }
    } catch (error) {
      setModelSwitchHint(`模型切换失败: ${String((error as Error).message ?? error)}`);
    }
  }, [
    activeModelId,
    config.apiKey,
    config.baseUrl,
    config.provider,
    runStatus,
    setActiveModelId,
    setModelSwitchHint,
  ]);

  const handleAutonomyModeChange = useCallback(async (mode: AutonomyModeId) => {
    setAutonomyMode(mode);
    if (!bootstrap || !sessionId) {
      return;
    }
    try {
      const result = await setRuntimeAutonomyMode(bootstrap, {
        session_id: sessionId,
        autonomy_mode: mode,
      });
      if (!result.ok) {
        throw new Error("runtime_set_autonomy_mode_failed");
      }
      appendMessage({
        role: "system",
        type: "text",
        content: `自主模式已切换为 ${autonomyModeLabel(mode)}。`,
      });
      await pollRuntimeEvents(sessionId);
    } catch (error) {
      const message = String((error as Error).message ?? error);
      setRuntimeError(message);
      appendMessage({
        role: "system",
        type: "text",
        content: `自主模式切换失败: ${message}`,
      });
    }
  }, [
    appendMessage,
    bootstrap,
    pollRuntimeEvents,
    sessionId,
    setAutonomyMode,
    setRuntimeError,
  ]);

  return {
    handleModelChange,
    handleAutonomyModeChange,
  };
}
