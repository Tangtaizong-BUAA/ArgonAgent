# 39 DeepSeek-Native Agent Kernel: Architecture and Implementation Plan

doc 37 给了三层骨架。doc 38 把骨架对齐到 Claude Code 的"manifest 不切，权限切"
和成熟度细节。但两者都没回答最核心的一个问题：

> 这个骨架本身**怎么就让 DeepSeek 用起来顺手**？

Claude Code 是**为 Claude 设计**的——长 system prompt、Anthropic tool_use block、
extended thinking、prompt cache 用 `cache_control` 标记、Plan mode 信赖 Claude
的指令跟随能力。它**不是**一个通用 agent runtime，是 Claude-native runtime。

我们要做的事情对称：**为 DeepSeek 设计** agent kernel——从流式协议、思考链回放、
工具调用形态、JSON 修复、缓存前缀稳定性、温度调度，到角色拆分（Pro/Flash），
全部按 DeepSeek 的实际行为模式从底层往上塑形。同时保留 Claude Code 的架构
discipline（分层、不变式、事件流、不可绕过的 mediation 层）。

本文给的是**最终架构 + 实施计划**，不是选项调研。读完这篇，下一步该写哪个
crate、哪个 trait、哪个 type，应该是清楚的。

---

## 0. Thesis

```text
DeepSeek-native agent kernel = Claude-Code-grade discipline + DeepSeek-shaped primitives
```

具体三句话：

1. **架构纪律抄 Claude Code**：分层不变、TCML 不可绕过、manifest 全开权限切、
   ToolDoctor、event 是单一真相源、自动 compaction、subagent 隔离。
2. **底层原语按 DeepSeek 塑形**：StreamProcessor 默认懂 reasoning_content +
   tool_calls.delta + DSML 漏出；CachePrefixPolicy 三段式；AliasRegistry 收录
   DeepSeek 实际错名习惯；RepairCatalog 覆盖 DeepSeek/Qwen 真实失败模式；
   RoleSplit 让 Pro/Flash 分工。
3. **不做的事**：不抄 Anthropic wire format；不抄 Claude 长 system prompt；
   不当通用 multi-provider 适配框架——通用化和深度优化 trade-off 在深度优化
   这一边。

---

## 1. DeepSeek Behavior Catalog（13 类问题，凡 kernel 必须懂）

按"模型层 / 协议层 / 内容层"三类列出 kernel 必须知道并主动处理的 DeepSeek 行为
特征。每条都有具体证据来源（社区 issue、官方文档、实测）。

### 模型层

#### B1. reasoning_content 必须在下一轮 tool_use 链中回传
- 来源：DeepSeek 官方 thinking_mode 文档、OpenCode #24442
- 行为：thinking 模式下 tool 调用后，下一次请求若不带前一轮 reasoning_content
  → API 400
- Kernel 责任：ReasoningReplayManager 在每个 tool 链 turn 之间存→注入

#### B2. reasoning_content 长度可达 10K+ tokens
- 来源：实测
- 行为：thinking-heavy turn 会让 reasoning 段反而比 visible content 大十倍
- Kernel 责任：reasoning 单独算 budget；在 compaction 时**不能**把 reasoning
  和 visible 一起截断（会破坏 B1 的回放）

#### B3. Pro/Flash 能力梯度
- 来源：DeepSeek 定价、capability 表
- 行为：Pro 推理强、贵；Flash 快、便宜
- Kernel 责任：RoleSplit——Executor 默认 Pro，Compactor 用 Flash，Title/Summary
  用 Flash

#### B4. 温度敏感
- 来源：函数调用最佳实践、社区
- 行为：tool calling 在 temp≤0.3 显著更稳；narrative 在 temp 0.7 更自然
- Kernel 责任：TemperatureSchedule——按 turn 阶段切

### 协议层

#### B5. tool_calls.delta 流式跨 chunk
- 来源：OpenAI compatible streaming spec
- 行为：function.arguments 可能在多个 SSE chunk 中以拼接形式出现，单 chunk
  里 JSON 不闭合
- Kernel 责任：StreamingToolCallAccumulator——按 `index` 累加直到 finish_reason

#### B6. DSML / `<tool_calls>` 在 content 中漏出
- 来源：DeepSeek-V3 issue #1244、近期实测（HTML 漏到可见文本）
- 行为：模型偶尔不走 structured tool_calls，把 DSML/XML 风格调用塞进 content
- Kernel 责任：状态化 DsmlChunkFilter（已实现）+ ContentToolCallExtractor 兜底
  抽取候选（不直接执行）

#### B7. strict mode 仅 beta 端点支持
- 来源：DeepSeek 函数调用文档
- 行为：`base_url=...beta` + `strict:true` 才生效；非 beta 端点 strict 不起作用
- Kernel 责任：ProviderCapabilityMatrix 探测；strict 不可用时仍通过 runtime
  validator 兜底

#### B8. 同一 model 在不同 provider 上 capability 不同
- 来源：vLLM、Fireworks、官方端点差异
- 行为：vLLM 可能不支持 `tool_choice="required"`；Fireworks 可能 streaming tools 部分支持
- Kernel 责任：启动时探测；ToolCallingCapabilities 缓存到 session

### 内容层

#### B9. 工具名错误小目录（高频）
- 来源：实测
- 高频错误：
  - `read` / `Read` / `read_file` / `readFile` / `fileRead` → 应为 `file.read`
  - `ls` / `list` / `ListDirectory` / `ListDir` → 应为 `file.list_directory`
  - `grep` / `search` / `rg` / `SearchFiles` → 应为 `search.ripgrep`
  - `bash` / `shell` / `exec` / `RunCommand` → 应为 `shell.command`
  - `plan` / `Plan` / `enter_plan_mode` → 应为 `plan.enter`
- Kernel 责任：AliasRegistry 全量收录 + 大小写 + snake/dot 互转

#### B10. JSON 形状错误小目录
- 来源：mega plan §2.5、社区
- 高频错误：
  - `null` for optional → 应省略字段
  - `"[\"a\",\"b\"]"` → 应直接 `["a","b"]`
  - `{}` 当 `[]` → 应空数组
  - 单字符串当数组：`"a"` → 应 `["a"]`
  - markdown 链接路径：`/path/[file.md](http://file.md)` → 应裸路径
- Kernel 责任：IssueGuidedRepairer 仅在 schema 失败后按 issue path 修

#### B11. 关系型不变量错误
- 来源：mega plan §2.6
- 高频错误：
  - `limit` 无 `offset`
  - `start > end`
  - `file.edit` 无 `base_hash`（模型不知道 hash）
