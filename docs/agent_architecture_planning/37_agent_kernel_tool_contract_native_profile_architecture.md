# 37 Agent Kernel, Tool Contract, and Native Profile Architecture

本文解决的问题：把 ClaudeCode/OpenCode 的成熟 agent runtime 思路，和
`tool-usage/deepseek_qwen_tool_calling_harness_engineering_mega_plan.md`
中的 DeepSeek/Qwen tool-calling harness 方案，合并成一个可落地的底层架构。

核心结论：

```text
AgentKernel 管流程。
ToolContractMediationLayer 管工具契约。
NativeModelProfile 管 DeepSeek/Qwen 模型差异。
```

这不是三套系统，而是一个 runtime 的三层职责边界。ClaudeCode/OpenCode 是
AgentKernel 的主要参考；DeepSeek/Qwen mega plan 是 ToolContractMediationLayer
的主要参考；现有 DeepSeek/Qwen native optimization 文档是 NativeModelProfile
的主要参考。

## 1. Problem Statement

当前问题不能再被解释成单个 `file.read` bug。`read` 风暴、工具名错误、
streaming 参数半截、工具失败后 loop 混乱、状态查询重新扫仓库、GUI 只看到一堆
tool event，都是同一个底层问题的不同表现：

- agent loop 缺少明确的 kernel 级任务路由；
- 工具调用缺少统一 mediation layer；
- DeepSeek/Qwen 的模型差异没有通过稳定 profile 接入 runtime；
- runtime hardening 很多是事后补丁，而不是事前 admission 和 workflow policy；
- GUI 连接了真实后端，但后端还没有成熟 control plane。

因此后续不能继续用零散 patch 修复。必须把 agent runtime 重构成清晰的
AgentKernel + ToolContractMediationLayer + NativeModelProfile。

## 2. Design Goals

1. 吸收 ClaudeCode/OpenCode 的 agent lifecycle 纪律。
2. 保留 DeepSeek/Qwen native-first 产品方向。
3. 让 DeepSeek/Qwen 的工具调用可靠性来自 harness，而不是要求模型完美输出 JSON。
4. 所有模型工具调用必须先经过 runtime mediation，再进入执行器。
5. GUI/TUI 只消费事件和提交审批，不拥有工具正确性。
6. 每个失败都必须可分类、可恢复、可 telemetry、可进入 eval。
7. 兼容 provider 可以接入，但不能继承 DeepSeek/Qwen native optimization。

## 3. Non-Goals

- 不照搬 ClaudeCode 的 Anthropic wire format。
- 不把 ClaudeCode 的完整 system prompt 原样塞给所有模型。
- 不把 DeepSeek/Qwen 优化写成到处散落的 `if deepseek` 分支。
- 不用 GUI 层解决 runtime 工具可靠性。
- 不让模型自主管理长工具循环。
- 不把 tool repair 作为危险字段的自动纠错通道。

## 4. High-Level Architecture

```text
User / GUI / TUI
  ↓
RuntimeFacade
  ↓
AgentKernel
  ├─ TurnRouter
  ├─ WorkflowFSM
  ├─ BudgetPolicy
  ├─ ContextManager
  ├─ EvidenceLedger
  ├─ PermissionManager
  ├─ Finalizer
  └─ EventLog
       ↓
ToolManifestBuilder
       ↓
NativeModelProfile
  ├─ PromptScaffoldPolicy
  ├─ ToolSchemaPolicy
  ├─ ParserChainPolicy
  ├─ ReasoningPolicy
  ├─ ContextBudgetPolicy
  ├─ CachePolicy
  └─ RecoveryPolicy
       ↓
ProviderAdapter
       ↓
ToolContractMediationLayer
  ├─ StreamingToolCallAccumulator
  ├─ ToolNameResolver / AliasRegistry
  ├─ ContentToolCallExtractorFallback
  ├─ SchemaValidator
  ├─ IssueGuidedToolInputRepairer
  ├─ RelationalInvariantResolver
  ├─ SafetyPolicy
  └─ ModelReadableToolError
       ↓
ToolDispatcher
       ↓
ToolResult / Artifact / Patch / Command Result
       ↓
AgentKernel decides continue / compact / final answer
```

