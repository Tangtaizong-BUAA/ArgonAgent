import { useState, useEffect } from "react";
import { OnboardingScreen } from "./components/OnboardingScreen";
import { AppShell } from "./components/AppShell";
import { configureRuntimeProvider, healthCheckRuntimeProvider, markDesktopReady } from "./runtime/localRuntimeClient";

interface AppConfig {
  provider: "deepseek" | "qwen";
  apiKey?: string;
  baseUrl?: string;
  modelId?: string;
}

const FRONTEND_BOOT_MARK = "argon-agent-app:boot-v1";

function normalizeConfig(input: AppConfig): AppConfig {
  return {
    ...input,
    provider: input.provider === "qwen" ? "qwen" : "deepseek",
    apiKey: input.apiKey?.trim() || undefined,
    baseUrl: input.baseUrl?.trim() || undefined,
    modelId:
      input.provider === "deepseek"
        ? input.modelId?.trim() || "deepseek-v4-flash"
        : undefined,
  };
}

function isConfigComplete(config: AppConfig): boolean {
  if (config.provider === "qwen") {
    return true;
  }
  return Boolean(config.apiKey?.trim() || config.baseUrl?.trim() || config.modelId?.trim());
}

function storageConfig(config: AppConfig): AppConfig {
  return {
    provider: config.provider,
    baseUrl: config.baseUrl,
    modelId: config.modelId,
  };
}

export function App() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [isReady, setIsReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const bootstrap = async () => {
      console.info("[ArgonAgent] app_boot", {
        build: FRONTEND_BOOT_MARK,
        tauriInvokeReady: Boolean(window.__TAURI__?.core?.invoke),
        tauriEventReady: Boolean(window.__TAURI__?.event?.listen),
      });
      if (window.__TAURI__ && (!window.__TAURI__?.core?.invoke || !window.__TAURI__?.event?.listen)) {
        console.warn("[ArgonAgent] partial tauri bridge detected; hot-reload/old host may be running.");
      }
      try {
        const saved = localStorage.getItem("deepcode_config");
        if (saved) {
          const parsed = JSON.parse(saved) as AppConfig;
          const normalized = normalizeConfig(parsed);
          if (cancelled) {
            return;
          }
          if (isConfigComplete(normalized)) {
            setConfig(normalized);
            await configureRuntimeProvider({
              provider: normalized.provider,
              apiKey: normalized.apiKey,
              baseUrl: normalized.baseUrl,
              modelId: normalized.modelId,
            });
            if (normalized.apiKey) {
              localStorage.setItem("deepcode_config", JSON.stringify(storageConfig(normalized)));
            }
          } else {
            localStorage.removeItem("deepcode_config");
            setConfig(null);
          }
        }
      } catch {
        // ignore
      } finally {
        if (cancelled) {
          return;
        }
        setIsReady(true);
        void markDesktopReady();
      }
    };
    void bootstrap();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleOnboardingComplete = async (newConfig: AppConfig) => {
    const normalized = normalizeConfig(newConfig);
    await configureRuntimeProvider({
      provider: normalized.provider,
      apiKey: normalized.apiKey,
      baseUrl: normalized.baseUrl,
      modelId: normalized.modelId,
    });
    const health = await healthCheckRuntimeProvider({ provider: normalized.provider });
    if (health && !health.ok) {
      const detail = health.http_status_code
        ? `HTTP ${health.http_status_code}`
        : health.reason ?? health.status;
      throw new Error(`模型服务探活失败: ${detail}`);
    }
    localStorage.setItem("deepcode_config", JSON.stringify(storageConfig(normalized)));
    setConfig(normalized);
  };

  const handleLogout = () => {
    localStorage.removeItem("deepcode_config");
    setConfig(null);
  };

  if (!isReady) {
    return (
      <div className="h-full w-full bg-bg-app flex items-center justify-center">
        <div className="w-6 h-6 border-2 border-text-muted/30 border-t-accent rounded-full animate-spin" />
      </div>
    );
  }

  if (!config) {
    return <OnboardingScreen onComplete={handleOnboardingComplete} />;
  }

  return <AppShell config={config} onLogout={handleLogout} />;
}