- Kernel 责任：RelationalInvariantResolver——透明默认 + base_hash 在 dispatch
  前 runtime 注入

#### B12. tool 失败后 doom loop
- 来源：实测（"重复同一调用直到死"）
- 行为：模型遇到工具错误，倾向重复调用而不是换策略
- Kernel 责任：observation cache + 重复 batch 给"已观察过"提示（**不**禁用工具）

#### B13. 长 context 性能拐点
- 来源：DeepSeek 长 context 实测
- 行为：256K 上限但 192K 后准确率明显下降
- Kernel 责任：Compactor 在 192K（不是 80%×256K=204K）触发；保留近 4 turn 完整

---

## 2. Architecture Layers（重述，DeepSeek-first）

```text
┌─────────────────────────────────────────────────────────────┐
│ User / GUI / TUI                                            │
└──────────────────────────────┬──────────────────────────────┘
                               ↓
┌─────────────────────────────────────────────────────────────┐
│ RuntimeFacade                                               │
│  - public API                                               │
│  - emits event stream                                       │
└──────────────────────────────┬──────────────────────────────┘
                               ↓
┌─────────────────────────────────────────────────────────────┐
│ AgentKernel                                                 │
│  TurnRouter         BudgetPolicy        PermissionPolicy    │
│  ContextManager     EvidenceLedger      Compactor           │
│  Finalizer          EventLog            TurnState           │
└──────────────────────────────┬──────────────────────────────┘
                               ↓
┌─────────────────────────────────────────────────────────────┐
│ NativeProfile (DeepSeek / Qwen)                             │
│  PromptScaffold       CachePrefixPolicy                     │
│  ToolSchemaPolicy     RoleSplit + TemperatureSchedule       │
│  ReasoningReplayMgr   StreamProcessor                       │
└──────────────────────────────┬──────────────────────────────┘
                               ↓
┌─────────────────────────────────────────────────────────────┐
│ ProviderAdapter (HTTP/SSE transport, retry, timeout)        │
└──────────────────────────────┬──────────────────────────────┘
                               ↓
┌─────────────────────────────────────────────────────────────┐
│ ToolContractMediationLayer (TCML)                           │
│  StreamingToolCallAccumulator                               │
│  ContentToolCallExtractor                                   │
│  AliasRegistry → ToolNameResolver                           │
│  SchemaValidator → IssueGuidedRepairer → SchemaValidator    │
│  RelationalInvariantResolver                                │
│  ModelReadableToolError 工厂                                │
└──────────────────────────────┬──────────────────────────────┘
                               ↓
┌─────────────────────────────────────────────────────────────┐
│ PermissionGate (执行点)                                     │
│  - 按 PermissionMode 决策 Allow/Ask/Deny                    │
└──────────────────────────────┬──────────────────────────────┘
                               ↓
┌─────────────────────────────────────────────────────────────┐
│ ToolDispatcher                                              │
│  - read-only 并发，state-changing 串行                      │
└──────────────────────────────┬──────────────────────────────┘
                               ↓
┌─────────────────────────────────────────────────────────────┐
│ ToolExecutors (file.read, file.write, shell.command, ...)   │
└─────────────────────────────────────────────────────────────┘
```

DeepSeek-specific 关键路径：

- B1, B2 → **ReasoningReplayManager**（NativeProfile 内）
- B3, B4 → **RoleSplit + TemperatureSchedule**（NativeProfile 内）
- B5, B6 → **StreamProcessor + ContentToolCallExtractor**
- B7, B8 → **ProviderCapabilityMatrix**（启动时探测）
- B9 → **AliasRegistry**（TCML 入口）
- B10, B11 → **RepairCatalog + RelationalInvariantResolver**
- B12 → **ObservationCache + 不禁用工具**（已实现）
- B13 → **Compactor**（在 192K 触发）

---

## 3. Data Model（核心 Rust 类型）

### 3.1 TurnState：单一状态源

```rust
pub struct TurnState {
    // identity
    pub session_id: String,
    pub turn_index: u32,
    pub started_at: Instant,

    // routing
    pub route: TurnRoute,
    pub mode: PermissionMode,
    pub role: AgentRole,                  // Executor / Compactor / Reviewer

    // budget tracking
    pub budget: TurnBudget,
    pub iterations: u32,
    pub tool_calls_used: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub reasoning_tokens: u64,            // 单独跟踪，与 visible 分开

    // batch info (informational, not for ban decisions)
    pub seen_tool_batches: Vec<String>,
    pub observation_cache: ObservationCache,
    pub last_tool_batch: Vec<ToolBatchEntry>,

    // DeepSeek native bits
    pub reasoning_replay: ReasoningReplayState,
    pub stream_state: StreamProcessorState,
    pub provider_capabilities: ToolCallingCapabilities,

    // permission flow
    pub awaiting_user: Option<AwaitingUserRequest>,

    // emit tracking
    pub emitted_event_count: usize,
}
```

### 3.2 ToolCallLifecycle：多阶段事件

```rust
pub enum ToolCallPhase {
    Streaming { partial_args: String },
    Assembled { call: AssembledCall },
    NameResolved { canonical: String, alias_used: bool },
    SchemaChecked { issues: Vec<ValidationIssue> },
    Repaired { applied_rules: Vec<&'static str> },
    PermissionEvaluated { decision: PermissionDecision },
    Dispatched { tool_use_id: String },
    Completed { result: ToolExecutionResult },
}

pub struct ToolCallTrace {
    pub call_id: String,
    pub phases: Vec<(Instant, ToolCallPhase)>,
}
```

每个 phase 转换都 emit 一条结构化 event；GUI/TUI 据此渲染（doc 38 §22）。

### 3.3 ModelReadableToolError 完整目录

```rust
#[derive(Serialize)]
pub struct ModelReadableToolError {
    pub error_code: ToolErrorCode,
    pub tool_name: String,
    pub short_message: String,
    pub field_errors: Vec<FieldError>,
    pub retry_hint: String,
    pub retry_example: Option<serde_json::Value>,
    pub retryable: bool,
    pub counts_against_budget: bool,    // 命名错误 false，执行错误 true
}

pub enum ToolErrorCode {
    UnknownTool,
    PlanModeRequired,
    PermissionDenied,
    SafetyDenied,
    SchemaValidationFailed,
    MalformedJson,
    RelationalInvariantFailed,
    ToolExecutionFailed,
    BudgetExhausted,
}
```

