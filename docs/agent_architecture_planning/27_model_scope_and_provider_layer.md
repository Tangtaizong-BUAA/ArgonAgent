# 27 Model Scope and Provider Layer

本文件解决的问题：把模型范围从“宏大多模型优化平台”收敛为 **DeepSeek/Qwen native-first**，并为其他模型定义只负责接入的 compatible provider layer。

它修正旧文档的方式：保留 `10_model_optimization_architecture.md` 与 `15_native_deepseek_qwen_modes.md` 中的 DeepSeek/Qwen native optimization；把 Claude/OpenAI/GLM/local/custom 从 native optimization 范围中移出，统一归入 CompatibleProvider。

## 1. Final Model Scope

**Decision:** 核心 native optimized models 只有两个：

- **DeepSeek = native optimized**
- **Qwen / Qwen3.6-27B = native optimized**

**Rule:** Claude、OpenAI/Codex/GPT、GLM、local models、OpenAI-compatible API、Anthropic-compatible API、custom provider 全部是 **CompatibleProvider**。它们可以接入、对比、手动选择、做 baseline 或 fallback，但不能被标记为 native。

**Rationale:** ClaudeCode 的价值是“如何为特定模型调教 agent scaffold”的参考模式，不是本产品要 native optimize Claude。DeepSeek-TUI 与 Qwen 文档已经给出了 DeepSeek/Qwen 的真实 native optimization 面。

**Implementation Impact:** Runtime 必须有两层模型系统：

- `NativeOptimizedModel`：DeepSeek/Qwen 专属 profile、prompt、parser、context、error recovery、eval gate。
- `CompatibleProvider`：协议接入、alias 映射、request/response transform、health check，不做专属优化。

**Eval Impact:** 只有 DeepSeek/Qwen 进入 native eval promotion。Compatible provider 只能进入 baseline/compatibility eval。

**Go/No-Go Impact:** 如果 compatible provider 能被配置成 `optimization_level = native`，则 scaffold 前必须阻塞。

## 2. DeepSeek/Qwen Native-First Strategy

**Decision:** 产品默认质量承诺只覆盖 DeepSeek 和 Qwen3.6-27B native modes。

DeepSeek native mode includes:

- 1M context policy;
- V4-aware late compaction;
- reasoning replay/sanitizer;
- native tool calls first;
- DSML/XML fallback;
- JSON argument repair;
- prefix-cache stable prompt and sorted/memoized tool catalog;
- cache/reasoning telemetry;
- Pro/Flash/Non-think role split;
- DeepSeek eval gates.

Qwen native mode includes:

- Qwen3.6-27B canonical target;
- 262K native context, extended context only by deployment capability flag;
- Qwen chat template;
- qwen3 reasoning parser;
- qwen3_coder tool parser where available;
- thinking/non-thinking/preserve-thinking policies;
- precise coding sampling;
- structured patch/stale-file/test/reviewer loop;
- Qwen parser/executor/long-context eval gates.

## 3. Why DeepSeek/Qwen Are Native Optimized Models

| Model Family | Native reason | Dedicated optimization surface |
|---|---|---|
| DeepSeek | Existing docs show V4 1M context, reasoning replay, prefix-cache economics, DSML/XML fallback, DeepSeek-TUI cache-aware compaction. | Prompt order, tool catalog stability, parser fallback, reasoning sanitizer, Pro/Flash role split, cache telemetry. |
| Qwen3.6-27B | User-selected canonical target; docs show Qwen-specific template/parser/thinking/262K context behavior. | Qwen template, qwen3 reasoning parser, qwen3_coder parser, preserve-thinking, deployment capability checks, Qwen eval gates. |

## 4. Why Other Models Are Compatible Providers Only

