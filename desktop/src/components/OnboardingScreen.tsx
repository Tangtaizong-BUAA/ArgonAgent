import { useState } from "react";
import { AlertCircle, ArrowRight, Brain, Check, KeyRound, Server } from "lucide-react";

interface OnboardingScreenProps {
  onComplete: (config: {
    provider: "deepseek" | "qwen";
    apiKey?: string;
    baseUrl?: string;
    modelId?: string;
  }) => Promise<void> | void;
}

export function OnboardingScreen({ onComplete }: OnboardingScreenProps) {
  const [provider, setProvider] = useState<"deepseek" | "qwen">("deepseek");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("https://api.deepseek.com/v1");
  const [isLoading, setIsLoading] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  const handleEnter = async () => {
    if (isLoading) {
      return;
    }
    setSubmitError(null);
    if (provider === "deepseek") {
      try {
        const parsed = new URL(baseUrl.trim());
        if (!parsed.protocol.startsWith("http")) {
          throw new Error("invalid protocol");
        }
      } catch {
        setSubmitError("Base URL 格式不正确，请填写 http(s) 地址。");
        return;
      }
    }
    setIsLoading(true);
    try {
      await onComplete({
        provider,
        apiKey: provider === "deepseek" ? apiKey : undefined,
        baseUrl: provider === "deepseek" ? baseUrl : "http://127.0.0.1:11434/v1",
        modelId: provider === "deepseek" ? "deepseek-v4-flash" : undefined,
      });
    } catch (error) {
      const message = String((error as Error).message ?? error).trim();
      setSubmitError(message || "配置保存失败，请重试");
      setIsLoading(false);
    }
  };

  const canEnter = provider === "qwen" || (provider === "deepseek" && apiKey.trim().length > 0);

  return (
    <div className="flex flex-col items-center justify-center h-full w-full bg-bg-app relative">
      <div className="flex flex-col items-center max-w-[420px] w-full px-6 -mt-8">
        {/* Logo */}
        <div className="w-[72px] h-[72px] rounded-2xl overflow-hidden bg-white shadow-lg mb-8 flex items-center justify-center">
          <img
            src="/logo.jpg"
            alt="Argon Agent"
            className="w-full h-full object-cover"
            draggable={false}
          />
        </div>

        {/* Title */}
        <h1 className="text-[28px] font-medium text-text-primary mb-3 tracking-tight">
          欢迎使用 Argon Agent
        </h1>

        {/* Tag */}
        <div className="flex items-center gap-1.5 px-3 py-1 rounded-full bg-accent/10 text-accent text-[12px] font-medium mb-10">
          <Check size={12} strokeWidth={2.5} />
          DeepSeek / Qwen 原生优化
        </div>

        {/* Provider Selection */}
        <div className="w-full space-y-3 mb-6">
          {/* DeepSeek Card */}
          <button
            onClick={() => setProvider("deepseek")}
            className={`w-full flex items-center gap-3 p-4 rounded-2xl border text-left transition-all ${
              provider === "deepseek"
                ? "bg-bg-card border-accent/40 shadow-sm"
                : "bg-transparent border-border-subtle hover:border-text-muted/40"
            }`}
          >
            <div className={`w-10 h-10 rounded-xl flex items-center justify-center shrink-0 ${
              provider === "deepseek" ? "bg-accent/15" : "bg-bg-card"
            }`}>
              <Brain size={20} className={provider === "deepseek" ? "text-accent" : "text-text-muted"} />
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-[14px] font-medium text-text-primary">DeepSeek</div>
              <div className="text-[12px] text-text-muted">云端 API，需要密钥</div>
            </div>
            <div className={`w-5 h-5 rounded-full border-2 flex items-center justify-center shrink-0 ${
              provider === "deepseek" ? "border-accent" : "border-text-muted/30"
            }`}>
              {provider === "deepseek" && <div className="w-2.5 h-2.5 rounded-full bg-accent" />}
            </div>
          </button>

          {/* DeepSeek API Key Input */}
          {provider === "deepseek" && (
            <div className="w-full space-y-3 pl-4 pr-4 pb-1 animate-in fade-in slide-in-from-top-1 duration-200">
              <div className="relative">
                <KeyRound size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-text-muted" />
                <input
                  type="password"
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  placeholder="输入 DeepSeek API Key"
                  className="w-full bg-bg-card border border-border-subtle rounded-xl pl-9 pr-3 py-2.5 text-[13px] text-text-primary placeholder:text-text-muted outline-none focus:border-accent/50 transition-colors"
                />
              </div>
              <div className="relative">
                <Server size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-text-muted" />
                <input
                  type="text"
                  value={baseUrl}
                  onChange={(e) => setBaseUrl(e.target.value)}
                  placeholder="Base URL"
                  className="w-full bg-bg-card border border-border-subtle rounded-xl pl-9 pr-3 py-2.5 text-[13px] text-text-primary placeholder:text-text-muted outline-none focus:border-accent/50 transition-colors"
                />
              </div>
              <p className="text-[11px] text-text-muted pl-1">
                密钥会写入本机运行时配置文件，仅用于连接你选择的模型服务。
              </p>
            </div>
          )}

          {/* Qwen Card */}
          <button
            onClick={() => setProvider("qwen")}
            className={`w-full flex items-center gap-3 p-4 rounded-2xl border text-left transition-all ${
              provider === "qwen"
                ? "bg-bg-card border-accent/40 shadow-sm"
                : "bg-transparent border-border-subtle hover:border-text-muted/40"
            }`}
          >
            <div className={`w-10 h-10 rounded-xl flex items-center justify-center shrink-0 ${
              provider === "qwen" ? "bg-accent/15" : "bg-bg-card"
            }`}>
              <Server size={20} className={provider === "qwen" ? "text-accent" : "text-text-muted"} />
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-[14px] font-medium text-text-primary">Qwen 3.6-27B</div>
              <div className="text-[12px] text-text-muted">本地 Ollama，无需密钥</div>
            </div>
            <div className={`w-5 h-5 rounded-full border-2 flex items-center justify-center shrink-0 ${
              provider === "qwen" ? "border-accent" : "border-text-muted/30"
            }`}>
              {provider === "qwen" && <div className="w-2.5 h-2.5 rounded-full bg-accent" />}
            </div>
          </button>

          {/* Qwen hint */}
          {provider === "qwen" && (
            <div className="pl-4 pr-4 pb-1 animate-in fade-in slide-in-from-top-1 duration-200">
              <p className="text-[11px] text-text-muted">
                需要本地安装 Ollama 并拉取 qwen3.6-27b 模型。
                <a
                  href="https://ollama.com/download"
                  target="_blank"
                  rel="noreferrer"
                  className="text-accent hover:underline ml-1"
                >
                  查看安装指南
                </a>
              </p>
            </div>
          )}
        </div>

        {/* Enter Button */}
        <button
          onClick={handleEnter}
          disabled={!canEnter || isLoading}
          aria-busy={isLoading}
          className={`w-full flex items-center justify-center gap-2 py-3 rounded-full text-[14px] font-medium transition-all ${
            canEnter
              ? "bg-white text-bg-app hover:bg-white/90 shadow-lg"
              : "bg-bg-card text-text-muted cursor-not-allowed"
          }`}
        >
          {isLoading ? (
            <>
              <div className="w-4 h-4 border-2 border-bg-app/30 border-t-bg-app rounded-full animate-spin" />
              保存并探活
            </>
          ) : (
            <>
              进入工作台
              <ArrowRight size={16} />
            </>
          )}
        </button>

        {submitError && (
          <div className="mt-3 w-full rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-[12px] text-red-200 flex items-start gap-2">
            <AlertCircle size={14} className="mt-[1px] shrink-0" />
            <span>{submitError}</span>
          </div>
        )}

        {/* Secondary hint */}
        <div className="mt-4 text-[12px] text-text-muted">
          首次使用？
          <a
            href="https://api-docs.deepseek.com/"
            target="_blank"
            rel="noreferrer"
            className="text-accent hover:underline ml-1"
          >
            阅读快速开始指南
          </a>
        </div>
      </div>
    </div>
  );
}