每个 code 都有标准 `retry_hint` 和 `retry_example` 模板（§13 给详表）。

### 3.4 PermissionMode

```rust
pub enum PermissionMode {
    Default,           // write/shell ask
    Plan,              // write/shell deny → "use plan.enter"
    AcceptEdits,       // write allow, shell ask
    DontAsk,           // 仅 allow-list 中的 tool
    BypassPermissions, // 全 allow（仅 dev）
}
```

### 3.5 AgentRole + Model 配对

```rust
pub enum AgentRole {
    Executor,    // 主任务执行
    Compactor,   // 压缩历史
    Reviewer,    // 审查产出
    Titler,      // 起 session 标题
    Summarizer,  // 摘要工具结果
}

pub struct RoleModelMap {
    pub executor: ModelEndpoint,        // 默认 deepseek-chat (Pro)
    pub compactor: ModelEndpoint,       // 默认 deepseek-chat-flash
    pub reviewer: ModelEndpoint,
    pub titler: ModelEndpoint,
    pub summarizer: ModelEndpoint,
}
```

---

## 4. StreamProcessor：DeepSeek-native 心脏

这是与 Claude Code 区别最大的组件。Claude 的 streaming 简单（content blocks
依次到达），DeepSeek 的 streaming 同时混合三类事件：

- `delta.content` 文本流
- `delta.reasoning_content` 思考流（DeepSeek thinking 独有）
- `delta.tool_calls[i].function.arguments` 工具参数流（按 index 累加）
- 偶发：DSML/`<tool_calls>` 漏到 `delta.content` 中（B6）

StreamProcessor 是状态机，按 chunk 输入，输出三个事件流：

```rust
pub struct StreamProcessor {
    state: StreamProcessorState,
    dsml_filter: DsmlChunkFilter,
    accumulator: ToolCallAccumulator,
    reasoning_buffer: String,
    visible_buffer: String,
}

pub struct StreamProcessorState {
    pub inside_dsml: bool,
    pub partial_calls: BTreeMap<u32, PartialToolCall>, // index → partial
    pub finish_reason: Option<String>,
    pub last_chunk_at: Instant,
}

pub enum StreamEvent {
    VisibleDelta(String),                  // 已过滤 DSML，安全可见
    ReasoningDelta(String),                // 思考增量
    ToolCallPartial { index: u32, name_so_far: Option<String>, args_so_far: String },
    ToolCallAssembled(AssembledCall),
    ContentToolCallCandidate(ExtractedContentCall), // DSML 抽取的兜底
    StreamCompleted { finish_reason: String },
}

impl StreamProcessor {
    pub fn ingest(&mut self, chunk: SseChunk) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        // 1. 处理 delta.reasoning_content
        if let Some(rc) = chunk.reasoning_delta() {
            self.reasoning_buffer.push_str(rc);
            events.push(StreamEvent::ReasoningDelta(rc.to_string()));
        }
        // 2. 处理 delta.tool_calls[i]
        for tc_delta in chunk.tool_call_deltas() {
            self.accumulator.merge(tc_delta);
            if let Some(assembled) = self.accumulator.try_complete(tc_delta.index) {
                events.push(StreamEvent::ToolCallAssembled(assembled));
            } else {
                events.push(StreamEvent::ToolCallPartial { /* ... */ });
            }
        }
        // 3. 处理 delta.content（含 DSML 过滤 + content tool call 兜底）
        if let Some(text) = chunk.content_delta() {
            let filtered = self.dsml_filter.filter(text);
            self.visible_buffer.push_str(&filtered);
            events.push(StreamEvent::VisibleDelta(filtered));
        }
        // 4. finish_reason → 收尾
        if let Some(reason) = chunk.finish_reason() {
            self.state.finish_reason = Some(reason.clone());
            // 如果 finish_reason=stop 但 visible_buffer 含 DSML 痕迹且无 tool_calls
            // → ContentToolCallExtractor 提取候选
            if reason == "stop" && self.accumulator.is_empty() {
                if let Some(candidate) = ContentToolCallExtractor::scan(&self.visible_buffer) {
                    events.push(StreamEvent::ContentToolCallCandidate(candidate));
                }
            }
            events.push(StreamEvent::StreamCompleted { finish_reason: reason });
        }
        events
    }
}
```

### 4.1 DsmlChunkFilter（已落地，需迁移到 native_profile/deepseek/）

跨 chunk 状态：

```rust
#[derive(Default)]
pub struct DsmlChunkFilter {
    inside: bool,
}

impl DsmlChunkFilter {
    const STARTS: &'static [&'static str] = &[
        "<｜｜DSML｜｜tool_calls>", "<tool_call>", "<|tool_calls_section_begin|>",
    ];
    const ENDS: &'static [&'static str] = &[
        "</｜｜DSML｜｜tool_calls>", "</tool_call>", "<|tool_calls_section_end|>",
    ];
    pub fn filter(&mut self, chunk: &str) -> String { /* state machine */ }
}
```

### 4.2 ToolCallAccumulator

按 `index` 累加 partial deltas，到 `finish_reason` 才尝试 parse：

```rust
pub struct PartialToolCall {
    pub index: u32,
    pub id: Option<String>,
    pub name: Option<String>,
    pub args_buffer: String,
    pub started_at: Instant,
}

impl ToolCallAccumulator {
    pub fn merge(&mut self, delta: ToolCallDelta) { /* 拼接到 index 对应的 partial */ }
    pub fn try_complete(&mut self, index: u32) -> Option<AssembledCall> {
        // 仅当 args_buffer 是闭合 JSON 才返回
    }
    pub fn finalize_all(&mut self) -> Vec<AssembledCall> { /* 在 finish_reason 调用 */ }
}
```

### 4.3 ContentToolCallExtractor（兜底，仅候选）

```rust
pub struct ExtractedContentCall {
    pub raw_text: String,
    pub tool_name: String,
    pub args_text: String,
    pub confidence: Confidence,        // High / Medium / Low
    pub extraction_pattern: &'static str,
}

impl ContentToolCallExtractor {
    pub fn scan(content: &str) -> Option<ExtractedContentCall> {
        // 1. 找 DSML/<tool_calls>/<|tool_call|> 闭合标签
        // 2. 解析其中 name 和 args
        // 3. 仅当 args 看起来是 JSON-like 才返回
        // 4. 不调用任何 executor
    }
}
```

候选交给 AgentKernel 决定：

- 如果当前 turn 期望 tool（route != DirectAnswer）且无正常 tool_calls → 把候选
  喂给 TCML 走完整 pipeline；若 state-changing 类，强制走 PermissionMode.Default
  的 ask 路径（即使是 AcceptEdits 也 ask）。