| Provider family | Role | Not native because |
|---|---|---|
| Claude | Architecture reference, optional compatible/baseline provider. | Product does not promise Claude-native prompt/cache/thinking optimization. |
| OpenAI/Codex/GPT | Baseline/manual provider. | Useful comparison, but not native optimization target. |
| GLM | Compatible/custom provider. | No dedicated profile/eval/prompt/parser commitment in current product scope. |
| Local models | Compatible provider or baseline. | Local serving varies too much; no native quality commitment. |
| OpenAI-compatible APIs | Protocol adapter. | Transport compatibility is not model-native behavior. |
| Anthropic-compatible APIs | Protocol adapter. | Protocol compatibility does not imply ClaudeCode-like optimization. |
| Custom Provider | Manual option. | Must prove behavior through health/eval; no native status. |

## 5. Definitions

### NativeOptimizedModel

```ts
type NativeOptimizedModel = {
  family: "deepseek" | "qwen";
  canonical_target: "deepseek-v4" | "qwen3.6-27b";
  native_profile_id: string;
  optimization_level: "native";
  prompt_strategy: NativePromptStrategy;
  tool_strategy: NativeToolStrategy;
  parser_strategy: NativeParserStrategy;
  context_strategy: NativeContextStrategy;
  error_recovery: NativeErrorRecoveryPolicy;
  eval_suite_id: string;
}
```

### CompatibleProvider

```ts
type CompatibleProvider = {
  provider_id: string;
  config: CompatibleProviderConfig;
  optimization_level: "compatible" | "baseline";
}
```

### NativeModelProfile vs CompatibleProviderConfig

| Aspect | NativeModelProfile | CompatibleProviderConfig |
|---|---|---|
| Applies to | DeepSeek/Qwen only | Claude/OpenAI/GLM/local/custom |
| Has dedicated prompt/parser/context | Yes | No; only protocol transforms |
| Can enter native eval promotion | Yes | No |
| Can define role split | Yes | No native role promise |
| Affects Product Kernel | Only through stable kernel interfaces | Must not alter kernel |
| Default quality commitment | Yes | No |

## 6. Native-Only Features

Only DeepSeek/Qwen can use:

- native model profile;
- family-specific system prompt and role prompt;
- family-specific tool serialization;
- family-specific parser;
- family-specific reasoning/thinking policy;
- family-specific compaction/context policy;
- family-specific error recovery;
- family-specific failure memory;
- family-specific profile tuning;
- native eval promotion.

## 7. Provider-Agnostic Features

All providers can use:

- Runtime API;
- Event Log;
- ToolSpec registry;
- Permission Manager;
- PatchProposal and read-before-write validation;
- Artifact Store;
- Research Worker job manifests;
- basic model call logging;
- baseline eval runs;
- GUI session view and approval flows.

## 8. ProviderConfig Schema

```ts
type OptimizationLevel = "native" | "compatible" | "baseline";
type ProviderProtocol = "openai_compatible" | "anthropic_compatible" | "custom";

type CompatibleProviderConfig = {
  provider_id: string;
  display_name: string;
  protocol: ProviderProtocol;
  base_url: string;
  api_key_env?: string;
  actual_model_name: string;
  display_model_name: string;
  model_alias?: string;
  headers?: Record<string, string>;
  capability_hints?: ProviderCapabilityHints;
  request_transform?: RequestTransformSpec;
  response_transform?: ResponseTransformSpec;
  health_check: ProviderHealthCheck;
  enabled_by_default: boolean;
  optimization_level: "compatible" | "baseline";
}
```

**Rule:** `optimization_level = "native"` is invalid for `CompatibleProviderConfig`.

## 9. ModelAliasMapping Schema

```ts
type ModelAliasMapping = {
  provider_id: string;
  model_alias: string;
  actual_model_name: string;
  display_model_name: string;
  base_url_override?: string;
  request_transform_id?: string;
  response_transform_id?: string;
  notes?: string;
}
```

Relationship:

- `base_url` points to endpoint root.
- `actual_model_name` is sent to provider.
- `display_model_name` is shown in GUI/logs.
- `model_alias` is user-facing shorthand.
- `request_transform` adapts request fields to protocol.
- `response_transform` normalizes response into kernel model events.

## 10. Provider Capability Hints

```ts
type ProviderCapabilityHints = {
  max_context_tokens?: number;
  max_output_tokens?: number;
  supports_streaming?: boolean;
  supports_tools?: boolean;
  supports_json_schema?: boolean;
  supports_reasoning_channel?: boolean;
  supports_parallel_tool_calls?: boolean;
  supports_usage_tokens?: boolean;
  supports_cache_telemetry?: boolean;
}
```

