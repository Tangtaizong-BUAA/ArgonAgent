# 10 Model Optimization Architecture

## 0. Native Model Scope

Architecture Decision:
- The code-agent runtime has two first-party native optimization modes: `DeepSeekOptimizedMode` and `Qwen36_27BOptimizedMode`.
- ClaudeCode is treated as the main reference for how to shape an agent scaffold around a model family. We borrow its model adaptation mechanics, while the initial native serving endpoints are DeepSeek and Qwen3.6-27B.
- Cross-family fallback is opt-in. A DeepSeek session stays in the DeepSeek family and a Qwen3.6-27B session stays in the Qwen family unless the user explicitly approves fallback.
- Non-native models may exist in eval comparisons or future adapter experiments, but they must not appear as default code-agent routing targets.
- Latest convergence rule: Claude, OpenAI/Codex/GPT, GLM, local models, OpenAI-compatible APIs, Anthropic-compatible APIs, and custom providers are `CompatibleProvider` only. They can be manual options, baselines, or explicitly approved fallbacks, but they do not receive native optimization and cannot enter DeepSeek/Qwen native eval promotion.

## 1. ModelProfile Standard

```ts
interface ModelProfile {
  id: string;
  provider: string;
  model: string;
  family: "deepseek" | "qwen";
  modeId: "deepseek-v4" | "qwen3.6-27b";
  roles: AgentRole[];
  strengths: string[];
  weaknesses: string[];
  forbiddenRoles: AgentRole[];
  context: {
    maxTokens: number;
    compactAtTokens: number;
    reserveOutputTokens: number;
    stablePrefixPreferred: boolean;
    preserveReasoning?: boolean;
  };
  prompting: {
    baseTemplate: string;
    roleTemplates: Record<AgentRole, string>;
    maxReasoningEffort?: "off" | "low" | "medium" | "high" | "max";
    promptTemplateHash?: string;
  };
  deployment: {
    stack: "api" | "dashscope" | "vllm" | "sglang" | "ktransformers" | "transformers";
    endpoint: string;
    parserFlags: string[];
    contextLength: number;
    supportsPreserveThinking: boolean;
  };
  generation: {
    thinkingGeneral?: SamplingProfile;
    thinkingCoding?: SamplingProfile;
    nonThinking?: SamplingProfile;
  };
  tools: {
    nativeToolCalling: boolean;
    preferredEditMode: "structured_edit" | "apply_patch" | "dsml_xml" | "none";
    strictToolSchema: boolean;
    repairStrategy: string[];
  };
  routing: {
    costTier: "low" | "medium" | "high";
    latencyTier: "low" | "medium" | "high";
    privacyTier: "local" | "cloud";
    sameFamilyFallbackOnly: boolean;
    fallbackProfiles: string[];
  };
  eval: {
    metrics: string[];
    minimumScores: Record<string, number>;
    knownFailures: string[];
  };
}

interface SamplingProfile {
  temperature: number;
  topP: number;
  topK?: number;
  minP?: number;
  presencePenalty?: number;
  repetitionPenalty?: number;
  maxTokens: number;
  thinking: "on" | "off";
  preserveThinking?: boolean;
}
```

## 2. ModelRouter Decision Logic

Steps:
1. Classify task: coding/research/data/review/summarization/automation.
2. Split roles: planner, explorer, executor, reviewer, summarizer.
3. Apply constraints: privacy, budget, latency, offline mode, user preference.
4. Select profile per role.
5. Assemble model-specific prompt and tool schema.
6. Parse/repair output.
7. On failure, apply retry policy or fallback.
8. Log metrics to eval/profile memory.

## 3. TaskClassifier

Signals:
- user intent;
- files selected;
- repository language/framework;
- requested action risk;
- data file types;
- expected artifacts;
- command/test requirement;
- privacy constraints.

Output:
- task kind, risk level, required tools, recommended roles, eval case tag.

## 4. Planner / Executor / Reviewer Split

Planner:
- high reasoning, broader context, lower tool access, produces plan.

Executor:
- precise tool calls, file edits, tests, lower context, constrained tools.

Reviewer:
- reads final diff, logs, tests, requirements; cannot modify unless asked; produces verdict/fix recommendations.