- 否则忽略，仅 emit `tool.call.content_candidate` 事件供 ToolDoctor 排查。

---

## 5. ToolCallLifecycle 完整路径

每一个 tool 调用从模型流出到执行完成，必须**逐阶段**通过下面的检查点。任意
检查点失败 → 转 ModelReadableToolError，不直接 crash。

```text
[StreamEvent::ToolCallAssembled]
        ↓
[1] AliasRegistry.resolve(name)            → canonical_id 或 UnknownTool
        ↓
[2] SchemaValidator.validate(args)         → ok 或 issues[]
        ↓ (if issues)
[3] IssueGuidedRepairer.repair(args, issues) → repaired_args
        ↓
[4] SchemaValidator.validate(repaired)     → ok 或 SchemaValidationFailed (final)
        ↓
[5] RelationalInvariantResolver.resolve(args) → adjusted_args + notes
        ↓
[6] ProviderCapabilityMatrix.check_strict_required(canonical, args)
        ↓
[7] PermissionPolicy.evaluate(call, mode)  → Allow / Ask / Deny
        ↓ (if Allow)
[8] ToolDispatcher.dispatch_or_queue(call) → read-only 并发 / 写串行
        ↓
[9] ToolExecutor.execute(call)             → ToolExecutionResult
        ↓
[10] ResultFormatter.format(result, tool)  → 标准化 preview + detail_json
        ↓
[11] AppendToConversationHistory(tool_result)
```

**没有任何捷径**。包括 base_hash 注入：第 5 步关系型不变量解析时由 runtime
计算 `file.write/edit` 的 base_hash 注入到 args，然后第 9 步 dispatch 才执行。

---

## 6. ReasoningReplayManager

DeepSeek thinking 模式下，模型 turn t 的 reasoning_content 必须在 turn t+1
请求里回传，否则 API 400。但 reasoning 可达 10K+ tokens，要主动管理。

```rust
pub struct ReasoningReplayManager {
    /// 按 (session_id, turn_index) 存最近一次 reasoning_content
    last_reasoning: HashMap<String, ReasoningEntry>,
    sanitizer: ReasoningSanitizer,
}

pub struct ReasoningEntry {
    pub turn_index: u32,
    pub assistant_message_id: String,
    pub raw_reasoning: String,        // 原始（用于回放）
    pub sanitized_preview: String,    // 给 GUI 用的脱敏版
    pub tokens: u64,
    pub captured_at: Instant,
}

impl ReasoningReplayManager {
    /// turn 中收到 reasoning_content delta 时调用
    pub fn capture(&mut self, session_id: &str, turn_index: u32, delta: &str) {
        let entry = self.last_reasoning.entry(session_id.into()).or_insert_with(|| {
            ReasoningEntry { turn_index, /* ... */ }
        });
        entry.raw_reasoning.push_str(delta);
    }

    /// 准备 turn t+1 的请求时调用
    pub fn inject_if_required(&self, session_id: &str, request: &mut DeepSeekRequest) {
        // 仅当 (a) 上一轮有 tool_calls (b) 模型是 thinking-capable (c) 是
        // DeepSeek family 时，需要在最后一条 assistant message 上加
        // reasoning_content 字段
        if let Some(entry) = self.last_reasoning.get(session_id) {
            if let Some(last_assistant) = request.last_assistant_with_tool_calls() {
                last_assistant.reasoning_content = Some(entry.raw_reasoning.clone());
            }
        }
    }

    /// Compactor 调用：把旧 reasoning 转成 token 摘要后丢弃 raw
    pub fn compact_old_reasoning(&mut self, session_id: &str, older_than_turn: u32) {
        // 仅保留最近 1 个 reasoning（再老的没用，因为 thinking 链只对相邻 turn 必要）
    }
}
```

### 6.1 Sanitizer 不丢内容

`raw_reasoning` 是回放用的，**不能脱敏**——脱敏会改变内容长度，导致
provider 端 reasoning 校验失败。脱敏只发生在 `sanitized_preview`（给 GUI 看）。
这是和 doc 37 §15 描述的关键差异。

### 6.2 何时丢弃 raw

- session 进入下一个 turn 且**没有** tool_use（纯文本回答）→ 丢；
- compaction 触发 → 仅保留最近 1 个；
- session 关闭 → 丢。

---

## 7. CachePrefixPolicy：三段式

DeepSeek context cache 命中要求**前缀字节级一致**。当前实现把 AGENTS.md、git
status、todo、conversation history 全混一起，每轮都变化，缓存命中率近 0。

### 7.1 三段式 prompt 结构

```text
┌──────────────────────────────────────────────┐
│ Zone A: IMMUTABLE                            │
│  - base system prompt for family             │
│  - tool catalog (sorted by canonical id)     │
│  - tool calling rules                        │
│  → Cache hot zone                            │
├──────────────────────────────────────────────┤
│ Zone B: PER-SESSION                          │
│  - PermissionMode + mode guidance            │
│  - Project AGENTS.md / RESEARCHCODE.md       │
│  - Workspace metadata (root path)            │
│  → Cache warm zone (session-stable)          │
├──────────────────────────────────────────────┤
│ Zone C: PER-TURN                             │
│  - Recent git status snapshot                │
│  - Active plan / todo list                   │
│  - Conversation history                      │
│  → Always fresh, not cached                  │
└──────────────────────────────────────────────┘
```

### 7.2 排序规则

Zone A 的 tool catalog 必须按 `canonical_tool_id` lexical 排序——任何顺序变动
都会击穿缓存。

Zone B 的项目元信息按字段名排序，AGENTS.md 内容原样保留（用户编辑会击穿，但
合理）。

### 7.3 实现

```rust
pub struct CachePrefixPolicy {
    family: NativeModelFamily,
}

impl CachePrefixPolicy {
    pub fn build_zones(&self, ctx: &PromptContext) -> ThreeZonePrompt {
        ThreeZonePrompt {
            zone_a: self.immutable_zone(ctx),
            zone_b: self.session_zone(ctx),
            zone_c: self.turn_zone(ctx),
        }
    }

    fn immutable_zone(&self, ctx: &PromptContext) -> String {
        let mut tools: Vec<_> = ctx.manifest.tools.iter().collect();
        tools.sort_by_key(|t| &t.canonical_tool_id);
        format!(
            "{base}\n\n# Tool Calling Rules\n{rules}\n\n# Available Tools\n{catalog}",
            base = base_system_prompt(&self.family),
            rules = tool_calling_rules(&self.family),
            catalog = serde_json::to_string(&tools).unwrap(),
        )
    }
    // ...
}
```