**Rule:** Capability hints are not native guarantees. They guide compatibility checks and must be validated by health checks/evals.

## 11. OpenAI-Compatible Provider Flow

1. User creates `CompatibleProviderConfig`.
2. Runtime validates `base_url`, `actual_model_name`, protocol, and auth env reference.
3. Health check sends non-sensitive minimal request.
4. Response is normalized through `response_transform`.
5. Provider appears in GUI as `Compatible` or `Baseline`.
6. It can be manually selected or used as baseline/fallback only when policy permits.

## 12. Anthropic-Compatible Provider Flow

1. User selects `protocol = "anthropic_compatible"`.
2. Config declares endpoint and actual model name.
3. Runtime runs protocol health check.
4. Adapter normalizes messages/tool outputs into kernel event shapes.
5. Provider remains compatible-only unless future user request authorizes native eval program.

## 13. Custom Provider Flow

1. User defines request/response transforms.
2. Health check must pass.
3. Provider is disabled by default until manually enabled.
4. GUI labels it `Custom Compatible`.
5. No DeepSeek/Qwen native parser or context policy is reused automatically.

## 14. GUI Display Rules

| Provider type | GUI label | Default? | Quality promise |
|---|---|---|---|
| DeepSeek native | `Native: DeepSeek` | Yes if configured | Native optimized |
| Qwen native | `Native: Qwen3.6-27B` | Yes if configured | Native optimized |
| Compatible | `Compatible: <display_name>` | No unless user enables | Connectivity only |
| Baseline | `Baseline: <display_name>` | No | Eval/manual comparison only |

**Rule:** GUI must not display compatible providers in the same visual rank as native modes.

## 15. Compatible Provider Usage Modes

- Manual option: user explicitly selects it for a session.
- Baseline: eval comparison against native modes.
- Fallback: explicit user-approved cross-provider fallback for a single task or session.

**Rule:** Compatible fallback is opt-in and must not silently replace a DeepSeek/Qwen native session.

## 16. Misconfiguration Risks

| Risk | Control |
|---|---|
| model alias points to wrong model | show actual/display names; health check records model id if available |
| compatible provider marked native | schema validation blocks it |
| OpenAI-compatible transport assumed as Qwen-native | native Qwen mode requires parser/template capability check |
| custom provider leaks data | cloud model permission and privacy classification still apply |
| response transform drops tool errors | baseline eval and tool-call logs catch it |

## 17. Provider Health Check

```ts
type ProviderHealthCheck = {
  enabled: boolean;
  method: "models" | "chat_minimal" | "custom";
  timeout_ms: number;
  requires_auth: boolean;
  non_sensitive_prompt: string;
  expected_response_shape: string[];
}
```

Health checks must not send project files or sensitive data.

## 18. Provider Config Validation

Validation fails if:

- `optimization_level = native` and provider is not DeepSeek/Qwen native adapter;
- `actual_model_name` is empty;
- `display_model_name` is misleadingly different without alias note;
- `base_url` is missing for non-local providers;
- `api_key_env` points to a literal key value instead of env var name;
- custom transform is unversioned;
- provider attempts to override native adapter prompt/parser/context policy.

## 19. Execution Impact

- Update model optimization docs to refer to CompatibleProvider rather than broad multi-provider native optimization.
- Add ProviderConfig/ModelAlias schema task to Phase 0.
- Keep DeepSeek/Qwen native profiles isolated from compatible adapters.

## 20. Next Tasks

1. Add JSON Schema drafts for `CompatibleProviderConfig` and `ModelAliasMapping`.
2. Build a health-check spike that sends no project data.
3. Add eval/baseline metadata so compatible providers can be compared without native promotion.

## Open Questions

- Should local Qwen3.6-27B deployments be represented as native Qwen adapters or compatible providers with native capability flags? Current decision: native only if Qwen template/parser/capability checks pass.
- Should Claude compatible provider be included in the first public UI or kept hidden behind advanced settings?