Architecture Decision:
- Stronger role profiles inside the same native family can plan/review; cheaper/faster profiles inside the same family can execute low-risk read/search/summarize. All high-risk patches need reviewer loop plus tests.

## 5. DeepSeek V4 Strategy

Observed basis:
- `DeepSeek_V4.pdf`: 1M advertised context, reasoning effort modes, DSML/XML tool schema, interleaved thinking, strong reasoning/long context, Flash weaker than Pro on coding agent tasks.
- Local runtime rule: ResearchCode keeps DeepSeek native requests under a 256K effective cap by default because local engineering validation found higher requests unreliable.
- DeepSeek-TUI: `reasoning_content` replay/sanitizer, stable prefix prompt, sorted memoized tool schemas, V4-aware compaction thresholds.

Profile:
- `deepseek-v4-pro-max`: planner, hard diagnosis, research synthesis, long-context reviewer.
- `deepseek-v4-pro-high`: coding executor, data analysis script generator, report writer.
- `deepseek-v4-flash-high`: explorer, summarizer, schema profiler narrator, cheap parallel candidate generation.
- `deepseek-v4-nonthink`: simple summaries and low-risk tasks.

Rules:
- Use native tool calls; do not simulate tool results as user text in thinking mode.
- Preserve `reasoning_content` where provider requires it.
- Keep stable prompt/tool catalog prefix.
- Compact late; log cache hit/miss.
- Use DSML/XML parser only as fallback.
- Use arg repair, but count repairs as model/tool errors in eval.

Failure recovery:
- Tool-call parse error -> repair once -> invalid-tool feedback -> retry with stricter prompt.
- Missing reasoning_content 400 -> sanitizer -> retry.
- Cache hit collapse -> stabilize context or compact intentionally.
- Over-thinking -> lower effort for executor/summarizer.

## 6. Qwen 3.6 27B Strategy

Observed basis:
- Hugging Face `Qwen/Qwen3.6-27B`: 27B post-trained model, agentic coding emphasis, repository-level reasoning, thinking preservation, 262K native context, extendable long context, OpenAI-compatible serving examples.
- Qwen3.6 deployment examples use Qwen-specific parser flags such as `qwen3` reasoning parser and `qwen3_coder` tool-call parser for SGLang/vLLM.
- Qwen3.6 thinks by default; non-thinking and preserve-thinking are explicit runtime/deployment parameters.
- Qwen function-calling docs show that tool use depends on Qwen templates/parsers, not generic transport compatibility.

Recommended roles:
- `qwen3.6-27b-thinking`: planner, diagnosis, repository-level reasoning, reviewer, research synthesis.
- `qwen3.6-27b-coding`: bounded code executor, frontend executor, build/test repair.
- `qwen3.6-27b-long-context`: repo-level analysis and long research/document context when 262K or extended context is deployed.
- `qwen3.6-27b-nonthink`: status updates, simple summaries, low-risk direct transforms.
- `qwen3.6-27b-research`: bilingual scientific assistant, data interpretation narrator, report drafter.

Guardrails:
- Use Qwen-specific chat template/tool parser configuration.
- Record parser flags, thinking flags, context length, and template hash in every model call.
- Use thinking mode for planning/diagnosis/review; use non-thinking mode only for simple low-risk responses.
- Use precise coding sampling for coding edits.
- Prefer structured patch proposals, diff preview, stale-file detection, and tests for all edits.
- Treat extended context beyond 262K as a deployment capability, not a default.

Sampling profiles:
- Thinking/general: `temperature=1.0`, `top_p=0.95`, `top_k=20`, `min_p=0.0`, `presence_penalty=0.0`, `repetition_penalty=1.0`.
- Thinking/precise coding: `temperature=0.6`, `top_p=0.95`, `top_k=20`, `min_p=0.0`, `presence_penalty=0.0`, `repetition_penalty=1.0`.
- Non-thinking/instruct: `temperature=0.7`, `top_p=0.80`, `top_k=20`, `min_p=0.0`, `presence_penalty=1.5`, `repetition_penalty=1.0`.
- `maxTokens` should default to the deployment's validated output budget, with Qwen3.6 examples allowing large outputs but runtime limiting per role.