### 7.4 Telemetry

每次 model_call 记录：
- `prompt_zone_a_hash`
- `prompt_zone_b_hash`
- `prompt_tokens_total`
- `prompt_tokens_cached_hint`（如果 provider 返回）

ToolDoctor 命令 `cache-status` 看命中率。

---

## 8. RepairCatalog：issue-guided

每条规则三件事：**触发条件**、**修复动作**、**禁用字段清单**。

```rust
pub struct RepairRule {
    pub name: &'static str,
    pub trigger: fn(&ValidationIssue, &ParsedToolArguments) -> bool,
    pub apply: fn(&mut ParsedToolArguments, &ValidationIssue),
    pub never_apply_to: &'static [&'static str], // 工具+字段路径 e.g. "file.write.content"
}

pub fn p0_repair_catalog() -> Vec<RepairRule> {
    vec![
        RepairRule {
            name: "strip_optional_null",
            trigger: |issue, _| issue.expected != "null" && issue.received == "null"
                && issue.field_optional,
            apply: |args, issue| args.unset(&issue.path),
            never_apply_to: &[],
        },
        RepairRule {
            name: "parse_stringified_array",
            trigger: |issue, args| issue.expected_starts_with("array")
                && issue.received == "string"
                && args.get_str(&issue.path).map(|s| s.starts_with('[')).unwrap_or(false),
            apply: |args, issue| {
                let s = args.get_str(&issue.path).unwrap();
                if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(s) {
                    args.set(&issue.path, serde_json::Value::Array(arr));
                }
            },
            never_apply_to: &["file.write.content", "shell.command.command"],
        },
        RepairRule {
            name: "wrap_bare_string_to_array",
            trigger: |issue, _| issue.expected == "array<string>" && issue.received == "string",
            apply: |args, issue| {
                let s = args.get_str(&issue.path).unwrap().to_string();
                args.set(&issue.path, serde_json::json!([s]));
            },
            never_apply_to: &["file.write.content", "shell.command.command"],
        },
        RepairRule {
            name: "unwrap_markdown_link_path",
            trigger: |issue, args| {
                issue.field_kind == FieldKind::Path
                    && args.get_str(&issue.path).map(looks_like_md_link).unwrap_or(false)
            },
            apply: |args, issue| {
                let s = args.get_str(&issue.path).unwrap();
                let unwrapped = unwrap_md_link(s);
                args.set(&issue.path, serde_json::Value::String(unwrapped));
            },
            never_apply_to: &["file.write.content"], // markdown 内容不能 unwrap
        },
        RepairRule {
            name: "empty_object_to_array",
            trigger: |issue, args| issue.expected_starts_with("array")
                && args.is_empty_object(&issue.path),
            apply: |args, issue| args.set(&issue.path, serde_json::json!([])),
            never_apply_to: &[],
        },
    ]
}
```

### 8.1 算法

```rust
pub fn validate_then_repair(
    tool_id: &str,
    args: ParsedToolArguments,
) -> Result<ParsedToolArguments, ModelReadableToolError> {
    let schema = lookup_schema(tool_id);
    // 1. 直接 validate
    if let Ok(()) = schema.validate(&args) {
        return Ok(args);
    }
    // 2. 收集 issues
    let issues = schema.validate_collect_issues(&args);
    // 3. 按 issue 应用 repair
    let mut repaired = args.clone();
    let mut applied = Vec::new();
    for issue in &issues {
        for rule in p0_repair_catalog() {
            if rule.never_apply_to.iter().any(|p| issue.matches_path(tool_id, p)) {
                continue;
            }
            if (rule.trigger)(issue, &repaired) {
                (rule.apply)(&mut repaired, issue);
                applied.push(rule.name);
                break;
            }
        }
    }
    // 4. validate 再
    if let Ok(()) = schema.validate(&repaired) {
        return Ok(repaired);
    }
    // 5. 失败 → ModelReadableToolError
    Err(ModelReadableToolError::schema_failed(tool_id, schema.validate_collect_issues(&repaired)))
}
```

**永不**对 `file.write.content` / `shell.command.command` 应用任何修复。

---

## 9. AliasRegistry：DeepSeek 习惯名字目录

```rust
pub struct AliasRegistry {
    forward: HashMap<String, String>,  // requested → canonical
}

impl AliasRegistry {
    pub fn deepseek_default() -> Self {
        let mut r = HashMap::new();
        // file.read 系列
        for alias in [
            "read", "Read", "read_file", "readFile", "fileRead",
            "read_source_code", "ReadFile",
        ] {
            r.insert(alias.into(), "file.read".into());
        }
        // file.list_directory 系列
        for alias in [
            "ls", "list", "list_dir", "listDir", "ListDirectory",
            "ListDir", "list_files", "list_directory",
        ] {
            r.insert(alias.into(), "file.list_directory".into());
        }
        // search.ripgrep 系列
        for alias in [
            "grep", "Grep", "search", "Search", "rg", "ripgrep",
            "search_files", "SearchFiles",
        ] {
            r.insert(alias.into(), "search.ripgrep".into());
        }
        // shell.command 系列
        for alias in [
            "bash", "Bash", "shell", "Shell", "exec", "Exec",
            "run", "RunCommand", "execute_command",
        ] {
            r.insert(alias.into(), "shell.command".into());
        }
        // file.write 系列
        for alias in [
            "write", "Write", "write_file", "WriteFile",
            "save", "save_file",
        ] {
            r.insert(alias.into(), "file.write".into());
        }
        // file.edit 系列
        for alias in [
            "edit", "Edit", "edit_file", "EditFile", "modify",
            "patch", "PatchFile",
        ] {
            r.insert(alias.into(), "file.edit".into());
        }
        // plan 系列
        for alias in [
            "plan", "Plan", "plan_enter", "enter_plan", "EnterPlanMode",
        ] {
            r.insert(alias.into(), "plan.enter".into());
        }
        // todo
        for alias in ["todo", "Todo", "todo_write", "TodoWrite", "write_todo"] {
            r.insert(alias.into(), "todo.write".into());
        }
        // git
        for alias in ["git_status", "GitStatus", "status"] {
            r.insert(alias.into(), "git.status".into());
        }
        Self { forward: r }
    }

    pub fn resolve(&self, requested: &str) -> AliasResolution {
        let trimmed = requested.trim();
        // 1. 直接 hit
        if let Some(canonical) = self.forward.get(trimmed) {
            return AliasResolution::Hit { canonical: canonical.clone(), original: trimmed.into() };
        }
        // 2. 大小写不敏感
        let lower = trimmed.to_ascii_lowercase();
        if let Some(canonical) = self.forward.get(&lower) {
            return AliasResolution::Hit { canonical: canonical.clone(), original: trimmed.into() };
        }
        // 3. snake/dot 互转
        let dotted = lower.replace('_', ".");
        if find_tool_spec(&dotted).is_some() {
            return AliasResolution::Hit { canonical: dotted, original: trimmed.into() };
        }
        // 4. 模糊建议（编辑距离）
        let suggestion = self.fuzzy_suggest(trimmed);
        AliasResolution::Miss { requested: trimmed.into(), suggestion }
    }
}
```

