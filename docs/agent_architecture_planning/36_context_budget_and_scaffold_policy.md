# 36 Context Budget and Scaffold Policy

本文件解决的问题：把 `context_budget_and_scaffold_policy_recommendation.md` 和最新讨论收敛成可执行规则，避免两个极端：一是把 ClaudeCode 的成熟 scaffold 简化丢掉，二是把所有模型都塞进同一套巨大提示词导致 Qwen 上下文被脚手架吃光。

修正旧文档的方式：补充 `10_model_optimization_architecture.md`、`34_AGENTS_md_draft.md`、`35_go_no_go_checklist.md` 中缺失的 context budget/scaffold gate。旧的 “Go for minimal scaffold” 仍成立，但 full prompt scaffold 不能默认启用，必须经过本文件定义的预算与 eval gate。

## Decision

DeepSeek/Qwen 的 agent 框架必须尽可能吸收 ClaudeCode 的成熟工程经验，但吸收对象要分层：

- Runtime scaffold: 尽量贴近 ClaudeCode 的成熟闭环，所有模型都强制保留。
- Prompt scaffold: DeepSeek 可以接近 ClaudeCode full scaffold，Qwen3.6-27B 必须使用 ClaudeCode-lite。
- Context scaffold: 由 `ContextBudgetManager` 动态分配，不能静态堆满 repo、工具、历史、日志。
- Eval scaffold: 每次修改 DeepSeek/Qwen native scaffold 都必须通过 parser、tool、patch、repair、long-context eval 才能 promotion。

## ClaudeCode Learning Rule

**Rule:** ClaudeCode 是 primary scaffold reference，不是 copy target。

**Rationale:** ClaudeCode 的成功主要来自稳定 agent lifecycle、模型特化 prompt、工具 schema 稳定、read/search/edit/test/review 闭环、权限边界、历史压缩、错误恢复和长任务连续性。这些机制应进入我们的 runtime 和 native adapter，而不是照搬成一个巨大、模型无关的 system prompt。

**Implementation Impact:**

- Runtime 层保留 ClaudeCode-like `plan -> context retrieval -> tool call -> patch -> test -> repair -> review`。
- Prompt assembler 必须按 `NativeModelFamily + ModelRole + ScaffoldLevel` 选择不同提示词预算。
- Tool catalog 必须稳定排序、可 memoize，并记录 prompt/tool hash。
- Model adapter 必须承载 DeepSeek/Qwen 专属 parser、reasoning/thinking、context 和 retry 策略。

## Scaffold Taxonomy

### Runtime Scaffold

运行时脚手架不应为了省 token 而削弱：

- event log;
- AgentSession state machine;
- TaskContract;
- permission manager;
- command classifier;
- patch manager;
- read-before-write;
- base hash and stale-file detection;
- tool dispatcher and schema validator;
- artifact store;
- model call/event log;
- reviewer loop;
- eval harness;
- security policy;
- bounded autonomy stop conditions.

**Rule:** Qwen 比 DeepSeek 更需要强 runtime scaffold，因为 Qwen 的稳定性应主要来自小步执行、严格 schema、校验器和 reviewer，而不是更长提示词。

### Prompt Scaffold

提示词脚手架消耗 token，必须分模型和分角色：

- DeepSeek: S3 Full Scaffold。
- Qwen planner/reviewer/researcher: S2 Guarded Scaffold。
- Qwen executor/summarizer: S1 Fast Scaffold。
- Compatible providers: S0 Minimal Scaffold。

**Rule:** 不能把 DeepSeek S3 full prompt 直接给 Qwen3.6-27B。

### Context Scaffold

上下文脚手架包含 repo map、文件片段、tool results、logs、diff、memory、research schema、reasoning replay。它必须由预算器动态分配。

**Rule:** Prompt/context builder 必须保护 output reserve 和 emergency reserve。repo map、文件片段、工具输出、历史摘要不得吞掉输出空间。

## Scaffold Levels

| Level | Native family | Default roles | Prompt strategy | Runtime strategy |
|---|---|---|---|---|
| S0 Compatible Minimal | compatible only | baseline/manual/fallback | tiny safety prompt, no native profile | basic kernel gates only |
| S1 Qwen Fast | Qwen3.6-27B | executor, summarizer | short mode prompt, narrow tools | patch-sized edits, validators, optional reviewer |
| S2 Qwen Guarded | Qwen3.6-27B | planner, reviewer, researcher | short structured prompt, thinking/preserve-thinking when useful | read-only exploration, small patch loop, required tests/reviewer |
| S3 DeepSeek Full | DeepSeek V4 | planner, executor, reviewer, researcher, summarizer | ClaudeCode-like full lifecycle scaffold | full runtime scaffold, 256K-bounded context, live compaction |

## DeepSeek S3 Policy

**Decision:** DeepSeek can be closest to ClaudeCode’s mature full scaffold, but ResearchCode defaults to a 256K effective safety cap because local runtime validation found higher requests unreliable.

DeepSeek S3 invariants:

- native DeepSeek adapter, not generic OpenAI-compatible mode;
- reasoning_content replay through native field only;
- reasoning_content sanitizer before persistence;
- native tool-call priority;
- DSML/XML fallback;
- JSON argument repair with confidence logging;
- prefix-cache stable system prefix;
- stable sorted/memoized tool catalog;
- prompt cache hit/miss telemetry;
- reasoning replay token telemetry;
- live-loop compaction threshold at 192K with a 240K request target;
- Pro/Flash/Non-think role split;
- strict tool mode gated by provider capability;
- eval-driven profile update.