Forbidden until eval:
- final reviewer for security-sensitive changes;
- destructive shell planning;
- unsupervised multi-file refactors;
- generic OpenAI tool calling without a Qwen-compatible parser/template;
- treating Qwen2/Qwen2-7B behavior as the Qwen3.6-27B production profile.

## 7. ClaudeCode Adaptation Pattern as Scaffold Reference

Observed from ClaudeCode:
- Model config maps canonical Claude families to provider-specific IDs.
- Model selection accounts for plan mode, user overrides, settings, defaults, and aliases.
- Context/output caps are model-gated, including long-context suffixes and beta headers.
- Thinking policy is model/provider-aware; temperature is omitted when thinking is enabled where required.
- Tool schemas are cached session-stably because byte drift breaks prompt cache.
- Tool-use/tool-result pairing is repaired before API submission.
- Beta headers, cache markers, and feature flags are latched for session stability.

How we absorb it:
- DeepSeek and Qwen get canonical family configs plus provider/deployment-specific IDs.
- DeepSeek and Qwen get capability registries for thinking, long context, tool calling, strict schema, parser mode, and cache behavior.
- DeepSeek and Qwen each get a `ModeAdapter`: prompt builder, tool serializer, output parser, retry policy, context policy, and eval suite.
- DeepSeek and Qwen both use stable prompt/tool bytes with hashes in logs.
- Model feature kill switches are first-class: if a gateway rejects strict schema, preserve-thinking, DSML, tool parser, or long-context flags, the adapter disables that capability and logs the downgrade.

## 8. Scaffold Levels and Context Budget Policy

Latest decision: ClaudeCode remains the primary reference for mature agent scaffold engineering, but the scaffold must be split into runtime, prompt, context, and eval layers.

Rules:
- Runtime scaffold should stay ClaudeCode-strength for both DeepSeek and Qwen: event log, state machine, TaskContract, permission manager, patch manager, read-before-write, stale-file detection, tool dispatcher, artifact store, reviewer loop, and eval gates.
- Prompt scaffold is model-specific. DeepSeek can receive the closest ClaudeCode-like full lifecycle prompt, but it must fit the 256K effective ResearchCode safety cap. Qwen3.6-27B receives ClaudeCode-lite prompts because its native budget is 262K and it benefits more from small steps plus runtime validators than from a huge prompt.
- Context scaffold is dynamically budgeted. Repo map, file snippets, tool outputs, memory, research schema, and reasoning replay must not consume output reserve or emergency reserve.
- Eval scaffold decides promotion. Any DeepSeek/Qwen scaffold change must be validated before becoming default.

Scaffold levels:
- `S0 CompatibleMinimal`: compatible providers only, connection/baseline/manual fallback, no native optimization.
- `S1 QwenFast`: Qwen executor/summarizer, short mode prompt, narrow active tools, patch-sized edits.
- `S2 QwenGuarded`: Qwen planner/reviewer/researcher, structured thinking/preserve-thinking use, read-only exploration before edits, tests/reviewer required.
- `S3 DeepSeekFull`: DeepSeek planner/executor/reviewer/researcher/summarizer, ClaudeCode-like lifecycle scaffold, 256K-bounded context, live-loop compaction, reasoning replay, stable prefix/cache strategy.

Implementation:
- `crates/runtime/src/context_budget.rs` defines `ContextBudget`, `ScaffoldLevel`, allocation, and validation.
- CLI smoke gate: `cargo run -q -p researchcode-cli -- context-budget-smoke`.
- Prompt assembly must consume `ContextBudget` before full prompt scaffold becomes default.

See `36_context_budget_and_scaffold_policy.md` for the full policy and budget targets.

## 9. Non-Native Systems Policy

Claude, OpenAI/Codex/GPT, GLM, generic local models, OpenAI-compatible APIs, Anthropic-compatible APIs, and custom providers are represented by `CompatibleProviderConfig`, not `NativeModelProfile`.

Rules:
- Used as architecture references, benchmark baselines, manual options, compatibility targets, or explicitly approved fallbacks.
- Not default code-agent targets in the first native product line.
- Not used for silent fallback in DeepSeek/Qwen sessions.
- Can inform eval design, prompt discipline, tool schema validation, GUI patterns, and runtime safety boundaries.
- Cannot be marked `optimization_level = native`.
- Cannot override DeepSeek/Qwen prompt, parser, context, error-recovery, or eval policies.
- Cannot enter native DeepSeek/Qwen promotion gates.