---

## 10. ProviderCapabilityMatrix：启动时探测

```rust
pub struct CapabilityProbe {
    pub endpoint: NativeProviderEndpoint,
}

impl CapabilityProbe {
    pub async fn probe(&self) -> ToolCallingCapabilities {
        let mut caps = ToolCallingCapabilities::default();

        // 1. 简单 ping
        if !self.ping().await { return caps; /* 全 false */ }

        // 2. 工具调用支持？
        caps.supports_tools = self.test_simple_tool_call().await;

        // 3. streaming tool 支持？
        caps.supports_streaming_tools = self.test_streaming_tool_call().await;

        // 4. tool_choice="required"？
        caps.supports_tool_choice_required = self.test_tool_choice_required().await;

        // 5. strict mode？
        caps.supports_strict_json_schema = self.test_strict_mode().await;

        // 6. reasoning_content 支持？（仅 DeepSeek thinking）
        caps.supports_reasoning_replay = self.test_reasoning_replay().await;

        // 7. 解析器侦测（vLLM）
        caps.tool_parser = self.detect_tool_parser_flag().await;
        caps.reasoning_parser = self.detect_reasoning_parser_flag().await;

        caps
    }
}
```

探测结果存到 `~/.researchcode/capabilities/{endpoint_hash}.json`，ttl 24h。
启动时若有效缓存就跳过探测。

---

## 11. RoleSplit + TemperatureSchedule

```rust
pub struct RoleSplit {
    pub role_models: RoleModelMap,
    pub temperatures: HashMap<RoleStage, f64>,
}

pub enum RoleStage {
    Routing,           // turn 分类，temp=0.0
    PlanDrafting,      // 起草计划，temp=0.5
    Executing,         // 主执行，temp=0.2（工具调用稳）
    Reviewing,         // 审查，temp=0.0
    Compacting,        // 压缩历史，temp=0.0
    NarrativeAnswer,   // 给用户最终答复，temp=0.7
}

impl RoleSplit {
    pub fn deepseek_default() -> Self {
        let role_models = RoleModelMap {
            executor: ModelEndpoint::deepseek_chat(),
            compactor: ModelEndpoint::deepseek_chat_flash(),
            reviewer: ModelEndpoint::deepseek_chat(),
            titler: ModelEndpoint::deepseek_chat_flash(),
            summarizer: ModelEndpoint::deepseek_chat_flash(),
        };
        let mut temperatures = HashMap::new();
        temperatures.insert(RoleStage::Routing, 0.0);
        temperatures.insert(RoleStage::PlanDrafting, 0.5);
        temperatures.insert(RoleStage::Executing, 0.2);
        temperatures.insert(RoleStage::Reviewing, 0.0);
        temperatures.insert(RoleStage::Compacting, 0.0);
        temperatures.insert(RoleStage::NarrativeAnswer, 0.7);
        Self { role_models, temperatures }
    }
}
```

每个 turn 进入 stage 时切 model + temp。

---

## 12. Compactor：DeepSeek-aware

继承 doc 38 §17，新增 DeepSeek 特化：

- threshold = `min(192K, context_window * 0.75)` —— 192K 是 DeepSeek 实测拐点；
- 压缩时**单独保留**最近 1 个 reasoning（B1 必须）；
- 压缩调用走 `Compactor` role（默认 Flash），不消耗 Executor 预算；
- 压缩生成的 summary 在系统消息加 `[compacted-context]` 标记，让模型知道这是
  摘要不是原话。

---

## 13. ModelReadableToolError 标准消息目录

每个 error_code 都给标准化模板（DeepSeek/Qwen 直接读后回退）：

```rust
impl ModelReadableToolError {
    pub fn unknown_tool(requested: &str, suggestion: Option<&str>) -> Self {
        Self {
            error_code: ToolErrorCode::UnknownTool,
            tool_name: requested.into(),
            short_message: format!(
                "Tool '{requested}' is not in the catalog. Use only canonical tool names."
            ),
            field_errors: vec![],
            retry_hint: match suggestion {
                Some(s) => format!("Did you mean '{s}'? Retry using that exact name."),
                None => "Choose from the available tools listed in the system prompt.".into(),
            },
            retry_example: suggestion.map(|s| serde_json::json!({"tool": s})),
            retryable: true,
            counts_against_budget: false, // 命名错误免费重试
        }
    }

    pub fn plan_mode_required(tool: &str) -> Self {
        Self {
            error_code: ToolErrorCode::PlanModeRequired,
            tool_name: tool.into(),
            short_message: format!(
                "Tool '{tool}' modifies state and cannot run in plan mode. Present your plan first."
            ),
            field_errors: vec![],
            retry_hint: "Call plan.enter with a plain-text description of what you intend to do. \
                The user will approve or reject; on approval, you can call this tool.".into(),
            retry_example: Some(serde_json::json!({
                "tool": "plan.enter",
                "args": {"plan": "I will modify <file> to <change> because <reason>."}
            })),
            retryable: true,
            counts_against_budget: false,
        }
    }

    pub fn schema_validation(tool: &str, issues: &[ValidationIssue]) -> Self {
        let field_errors = issues.iter().map(|i| FieldError {
            path: i.path.clone(),
            expected: i.expected.clone(),
            received: i.received.clone(),
            fix_hint: hint_for_issue(tool, i),
        }).collect();
        let retry_example = canonical_example_for(tool);
        Self {
            error_code: ToolErrorCode::SchemaValidationFailed,
            tool_name: tool.into(),
            short_message: format!("Arguments for '{tool}' do not match required schema."),
            field_errors,
            retry_hint: "Fix the listed fields and resend. Do not change other fields.".into(),
            retry_example,
            retryable: true,
            counts_against_budget: false,
        }
    }

    // ... (PermissionDenied, ToolExecutionFailed, etc.)
}
```