## 5. Relationship To ClaudeCode And OpenCode

ClaudeCode 的核心启发不是某个单独工具，而是它的 runtime discipline：

- prompt construction、tool admission、streaming tool execution、permission policy、
  context compaction 和 UI state 在一个 turn loop 中协同；
- tool result 是模型继续推理的一部分，不是旁路日志；
- read/search/edit/test/review 是受控闭环；
- 失败被转成可恢复状态，而不是直接让 session 崩溃；
- context 会被主动压缩和重新注入。

OpenCode 的核心启发是更显式的分层：

- agent definitions: build / plan / explore / compaction / title / summary；
- session processor 统一处理 tool-call、tool-result、tool-error、retry、doom loop；
- tool registry 统一 schema decode、truncate、plugin hook；
- provider transform 按模型修正 message、reasoning、tool schema 和 provider options。

ResearchCode 应该吸收这两者的结构，而不是复制 Claude/Anthropic 的协议假设。

## 6. Relationship To The DeepSeek/Qwen Mega Plan

`deepseek_qwen_tool_calling_harness_engineering_mega_plan.md` 的核心不是一个
辅助工具，而是 runtime 必须拥有的 Tool Contract Mediation Layer。

这个 layer 的核心原则必须成为 release-blocking invariant：

1. ToolManifest 必须由 runtime 生成，不能靠 prompt 手写。
2. UnknownTool 是 recoverable tool error，不是 fatal runtime error。
3. Valid inputs are never touched。
4. repair 只在 schema validation 失败后发生，并且只修 issue path。
5. raw validator error 必须转成 model-readable retry guidance。
6. streaming tool calls 必须先 accumulate 和 validate，不能半截执行。
7. DeepSeek thinking-mode tool calls 必须保留 reasoning replay。
8. read-only tools 可以并发；state-changing tools 必须串行。
9. WorkflowFSM 决定每个状态暴露的小工具集。
10. telemetry 和 eval 是 tool repair 的核心信号。

## 7. Core Layer Responsibilities

### 7.1 AgentKernel

AgentKernel 是 runtime 的决策中心。它不执行具体工具，也不直接修复模型参数；
它决定当前 turn 应该处于什么 workflow state，允许什么工具，使用多少预算，
何时继续、压缩、恢复或结束。

Responsibilities:

- classify turn route;
- own WorkflowFSM;
- own tool/read/search/shell/write budgets;
- build ContextBundle;
- request ToolManifest;
- call NativeModelProfile for prompt/schema/parser policy;
- invoke ProviderAdapter;
- pass model outputs into ToolContractMediationLayer;
- dispatch only validated tool calls;
- write EventLog and EvidenceLedger;
- decide continuation, compaction, finalization, or escalation.

AgentKernel must not:

- hard-code DeepSeek or Qwen wire format;
- execute raw model tool calls directly;
- allow GUI to bypass mediation;
- expose unlimited tools by default;
- treat tool failure as session crash by default.

### 7.2 ToolContractMediationLayer

ToolContractMediationLayer sits between provider output and ToolDispatcher.
It owns syntactic and semantic validity of each tool call.

Pipeline:

```text
Provider Response
  ↓
StreamingToolCallAccumulator
  ↓
ToolCallAssembler
  ↓
ToolNameResolver / AliasRegistry
  ↓
ContentToolCallExtractorFallback
  ↓
SchemaValidator
  ↓
IssueGuidedToolInputRepairer
  ↓
SchemaValidator again
  ↓
RelationalInvariantResolver
  ↓
PermissionManager / SafetyPolicy
  ↓
WorkflowFSM admission check
  ↓
ToolDispatcher
```

ToolContractMediationLayer must not decide long-horizon strategy. It returns
one of:

```rust
enum ToolCallOutcome {
    Ready(ValidatedToolCall),
    Repaired(RepairedToolCall),
    Rejected(ModelReadableToolError),
    PermissionRequired(PermissionRequest),
    SafetyDenied(SafetyDenial),
}
```