See `27_model_scope_and_provider_layer.md` for `ProviderConfig`, `ModelAliasMapping`, health check, and GUI display rules.

## 9. Same-Family Failure Compensation

Mechanisms:
- plan generated by stronger profile inside the selected native family;
- cheaper/faster profile gets one bounded step;
- runtime validates tool calls and paths;
- patch manager blocks risky edits;
- reviewer profile inside the same native family inspects result;
- fallback across model families only after explicit user approval.

## 10. Tool-Use Error Repair

Repair stack:
1. parser normalization;
2. schema coercion;
3. deterministic argument repair;
4. invalid-tool feedback;
5. retry with stricter tool prompt;
6. same-family fallback profile.

All repairs are logged as eval events.

## 11. Context Compression

Policies:
- DeepSeek V4: preserve stable prefix, trigger live-loop compaction at 192K, target prepared requests below 240K, and block before the 256K hard cap.
- Qwen3.6-27B: use 262K native context deliberately, keep repo map and exact edit snippets, preserve thinking only when it improves continuity, and compact before extended-context assumptions unless deployment is verified.

## 12. Long Task Continuation

Artifacts:
- structured plan;
- current state;
- failures/retries;
- edited files;
- test commands and results;
- next action;
- model profile used and known issues.

## 13. Eval-Driven Profile Update

Eval records:
- role, model, task kind, context size, tools, retries, failures, cost, latency, user interventions, final pass/fail.

Profile tuning:
- adjust role eligibility;
- adjust context budget;
- change edit mode;
- raise/lower reasoning effort;
- change fallback order.

## 14. Prompt Template Management

Templates:
- global system contract;
- role-specific prompt;
- model-profile adapter;
- tool contract;
- output schema;
- recovery prompt;
- reviewer prompt.

Versioning:
- each model call records template IDs and hashes.

## 15. Per-Model Output Parser

Parsers:
- DeepSeek reasoning/tool calls plus DSML fallback.
- Qwen3.6 reasoning parser, `qwen3_coder` tool-call parser where available, Qwen template/tag parser fallback, stricter JSON repair.
- Generic text-to-tool fallback disabled unless eval permits it for the selected native family.

## 16. Cost / Latency / Success Tradeoff

Routing policy:
- high-risk/high-value -> strongest profile inside selected native family.
- low-risk/parallel exploration -> faster/cheaper profile inside selected native family.
- privacy-critical -> local Qwen3.6-27B deployment if available, or user-approved DeepSeek/Qwen cloud endpoint.
- long-context research -> DeepSeek V4 Pro or Qwen3.6-27B long-context mode depending on selected family and verified deployment.

## 17. Routing Examples

| Task | Planner | Executor | Reviewer | Notes |
|---|---|---|---|---|
| Coding planning in DeepSeek mode | DeepSeek V4 Pro-Max | none | none | broad repo context inside 256K safety cap |
| Coding planning in Qwen mode | Qwen3.6-27B thinking | none | none | Qwen thinking parser, 128K+ preferred |
| Coding execution in DeepSeek mode | DeepSeek V4 Pro-High | DeepSeek V4 Pro/Flash by risk | DeepSeek V4 Pro-Max | patch manager required |
| Coding execution in Qwen mode | Qwen3.6-27B thinking | Qwen3.6-27B coding | Qwen3.6-27B thinking | parser/tool fixtures required |
| Build failure diagnosis | selected family strongest diagnosis profile | selected family executor | selected family reviewer | include command logs |
| CSV data profiling | selected family research profile summarizes | Python sidecar computes | selected family reviewer if report | model does not compute stats |
| Python analysis script | DeepSeek Pro or Qwen thinking by selected mode | Python sidecar executes | selected family reviewer | test on sample/full |
| Research report writing | selected family research/planner profile | none | selected family reviewer | cite artifacts |
| Long-context summarization | DeepSeek V4 Pro/Flash or Qwen3.6 long-context by selected mode | none | strongest same-family reviewer | cache/parser aware |

## 18. Model Failure Memory

Store:
- model, role, failure type, prompt hash, tool schema hash, context size, recovery success.

Use:
- avoid repeating bad routing choices;
- show warnings in model profile settings;
- drive eval backlog.