`canonical_example_for` 给每个 tool 一份"最小有效 args"模板，DeepSeek 可以
直接照抄。

---

## 14. ToolResult Format（DeepSeek-friendly）

延续 doc 38 §14，加 DeepSeek 特化：

### 14.1 `file.read` preview 格式

```text
file.read · src/foo.rs · lines 1-50/200

   1  pub fn main() {
   2      println!("hello");
   3  }
   4
   ...
  50  }

[truncated; 150 more lines, request offset=50,limit=N to continue]
```

DeepSeek 在引用代码时倾向用行号；带行号能让它精确定位。

### 14.2 `file.edit` 成功格式

```text
file.edit · src/foo.rs · 1 replacement

base_hash: 7a3b... → new_hash: 9c4d...

@@ -1,3 +1,3 @@
- pub fn main() {
+ pub fn run() {
   println!("hello");
 }
```

### 14.3 `shell.command` 格式

```text
shell.command · `cargo test --lib` · exit 0 · 3.4s

stdout (last 80 lines):
running 314 tests
test ... ok
...

stderr: (empty)
```

stderr 即使为空也 emit，避免模型猜。

### 14.4 错误格式（统一）

```text
file.write · ERROR · SchemaValidationFailed

Field "path" is required but missing.

Retry with:
{
  "path": "src/example.rs",
  "content": "..."
}
```

---

## 15. Implementation Phases（具体）

### Phase 1: Foundation refactor (1 周)

**目标**：把单体 native_agent_loop.rs 拆成可独立测试的组件。不改外部行为。

```text
crates/runtime/src/agent_kernel/
├── mod.rs                  pub use {AgentKernel, TurnRequest, TurnResult}
├── kernel.rs               struct AgentKernel + run_turn()
├── turn_state.rs           struct TurnState（替代 12 个散落变量）
├── budget_policy.rs        struct BudgetPolicy
├── permission_policy.rs    struct PermissionPolicy + 5 modes
└── compactor.rs            stub（Phase 4 实现）

crates/runtime/src/native_profile/
├── mod.rs                  trait NativeProfile + factory
├── deepseek/
│   ├── mod.rs              struct DeepSeekProfile
│   ├── stream.rs           StreamProcessor + DsmlChunkFilter
│   ├── reasoning.rs        ReasoningReplayManager
│   ├── cache_prefix.rs     CachePrefixPolicy
│   └── role_split.rs       RoleSplit (DeepSeek defaults)
└── qwen/
    └── mod.rs              struct QwenProfile（先空壳，Phase 5 实现）

crates/runtime/src/tcml/
├── mod.rs                  pub use TCML pipeline
├── alias_registry.rs       AliasRegistry
├── repair_catalog.rs       RepairRule + p0_catalog
├── relational_resolver.rs  RelationalInvariantResolver (含 base_hash 注入)
├── content_extractor.rs    ContentToolCallExtractor
└── error_factory.rs        ModelReadableToolError builders
```

**Phase 1 不变更行为**，只搬位置。每搬一次跑全部测试。

### Phase 2: PermissionPolicy + manifest 全开 (3-4 天)

- 删 manifest 的 turn-state 切除（现有 `tool_exposure` 内部全部走 FastAutoWrite）；
- shell.command 始终在 manifest（已开始）；
- PermissionPolicy 5 modes 实现，`Default` 模式下写/shell 走 ask；
- ask 在 GUI 走 plan_approval_pending 类似的 await；
- 删 `non_progress_recovery_count` finalizer（永久禁工具的路径）；
- 删 `loop_guard_recovery_count` finalizer（同上）；
- 跑通 5 个被打破的测试，更新断言。

**测试新增**：
- "naming errors don't trigger tool ban"
- "plan mode write is denied with retry guidance"
- "Default mode write triggers ask"

### Phase 3: ConversationHistory 全保真 (2-3 天)

- `session.to_conversation_messages()` 把 EventLog 变成 OpenAI message format；
- assistant message 包含 `tool_calls`（如有）和 `reasoning_content`（如有）；
- tool message 用 `tool_call_id` 关联；
- runtime_facade build_context_bundle 改为只生成 system 段；
- 测试新增："turn 2 sees turn 1 tool_result"。

### Phase 4: Compactor + DeepSeek-aware threshold (1 周)

- 实现 `compactor.rs`：摘要 + 保留最近 N pair + 单独保留最新 reasoning；
- 接 ReasoningReplayManager.compact_old_reasoning；
- 触发条件：tokens_in > 192K 或显式 `/compact`；
- 删除 finalizer 路径残余；
- 测试新增："context > 192K triggers compaction, reasoning preserved"。

### Phase 5: NativeProfile 完整化 (1 周)

- StreamProcessor 完整迁移（DsmlChunkFilter 已实现）；
- ReasoningReplayManager 接入主循环；
- CachePrefixPolicy 三段式落地，metrics 上报；
- ProviderCapabilityMatrix 启动探测；
- RoleSplit + TemperatureSchedule 接入；
- QwenProfile 落地（chat-template 探测）。

### Phase 6: ToolResult Format + Error Catalog (3-5 天)

- 每个 ToolExecutor 输出走 ResultFormatter；
- ModelReadableToolError 标准模板覆盖所有 ToolErrorCode；
- file.read 加行号、file.edit 加 diff、shell.command 加 stderr；
- 测试 fixture 库覆盖每个工具的 success/error 输出。

### Phase 7: Subagent (task.dispatch) (1-2 周)

- `task.dispatch` 工具实现；
- 子 session 隔离 context；
- 子 session 用 RoleSplit::Compactor model（默认 Flash）；
- 父收到 summary string + artifact_refs。

### Phase 8: Telemetry + ToolDoctor (3-5 天)

- 每条 tool call lifecycle phase emit 结构化 event；
- ToolDoctor 命令：`cache-status`, `alias-stats`, `repair-rate`, `unknown-tool-history`；
- GUI 加诊断面板。

---

## 16. File-Level Changes 清单