AgentKernel then decides whether to retry, continue, final-answer, compact, or
ask the user.

### 7.3 NativeModelProfile

NativeModelProfile is the model-family strategy bundle. Only DeepSeek and Qwen
may define native profiles. Compatible providers may define protocol adapters
but cannot override native prompt/parser/context policy.

Interface sketch:

```rust
trait NativeModelProfile {
    fn family(&self) -> NativeModelFamily;
    fn role_profile(&self, role: AgentRole, route: TurnRoute) -> RoleProfile;
    fn prompt_scaffold(&self, role: AgentRole, route: TurnRoute) -> PromptScaffold;
    fn context_policy(&self, route: TurnRoute) -> ContextPolicy;
    fn tool_schema_policy(&self, manifest: &ToolManifest) -> ToolSchemaPolicy;
    fn parser_chain(&self, capability: &ProviderCapability) -> ParserChain;
    fn reasoning_policy(&self, route: TurnRoute) -> ReasoningPolicy;
    fn cache_policy(&self) -> CachePolicy;
    fn recovery_policy(&self, error: ModelOrToolError) -> RecoveryPolicy;
    fn eval_tags(&self) -> EvalTags;
}
```

## 8. Turn Routes

AgentKernel must classify each turn before exposing tools.

Initial route set:

| Route | Purpose | Tools | Budget |
|---|---|---|---|
| `DirectAnswer` | greeting/simple Q&A | none | 1 model call |
| `ProjectStatus` | project completion/status query | git/status/summary only | very small |
| `ReadOnlyExplore` | bounded codebase inspection | repo/search/read | small/medium |
| `PlanOnly` | design without writes | repo/search/read/plan | bounded |
| `EditTask` | patch-producing task | read/search/patch/test | governed |
| `LongHorizonTask` | accepted long implementation | full workflow FSM | contract-bound |
| `ReviewTask` | code review | diff/read/search/test readonly | bounded |
| `ResearchTask` | data/literature/report | research tools | contract-bound |

This route controls:

- allowed workflow states;
- active tool manifest;
- max iterations;
- max total tool calls;
- max read/search calls;
- context bundle size;
- reasoning replay allowance;
- finalizer behavior.

## 9. Workflow FSM

WorkflowFSM owns multi-step tool loops. The model proposes the next action; the
runtime decides whether that action is legal in the current state.

Baseline workflow:

```text
Created
→ Classifying
→ Planning
→ RetrievingContext
→ ProposingPatch
→ ReviewingDiff
→ ApplyingPatch
→ RunningTests
→ DiagnosingFailure
→ Repairing
→ Reviewing
→ Finalizing
→ Completed / Failed / WaitingForUser
```

State-specific tool exposure:

| State | Visible tools |
|---|---|
| `Planning` | `repo.map`, `file.search`, `git.status`, `plan.write` |
| `RetrievingContext` | `file.read`, `file.grep`, `symbol.search`, `repo.map` |
| `ProposingPatch` | `patch.propose`, `file.read` |
| `ReviewingDiff` | `git.diff.readonly`, `file.read`, `search.ripgrep` |
| `ApplyingPatch` | `patch.apply` |
| `RunningTests` | `shell.command` with test-classified commands |
| `DiagnosingFailure` | `file.read`, `search.ripgrep`, `shell.command` readonly/test |
| `Finalizing` | no tools |

`ProjectStatus` should not enter full `RetrievingContext` by default. It should
consume status sources first:

- latest `agent.turn_summary`;
- implementation status docs;
- recent event summaries;
- git status/diff metadata;
- latest test/check results.

Only if status evidence is stale or absent may it do a small number of targeted
reads.

## 10. ToolManifest Single Source Of Truth

ToolManifest is generated by runtime per turn.

Build chain:

```text
ToolRegistry
  ↓
PermissionManager
  ↓
TaskContract
  ↓
WorkflowFSM state
  ↓
TurnRoute budget
  ↓
ProviderCapabilityMatrix
  ↓
NativeModelProfile ToolSchemaPolicy
  ↓
ToolManifestBuilder
```

Manifest type:

```rust
struct ToolManifest {
    manifest_id: String,
    version: String,
    provider_id: String,
    model_id: String,
    native_family: Option<NativeModelFamily>,
    workflow_state: WorkflowState,
    turn_route: TurnRoute,
    task_contract_id: Option<String>,
    tools: Vec<ToolSpec>,
    aliases: Vec<ToolAlias>,
    permission_summary: PermissionSummary,
    manifest_hash: String,
}
```

Manifest invariants:

- every model-visible tool must be registered;
- every alias must resolve to a registered canonical tool;
- every exposed tool must be allowed by TaskContract and WorkflowFSM;
- manifest hash must be logged on every model call;
- prompts must not mention tools absent from the manifest.

## 11. Tool Naming And Alias Policy

Use stable canonical names internally:

Read-only:

```text
file.list
file.read
file.search
file.grep
file.stat
repo.map
symbol.search
symbol.definition
symbol.references
git.status
git.diff.readonly
```

State-changing:

```text
patch.propose
patch.apply
file.write
file.edit
shell.command
python.run
```

Provider-facing names may differ. Alias resolution must be explicit and logged.

Examples:

```text
read_source_code -> file.read
read_file        -> file.read
file_read        -> file.read
list_directory   -> file.list
grep             -> file.grep
rg.search        -> file.grep
repo_map         -> repo.map
```

Unknown tool behavior:

- return `ModelReadableToolError`;
- include canonical alternatives;
- log `tool.name.unknown`;
- do not crash the runtime;
- do not silently execute a fuzzy match for state-changing tools.

## 12. Streaming Tool Calls

Streaming tool calls must not execute until fully assembled.

Required events:

```text
tool_call.delta_received
tool_call.name_detected
tool_call.arguments_delta_received
tool_call.arguments_completed
tool_call.assembly_completed
tool_call.validation_started
tool_call.validation_failed
tool_call.validation_passed
```

Execution gates:

1. stream block complete;
2. JSON parse complete or recoverable;
3. schema validation passes;
4. issue-local repair applied only if needed;
5. validation passes again;
6. relational invariants resolved;
7. permission and safety pass;
8. WorkflowFSM admits the tool;
9. read-only parallel policy or state-changing serialization policy passes.

## 13. Validate-Then-Repair

Hard rule:

```text
Valid inputs are never touched.
```

Algorithm:

1. parse arguments as JSON;
2. validate raw input;
3. execute unchanged if valid;
4. if invalid, inspect validation issue paths;
5. apply only issue-local repair rules;
6. validate again;
7. execute repaired input only if valid;
8. otherwise return `ModelReadableToolError`.

P0 repair catalogue:

- strip optional null when schema disallows null;
- parse stringified JSON array when schema expects array;
- convert bare string to `array<string>` only for safe list fields;
- unwrap markdown auto-link paths for path fields;
- fill read range defaults through relational resolver.

No auto-repair:

- shell command string;
- file write content;
- patch body;
- destructive path;
- network URL unless explicitly schema-marked safe;
- permission policy;
- security-sensitive fields.

## 14. ModelReadableToolError

Raw validator errors must not be returned as the only model feedback.

Error shape:

```rust
struct ModelReadableToolError {
    error_code: String,
    tool_name: String,
    recoverable: bool,
    message: String,
    invalid_fields: Vec<String>,
    retry_hint: String,
    retry_example: Option<serde_json::Value>,
}
```

Examples:

- unknown tool: suggest canonical tool names;
- path is directory: suggest `file.list` before `file.read`;
- invalid path field: explain raw path, not markdown link;
- schema field null: show omitted-field retry example;
- permission denied: explain waiting for user or alternative readonly path.

## 15. DeepSeek Native Profile

DeepSeek is not a compatible provider with a nicer prompt. It is a native
optimized family.

DeepSeekProfile responsibilities:

- use native DeepSeek tool calling where available;
- support OpenAI-compatible and Anthropic-compatible transport as wire modes,
  not as native behavior definitions;