DeepSeek 256K safe budget target:

| Area | Target |
|---|---:|
| Static system/product scaffold | 8K |
| Native profile/tool-use rules | 3K |
| Active tool schemas | 6K |
| Task contract/plan | 4K |
| Repo map | 20K |
| Relevant source files | 70K |
| Tool outputs/test logs | 24K |
| Session memory/summaries | 12K |
| Reasoning replay reserve | 12K |
| Research data | 8K |
| Output reserve | 16K |
| Emergency reserve | 16K |

## Qwen S1/S2 Policy

**Decision:** Qwen3.6-27B must use ClaudeCode-lite prompts with ClaudeCode-strength runtime discipline.

Qwen invariants:

- Qwen3.6-27B is canonical target;
- Qwen-specific chat template;
- Qwen-specific reasoning parser;
- `qwen3_coder` tool-call parser where available;
- thinking/non-thinking/preserve-thinking mode separation;
- 262K native context as default deployment assumption;
- extended context only by capability flag;
- precise coding sampling;
- structured patch output;
- stale-file detection;
- tests/reviewer loop;
- Qwen parser eval gate;
- Qwen executor eval gate.

Qwen 262K target:

| Area | Target |
|---|---:|
| Static system scaffold | 6K-12K |
| Native Qwen profile | 2K-5K |
| Active tool schemas | 4K-8K |
| Task contract/plan | 2K-5K |
| Repo map | 15K-30K |
| Relevant source files | 80K-130K |
| Tool outputs/test logs | 20K-40K |
| Memory/summaries | 10K-25K |
| Output reserve | 16K-24K |
| Emergency reserve | 25K-40K |

Qwen 128K degraded target:

| Area | Target |
|---|---:|
| Static scaffold | 4K-6K |
| Native profile | 1K-3K |
| Active tools | 3K-5K |
| Task contract/plan | 1K-3K |
| Repo map | 8K-15K |
| Source snippets | 40K-60K |
| Tool outputs | 8K-15K |
| Memory | 5K-10K |
| Output reserve | 10K-16K |
| Emergency reserve | 15K-25K |

## ContextBudgetManager v0

The Rust v0 implementation lives in `crates/runtime/src/context_budget.rs`.

```rust
enum ScaffoldLevel {
    CompatibleMinimal,
    QwenFast,
    QwenGuarded,
    DeepSeekFull,
}

struct ContextBudget {
    model_id: String,
    model_family: NativeModelFamily,
    scaffold_level: ScaffoldLevel,
    max_context_tokens: u64,
    output_reserve_tokens: u64,
    emergency_reserve_tokens: u64,
    static_prompt_budget: u64,
    model_profile_budget: u64,
    tool_schema_budget: u64,
    task_contract_budget: u64,
    repo_map_budget: u64,
    file_snippet_budget: u64,
    tool_output_budget: u64,
    memory_budget: u64,
    reasoning_replay_budget: u64,
    research_data_budget: u64,
    compaction_threshold: u64,
    compaction_floor: u64,
    min_retrieval_budget: u64,
    max_active_tools: usize,
    max_files_per_turn: usize,
    max_tool_output_per_turn: u64,
}
```

Validation rules:

- named budget must fit inside max context;
- output and emergency reserve are mandatory;
- Qwen prompt scaffold must stay below 10% of available context;
- Qwen Fast active tools must remain narrow;
- DeepSeek Full must reserve reasoning replay budget;
- compaction floor must be below compaction threshold.

## Eval Gates

Promotion requires:

- `context-budget-smoke` passes;
- `run_scaffold_eval.py` passes for DeepSeek S3, Qwen S1/S2, active-tool limits, and protected reserves;
- `run_scaffold_comparison_eval.py` passes to prove Qwen lite/guarded modes do not inherit DeepSeek-sized full scaffold, while DeepSeek full mode retains large dynamic context, protected reserves, and broad enough tool budget;
- future live/offline quality eval should show Qwen-lite is not worse on patch success and reduces parser/tool errors;
- future live/offline quality eval should show DeepSeek full scaffold improves long-task continuation, repair, or research success enough to justify token cost;
- prompt scaffold changes do not increase wrong-tool execution;
- patch apply success does not regress;
- tool result summarization does not lose required evidence;
- sensitive reasoning traces are not persisted.

## Go/No-Go Impact

**Go:** runtime implementation can continue with `ContextBudgetManager v0` and ClaudeCode-like runtime lifecycle.

**No-Go:** full prompt scaffold cannot become default until prompt assembler consumes the budget and model-specific eval gates exist.

**No-Go:** Qwen cannot receive DeepSeek S3 full scaffold by default.

**No-Go:** compatible providers cannot receive native DeepSeek/Qwen scaffold or enter native eval promotion.

## Next Tasks

1. Add Qwen lite-vs-full scaffold eval fixtures.
2. Add DeepSeek full-vs-lite scaffold eval fixtures.
3. Record scaffold level, budget summary, prompt/tool hash, and compaction decision in model-call events.
4. Add budget-aware context retrieval before prompt assembly.
5. Extend prompt assembler warnings into eval telemetry and GUI-visible diagnostics.

## Open Questions

- Should DeepSeek Flash use the same S3 prompt budget as DeepSeek Pro/Max, or a smaller S2.5 mode for latency?
- Should Qwen researcher mode use S2 by default for CSV/report tasks, or S1 plus Python sidecar profiling first?
- Should prompt assembler hard fail on over-budget context, or compact automatically and log a warning before first failure?