| 文件 | 操作 |
|---|---|
| `crates/runtime/src/native_agent_loop.rs` | 拆解到 `agent_kernel/` 各文件，最终留 < 400 行的 `kernel.rs` |
| `crates/runtime/src/runtime_facade.rs` | 删 prompt-keyword exposure（已删）；删 build_context_bundle 的 history 部分 |
| `crates/runtime/src/tool_contract.rs` | 拆 alias/repair/relational 到 `tcml/` 子文件；保留 `mediate_tool_call` 入口 |
| `crates/runtime/src/tool_call_parser.rs` | 移到 `tcml/` 下，归并到 alias_registry |
| `crates/runtime/src/deepseek_reasoning.rs` | 移到 `native_profile/deepseek/reasoning.rs` |
| `crates/runtime/src/thinking_chain.rs` | 移到 `native_profile/deepseek/stream.rs` |
| `crates/runtime/src/native_turn_controller.rs` | 合并进 `agent_kernel/turn_state.rs` |
| `crates/runtime/src/context_budget.rs` | 移到 `native_profile/deepseek/budget.rs`（Qwen 用 qwen 版） |
| `crates/runtime/src/prompt_assembler.rs` | 拆：cache_prefix 部分进 NativeProfile，conversation history 进 `agent_kernel/conversation_history.rs` |
| `crates/runtime/src/tool_dispatcher.rs` | 保留并加 PermissionGate 接入 |
| `crates/runtime/src/tool_execution.rs` | 加 ResultFormatter |
| `crates/kernel/src/tool.rs` | 加完整 alias 列表（大小写、snake/dot 变体） |
| `crates/runtime/src/observation_cache.rs` | 保留（重复调用不禁工具，给提示） |

---

## 17. Risk Register

| 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|
| 拆 native_agent_loop.rs 引入回归 | 高 | 高 | 严格 Phase 1 不改行为；每搬一次跑全测试 |
| ReasoningReplayManager 误丢 reasoning_content | 中 | 高 | unit test 覆盖：thinking + tool_use + 下一轮场景 |
| CachePrefixPolicy 三段式破坏现有缓存 | 低 | 低 | 三段式纯改善，旧缓存自然过期 |
| RoleSplit 把 Compactor 用 Flash 导致摘要质量下降 | 中 | 中 | 可配置切回 Pro；A/B 评估 |
| AliasRegistry 误把模糊词强制映射（如 "list"→file.list_directory 当用户其实想查 git log） | 低 | 中 | resolve 时返回 confidence；Low confidence 进 ModelReadableToolError 让模型确认 |
| ContentToolCallExtractor 误判把无害文本当 tool call | 中 | 低 | 仅在 tool_calls=空 + finish_reason=stop 触发；高门槛 |
| Compactor 摘要质量低导致后续轮模型迷失 | 中 | 高 | Compactor prompt 严控；保留近 4 完整 turn；测试场景验证 |
| DeepSeek 端点 strict 探测误判 | 低 | 低 | 探测缓存 ttl 24h；显式开关覆盖 |
| Phase 顺序变化导致中间态不可用 | 中 | 中 | Phase 1-3 后先完整跑一遍 user 场景再继续；不一次性堆 8 phase |

---

## 18. Eval Gates（DeepSeek-specific）

doc 38 §10 列了通用项。这里加 DeepSeek 专用：

- **R1. reasoning_content 在 tool_use 链中始终回放** —— 模拟 thinking + tool_use 三轮，第 4 轮检查请求体含正确 reasoning_content
- **R2. DSML 跨 chunk 不漏到 visible**  —— DSML 标签在 chunk N 起、chunk N+5 终，中间 chunk 内容必须不出现在 visible_buffer
- **R3. tool_calls.delta 跨 chunk 累加正确** —— 5 个 chunk 拼出一个 finalize 后 args 是闭合 JSON
- **R4. ContentToolCallExtractor 不误执行 state-changing** —— 喂一段含 `<tool_call>file.write {...}</tool_call>` 的文本，确保候选生成但不直接 dispatch
- **R5. AliasRegistry 覆盖 DeepSeek 高频错名** —— 列表 50+ 项，每项 resolve 返回正确 canonical
- **R6. RepairCatalog 不修 file.write.content** —— content 字段含 stringified array、null、markdown link 全部不动
- **R7. CachePrefixPolicy 排序稳定** —— 同 manifest 不同顺序生成的 zone_a hash 相同
- **R8. RoleSplit Compactor 用 Flash** —— compaction 触发时实际请求 endpoint 是 Flash
- **R9. 192K 触发 compaction** —— 模拟 prompt_tokens=193K，下一轮请求前必有 `compaction.completed`
- **R10. base_hash 由 runtime 注入** —— 模型 args 不含 base_hash，dispatch 后实际请求含 hash

---

## 19. Telemetry 事件（DeepSeek 关键指标）

```text
deepseek.cache.zone_a_hit_rate
deepseek.cache.zone_b_hit_rate
deepseek.reasoning.tokens_per_turn
deepseek.reasoning.replay_count
deepseek.reasoning.replay_size_kb
deepseek.dsml.leak_chunks_count
deepseek.dsml.leak_recovered
deepseek.tool_call.partial_chunks_avg
deepseek.tool_call.assembly_latency_ms
deepseek.alias.resolution_count_by_alias
deepseek.repair.rule_applied_count_by_rule
deepseek.repair.success_rate
deepseek.compaction.triggers_count
deepseek.compaction.tokens_freed
deepseek.role_split.executor_calls
deepseek.role_split.compactor_calls
deepseek.role_split.flash_savings_estimate_usd
```

每天聚合一次写入 `~/.researchcode/telemetry/{date}.jsonl`，ToolDoctor 命令读取。

---

## 20. Summary（一段话）

> doc 39 把 doc 37 的三层骨架和 doc 38 的 Claude-Code-grade 成熟度结合起来，
> 但**不**当成"通用 runtime + DeepSeek 适配器"——而是从 StreamProcessor 起，
> 就按 DeepSeek 的实际行为模式（reasoning_content 流、tool_calls.delta 跨 chunk、
> DSML 漏出、cache 前缀敏感、Pro/Flash 梯度、温度敏感、长 context 拐点、工具
> 名错误小目录、JSON 形状错误小目录）从底层往上塑形脚手架。Claude Code
> 借鉴的是**架构纪律**（分层、不变式、不可绕过的 mediation、event 单一真相）；
> 而 DeepSeek-native 的部分则把 ReasoningReplayManager、CachePrefixPolicy、
> AliasRegistry、RepairCatalog、RoleSplit 这些组件当**一等公民**写进 kernel，
> 不是事后补丁。最终的 ResearchCode agent kernel 同时具备 Claude Code 的纪律
> 和 DeepSeek 的深度优化——用户可以在里面像在 Claude Code 里一样顺手地工作，
> 但每一个底层决策（流式解析、提示构造、错误格式、模型选择）都为 DeepSeek
> 调优过。