- preserve and replay `reasoning_content` for tool-call turns;
- sanitize reasoning before persistence;
- prefer native structured tool calls;
- support DSML/XML fallback as candidate extraction, not trusted execution;
- use JSON argument repair for known streaming failure modes;
- use stable sorted/memoized tool catalog;
- keep prompt prefix cache-stable;
- log cache hit/miss and reasoning replay tokens;
- use 256K safe cap until deployment eval proves larger safe context;
- target prepared requests below 240K;
- emit live compaction telemetry at 192K;
- split Pro/Flash/Non-think roles;
- gate strict tool mode by ProviderCapabilityMatrix.

DeepSeek parser chain:

```text
native structured tool call
  -> OpenAI-compatible tool_calls
  -> Anthropic-compatible tool_use
  -> DSML/XML candidate extraction
  -> content tool-call candidate extraction
  -> ModelReadableToolError
```

DeepSeek content fallback safety:

- only when no normal tool calls exist;
- only if WorkflowFSM expects a tool;
- only exact canonical or alias tool names;
- only JSON-like arguments;
- never direct execution;
- state-changing extracted calls require explicit permission unless already
  allowed by TaskContract.

## 16. Qwen Native Profile

Qwen must not receive DeepSeek S3 full scaffold by default. Its reliability
should come from strong runtime scaffold, narrow tools, strict schemas, small
steps, validators, and reviewer loop.

QwenProfile responsibilities:

- Qwen3.6-27B is canonical target;
- use Qwen chat-template semantics when deployment supports it;
- require qwen3 reasoning parser and qwen3_coder tool parser where available;
- check serving stack capability at setup time;
- use deterministic Qwen tool JSON;
- keep active tool set narrow;
- prefer patch-sized edits;
- preserve thinking only when useful and supported;
- require tests/reviewer for guarded modes;
- block promotion on wrong-tool execution.

Qwen route defaults:

| Route | Scaffold |
|---|---|
| `DirectAnswer` | S1 Fast, no tools |
| `ProjectStatus` | S1/S2, narrow status tools |
| `ReadOnlyExplore` | S2 Guarded |
| `EditTask` | S1 executor + reviewer |
| `LongHorizonTask` | S2 with small steps |

## 17. ProviderCapabilityMatrix

Provider behavior differs even within the same model family. Runtime must
measure capabilities before agent execution.

Type sketch:

```rust
enum ProviderToolCallingMode {
    DeepSeekNative,
    FireworksOpenAICompatible,
    DeepSeekAnthropicCompatible,
    CustomOpenAICompatible,
    LocalDeepSeek,
    QwenNative,
    VllmQwen,
    None,
}

struct ToolCallingCapabilities {
    supports_tools: bool,
    supports_streaming_tools: bool,
    supports_parallel_tool_calls: bool,
    supports_tool_choice_required: bool,
    supports_tool_choice_specific: bool,
    supports_strict_json_schema: bool,
    supports_reasoning_replay: bool,
    supports_native_deepseek_thinking: bool,
    tool_parser: String,
    reasoning_parser: Option<String>,
}
```

Startup checks:

- endpoint reachable;
- tool calling works;
- streaming tool calls work or are disabled;
- tool choice behavior confirmed;
- strict mode confirmed if enabled;
- reasoning replay confirmed for DeepSeek thinking;
- Qwen parser/template flags confirmed for Qwen native.

## 18. Context And Evidence Policy

Context is a curated input package, not a dump of history.

AgentKernel owns:

- ContextBundle;
- EvidenceLedger;
- status summary;
- compacted summaries;
- tool result artifact refs;
- read/search/git summaries;
- current plan and TaskContract.

DeepSeekProfile owns:

- reasoning replay reserve;
- prefix cache stable order;
- compaction threshold;
- tool schema hash stability;
- cache telemetry.

QwenProfile owns:

- smaller prompt scaffold;
- narrow active tools;
- parser/template metadata;
- thinking preservation policy.

Project status queries must use status evidence first:

```text
agent.turn_summary
implementation status docs
recent event log summary
git.status / git.diff metadata
latest test result summary
```

They must not default to broad source reads.

## 19. Parallel Tool Policy

Parallel allowed:

```text
file.read
file.search
file.grep
repo.map
symbol.search
git.diff.readonly
git.status
```

Serialized required:

```text
patch.apply
file.write
file.edit
shell.command
python.run
matlab.eval
computer.click
external app automation
```

State-changing checks:

- TaskContract allows it;
- PermissionManager allows it;
- SafetyPolicy allows it;
- base hash exists when editing;
- stale file check passes;
- rollback plan exists where applicable;
- event log record is written before and after execution.

## 20. Event Model

Required high-level events:

```text
agent.turn_classified
workflow.state_changed
tool_manifest.built
model.request_prepared
model.stream_delta
tool_call.delta_received
tool_call.assembled
tool.name.alias_resolved
tool.name.unknown
tool.input.validation_failed
tool.input.repair_applied
tool.permission.checked
tool.execution.started
tool.execution.completed
tool.execution.failed
tool.error.model_readable
reasoning.replay.recorded
reasoning.replay.missing
context.compaction.started
context.compaction.completed
agent.loop_finalizer
agent.turn_summary
```

GUI/TUI must show enough of this lifecycle to make tool failures debuggable.
Tool lifecycle events must not be hidden behind only final text.

## 21. ToolDoctor

ToolDoctor is required for debugging harness failures.

Checks:

- registered tools;
- visible tools;
- allowed tools;
- prompt mentions of tool names;
- AGENTS.md tool mentions;
- aliases;
- provider capability;
- workflow state;
- schema validation status;
- recent unknown tool errors;
- recent repair rates;
- reasoning replay status.

Output should answer:

- why the model saw this tool;
- why a requested tool was rejected;
- whether a tool was prompt-only;
- whether alias mapping caused behavior;
- whether provider capability disabled a mode.

## 22. Eval Gates

Release-blocking evals:

1. UnknownTool recovery.
2. Alias resolution.
3. Streaming tool-call assembly.
4. Content tool-call fallback candidate extraction.
5. Optional null repair.
6. Stringified array repair.
7. Bare string array repair.
8. Markdown path unwrap.
9. Offset/limit relational default.
10. Model-readable error retry.
11. No mutation of valid inputs.
12. No mutation of `file.write.content`.
13. No auto-repair of shell command.
14. DeepSeek `reasoning_content` replay.
15. Provider capability mismatch detection.
16. ToolDoctor mismatch detection.
17. GUI/TUI event stream visibility.
18. Read-only parallel execution.
19. State-changing serialization.
20. ProjectStatus does not broad-read by default.
21. Readstorm finalizer and budget enforcement.
22. UltraReview classifies harness failure separately from code bug.

Release must block if:

- UnknownTool is fatal;
- model-visible tool is not registered;
- streaming tool args can execute before completion;
- valid inputs are mutated;
- dangerous command is auto-repaired;
- raw validator blob is returned as sole feedback;
- DeepSeek reasoning replay is dropped after tool call;
- Qwen native mode runs without parser/template capability check;
- GUI hides tool lifecycle events;
- status query performs unbounded reads.

## 23. Integration With Current Codebase

Current files likely map as follows:

| Current file | Future role |
|---|---|
| `crates/runtime/src/runtime_facade.rs` | public boundary into AgentKernel |
| `crates/runtime/src/native_agent_loop.rs` | migrate from monolithic loop to execution engine pieces |
| `crates/runtime/src/tool_call_parser.rs` | parser-chain component under NativeModelProfile/TCML |
| `crates/runtime/src/tool_execution.rs` | ToolDispatcher and concrete tool executors |
| `crates/runtime/src/tool_dispatcher.rs` | read-only parallel/state-changing serialization policy |
| `crates/runtime/src/provider_response_adapter.rs` | ProviderAdapter and sanitation layer |
| `crates/runtime/src/context_budget.rs` | ContextBudgetPolicy |
| `crates/runtime/src/prompt_assembler.rs` | PromptScaffoldPolicy consumer |
| `crates/kernel/src/tool.rs` | ToolRegistry source of truth |
| `desktop/src-tauri/src/main.rs` | must call AgentKernel route, not raw unlimited loop |
| `desktop/src/runtime/localRuntimeClient.ts` | GUI event consumer |

Do not delete existing hardening. Rehouse it:

- duplicate observation suppression becomes EvidenceLedger/read cache policy;
- non-progress finalizer becomes AgentKernel finalizer policy;
- model-readable tool errors become TCML output type;
- directory-read recovery becomes relational invariant and retry guidance;
- `agent.turn_summary` becomes status evidence source.

## 24. Implementation Plan

### Phase 0: Architecture Lock

Deliverables:

- adopt this document as architecture source;
- promote the mega plan into `docs/agent_architecture_planning/42_tool_calling_harness_engineering_mega_plan.md`
  or explicitly cross-link it;
- define module boundaries and prohibited bypasses;
- add ADR: raw model tool calls may not bypass TCML.

### Phase 1: ToolManifest And Alias Foundation

Deliverables:

- runtime-generated ToolManifest from ToolRegistry;
- manifest filtered by TurnRoute, WorkflowFSM, TaskContract, permissions, and provider capability;
- manifest hash in model-call events;
- alias resolver with recoverable UnknownTool;
- ToolDoctor minimal output.

### Phase 2: Streaming And Validation

Deliverables:

- StreamingToolCallAccumulator;
- no execution before full assembly;
- SchemaValidator;
- ModelReadableToolError;
- GUI/TUI tool lifecycle events.

### Phase 3: Validate-Then-Repair

Deliverables:

- issue-guided repairer;
- P0 repair catalogue;
- no mutation of valid inputs;
- no repair of dangerous/state-changing fields;
- repair telemetry and eval fixtures.

### Phase 4: WorkflowFSM And Budgets

Deliverables:

- TurnRouter;
- state-specific active tools;
- ProjectStatus route;
- read/search/tool budgets;
- readstorm detector;
- finalizer after budget/non-progress;
- evidence-ledger-backed continuation.

### Phase 5: DeepSeek/Qwen Native Profiles

Deliverables:

- DeepSeekReasoningReplayManager;
- DeepSeek parser chain with DSML/content fallback candidates;
- DeepSeek cache/prefix telemetry;
- Qwen parser/template capability check;
- Qwen narrow-tool S1/S2 policies;
- native eval tags on all model calls.

### Phase 6: GUI Product Integration

Deliverables:

- show active mode: DeepSeek Mode / Qwen3.6-27B Mode;
- show turn route and workflow state;
- show tool budget and remaining budget;
- show tool lifecycle events;
- show model-readable errors;
- show evidence/status source for ProjectStatus.

## 25. Architectural Invariants

1. Model proposes; runtime executes.
2. No raw model tool call bypasses TCML.
3. ToolManifest is the only model-visible tool source.
4. Valid tool inputs are never mutated.
5. Invalid inputs are repaired only at schema issue paths.
6. Dangerous/state-changing inputs are never auto-repaired.
7. UnknownTool is recoverable.
8. Streaming tool calls execute only after complete assembly and validation.
9. WorkflowFSM owns long tool loops.
10. Active tool sets are small and state-specific.
11. Read-only tools may parallelize; state-changing tools serialize.
12. DeepSeek reasoning replay is preserved or blocked before provider call.
13. Qwen native mode requires parser/template capability evidence.
14. Compatible providers cannot override native profile policy.
15. GUI/TUI consumes event truth; it does not invent runtime state.

## 26. Final Architecture Decision

ResearchCode should not choose between ClaudeCode/OpenCode architecture and the
DeepSeek/Qwen tool-calling mega plan. They solve different layers:

```text
ClaudeCode/OpenCode:
  mature agent runtime lifecycle.

DeepSeek/Qwen mega plan:
  reliable tool-calling harness for open/native models.

ResearchCode:
  AgentKernel + ToolContractMediationLayer + NativeModelProfile.
```

This is the target architecture for a DeepSeek/Qwen-first local AI agent
workbench. It preserves ClaudeCode-strength runtime discipline while making
DeepSeek/Qwen native optimization a first-class product advantage instead of a
prompt-only patch.
