# 38 Agent Kernel v2: Claude Code Alignment with DeepSeek/Qwen Concessions

本文是 doc 37 的**修订与超集**。doc 37 给出了正确的三层骨架
（AgentKernel + ToolContractMediationLayer + NativeModelProfile），但有两条
invariant 在落地时把 ResearchCode 推到了"过度受限"那一边，反而让 DeepSeek/Qwen
比应有的样子更难用。本文做三件事：

1. 把 doc 37 中**两条错误的 invariant 替换掉**，对齐 Claude Code 的实际架构；
2. 把最近一周从用户反馈和实测中归纳出的**八个具体故障**翻译成结构性教训；
3. 给一套**到位即用**的迁移路径——告诉每个文件该改什么、该删什么、新增什么。

---

## 0. Context: 为什么需要 v2

doc 37 写于 ResearchCode 还没有真实多轮 DeepSeek/Qwen 用户压力时。落地后的实测
暴露了几个矛盾：

> "agent runtime 应该 ClaudeCode-grade，DeepSeek 应该比闭源更顺手"
> 这两件事的张力，集中在**工具暴露策略**上。

doc 37 的两条 invariant：

- 9. WorkflowFSM owns long tool loops.
- 10. Active tool sets are small and state-specific.

按字面落地后变成了：

- `tool_exposure ∈ {ReadOnly, FastAutoWrite}` + 关键词启发式决定 mode；
- `workflow_state ∈ {planning, reading, editing, testing, ...}` 进一步砍 manifest；
- `non_progress_recovery_count >= 2 → disable_tools_finalizer`；
- `loop_guard_recovery_count > 2 → finalizer`。

结果是：用户说"试一试 write" → ReadOnly 模式 → 模型告诉用户"我没有 write 工具" →
用户困惑。模型打错一次工具名（"plan" 而非 "plan.enter"）→ 两轮内被禁用所有工具
→ agent 死掉。

而 Claude Code 的实际做法**完全不是这样**。

---

## 1. Claude Code 的真实架构（核对过的事实）

经过对 Claude Code 文档和行为的核对，确认以下事实（不是推测）：

### 1.1 工具 manifest 是静态的

Claude Code **不分析 prompt 关键词**来决定 manifest 内容。manifest 由 session
配置 + 已注册工具 + MCP servers 决定，每一轮 turn 都暴露同一份完整工具表给
模型。

### 1.2 权限在执行时检查，不在 manifest 时

Claude Code 的权限模式（mode）影响**工具执行能否通过**，不影响**模型是否看见
工具**：

| Mode | 工具是否可见 | 工具是否能执行 |
|---|---|---|
| `default` | 全部 | 按 `permissions.allow/ask/deny` 规则 |
| `plan` | 全部 | 写工具被拦截直到 plan 通过审批 |
| `acceptEdits` | 全部 | 编辑类工具自动放行 |
| `dontAsk` | 全部 | 仅 `permissions.allow` 中的工具放行 |
| `bypassPermissions` | 全部 | 全部立即放行 |

### 1.3 权限规则的优先级

```
deny  >  ask  >  allow
```

`PreToolUse` hook 通过 exit code 2 还能在 allow 之上强制 block。

### 1.4 工具错误不致命

Tool error 是**模型可读的反馈**，不是 runtime crash。模型看到错误，自己决定
重试或换策略。Claude Code **没有**：
- "non_progress_recovery_count" 累计逻辑
- "disable_tools_finalizer" 终结路径
- "loop_guard recovery" 强制工具禁用

它只有：
- 软预算（max iterations、max tokens）
- 自动 compaction（context 满了就压缩历史）
- 用户可中断

### 1.5 Plan mode 是权限语义，不是 manifest 过滤

Plan mode 下模型仍然看到全部工具。它知道自己处于 plan mode（系统消息提示），
所以会先用 `ExitPlanMode` 工具呈现计划。如果它直接调用写工具，runtime 在
**执行时**拦截并返回"需要先 plan"错误。模型理解后会回到 plan 流程。

### 1.6 Subagent 提供上下文隔离

复杂探索丢给 Task subagent，它有自己的 context 和工具集。父对话只看到
subagent 的最终摘要。这取代了"多轮迭代污染主上下文"问题。

---

## 2. 修订 doc 37 的 invariant

doc 37 第 25 节的 15 条 invariant，**12 条保留**，**2 条替换**，**1 条加强**：

### 保留（不改）

1. Model proposes; runtime executes.
2. No raw model tool call bypasses TCML.
3. ToolManifest is the only model-visible tool source.
4. Valid tool inputs are never mutated.
5. Invalid inputs are repaired only at schema issue paths.
6. Dangerous/state-changing inputs are never auto-repaired.
7. UnknownTool is recoverable.
8. Streaming tool calls execute only after complete assembly and validation.
11. Read-only tools may parallelize; state-changing tools serialize.
12. DeepSeek reasoning replay is preserved or blocked before provider call.
13. Qwen native mode requires parser/template capability evidence.
14. Compatible providers cannot override native profile policy.

### 替换（v2 关键变化）

- ~~9. WorkflowFSM owns long tool loops.~~
  **9'. Loop is budget-bounded, model-driven.** runtime 提供 max_iterations、
  max_tokens、max_tool_calls 三类软预算，并在每一轮检查 user cancellation。
  runtime 不替模型"决定下一步该用什么工具"。

- ~~10. Active tool sets are small and state-specific.~~
  **10'. Tool manifest is full and stable per session.** 模型每一轮看到同一份
  完整工具表。能否真正**执行**某工具，由 PermissionPolicy 在执行点决定，不在
  manifest 时切除可见性。

### 加强

- **15'. Tool errors are informative, never terminal.** UNKNOWN_TOOL、
  TOOL_NOT_IN_MANIFEST、SCHEMA_VALIDATION_FAILED、TOOL_EXECUTION_FAILED 都不
  能触发"永久禁用工具"。runtime 唯一能在 turn 内终止 tool calling 的途径是
  达到显式预算上限或 user cancellation。

### 新增

- **16. Permission gating happens at execution time, not manifest time.** 所有
  写、shell、网络类工具始终在 manifest，PermissionPolicy 决定执行通过/拦截/
  转 ask。
- **17. Naming errors are free retries.** UNKNOWN_TOOL 和 TOOL_NOT_IN_MANIFEST
  不计入预算，因为它们是导航错误，模型自己看到错误就能修正。
- **18. Compaction replaces non-progress finalizer.** context 接近上限时执行
  压缩，不在 "几次没进展就 disable tools" 这种启发式上做决策。

---

## 3. 新架构图（v2）

```text
User / GUI / TUI
  ↓
RuntimeFacade
  ↓
AgentKernel
  ├─ TurnRouter            (轻量分类，仅决定 budget/scaffold，不砍 manifest)
  ├─ BudgetPolicy          (max_iterations / max_tokens / max_tool_calls)
  ├─ ContextManager        (history、压缩、重注入)
  ├─ EvidenceLedger        (read 缓存、status 摘要)
  ├─ PermissionPolicy      (default / plan / acceptEdits / bypass / hooks)
  ├─ Compactor             (context 接近上限时自动压缩)
  ├─ Finalizer             (仅在预算耗尽或显式 final 时触发)
  └─ EventLog
       ↓
ToolManifestBuilder        (静态生成，不被 prompt 关键词或 workflow_state 影响)
       ↓
NativeModelProfile         (DeepSeek/Qwen 专属，处理模型层 quirks)
  ├─ PromptScaffoldPolicy
  ├─ ToolSchemaPolicy
  ├─ ParserChainPolicy
  ├─ ReasoningReplayPolicy (DeepSeek thinking)
  ├─ DsmlChunkFilter       (DeepSeek text-mode tool 漏出)
  ├─ ContextBudgetPolicy
  └─ CachePrefixPolicy
       ↓
ProviderAdapter
       ↓
ToolContractMediationLayer (TCML，不变，按 doc 37 第 7.2 节)
       ↓
ToolDispatcher
       ↓
PermissionGate             (执行时拦截：plan-mode write / shell / 高风险)
       ↓
ToolExecutor
```

差异要点（标红）：
- TurnRouter 现在**只**输出 budget 和 scaffold 信息，**不**改 manifest；
- ToolManifestBuilder 输出**单一固定** manifest（基于 session 配置+权限+MCP），
  不再被 turn-level 状态过滤；
- 新增 **PermissionGate** 在 ToolDispatcher 之后、ToolExecutor 之前，做
  Claude-Code 风格的执行时检查；
- **Compactor** 取代 doc 37 中"几次失败就 finalizer"的逻辑。

---

## 4. PermissionPolicy 详细设计

这是 v2 最重要的新组件，负责吸收 doc 37 中
`tool_exposure / workflow_state` 误用为 manifest 过滤的责任。

### 4.1 输入

```rust
struct PermissionContext {
    tool_id: String,
    arguments: ParsedToolArguments,
    mode: PermissionMode,           // default / plan / acceptEdits / dontAsk / bypass
    session_id: String,
    workspace_root: PathBuf,
    user_intent_summary: Option<String>,  // 来自最近的 user message
}
```

### 4.2 输出

```rust
enum PermissionDecision {
    Allow,                          // 直接执行
    Ask(AskRequest),                // GUI/TUI 询问用户
    Deny(DenyReason),               // 转 ModelReadableToolError
    DeferToHook(HookSpec),          // 走 PreToolUse hook
}
```

### 4.3 默认规则

| 工具类别 | default | plan | acceptEdits | bypass |
|---|---|---|---|---|
| 只读（read/search/list/git.status/...） | Allow | Allow | Allow | Allow |
| 编辑（write/edit/multi_edit/patch.apply） | Ask | Deny(plan-required) | Allow | Allow |
| Shell（shell.command） | Ask | Deny(plan-required) | Ask | Allow |
| 计划（plan.enter/plan.write/plan.exit） | Allow | Allow | Allow | Allow |
| Todo/Question | Allow | Allow | Allow | Allow |

### 4.4 关键差异 vs 当前实现

```diff
- if !manifest_allowed_tools.contains(&tool_id) {
-     // 工具不在 manifest -> ModelReadableToolError
- }
+ // manifest 始终包含所有已注册工具
+ match permission_policy.evaluate(&ctx) {
+     PermissionDecision::Allow => execute(),
+     PermissionDecision::Deny(reason) => emit_model_readable_error(reason),
+     PermissionDecision::Ask(req) => suspend_and_ask_user(req),
+     PermissionDecision::DeferToHook(h) => run_hook(h),
+ }
```

### 4.5 与 plan.enter 的协同

Plan 流程的本质：
1. 模型在 plan mode 下尝试调用 write 工具 → PermissionGate 返回
   `Deny(plan-required)`，错误消息提示"先用 plan.enter 提交计划"；
2. 模型调用 plan.enter，runtime 进入"等待用户审批"状态；
3. 用户审批 → 模式从 plan 切到 acceptEdits → 后续 write 直接 Allow；
4. 用户拒绝 → 模式留在 plan，模型必须修订 plan。

这套流程在 doc 37 已存在，但当前实现把 manifest 也按 plan/edit 切了，本文修正
这一点：**manifest 不切，权限切**。

---

## 5. 修订 BudgetPolicy

### 5.1 三类软预算

```rust
struct TurnBudget {
    max_iterations: u32,        // model→tool→model 的循环次数
    max_tool_calls: u32,        // 总 tool 调用次数
    max_tokens: u64,            // model output token cap
    max_concurrent_tools: u32,  // 并行只读工具上限
}
```

### 5.2 哨兵值约定

**禁止把 0 当"无限"哨兵**。当前 `DEFAULT_MAX_TOOL_CALLS = 0` 引起了
`0 >= 0` 立即触发 freeze 的 bug。

```rust
// 错误（doc 37 之后落地的写法）：
if tool_call_count >= max_tool_calls { trigger_finalizer(); }
// 当 max_tool_calls=0 时，第一轮就 trigger，agent 死掉。

// 正确：用 Option，明确语义
if let Some(cap) = budget.max_tool_calls {
    if tool_call_count >= cap { /* finalize */ }
}
```

### 5.3 错误**不**计入预算的清单

下面这些错误是导航/打字错误，不消耗预算：

- UNKNOWN_TOOL（工具名不存在）
- TOOL_NOT_IN_MANIFEST（被权限拒绝—严格说在 v2 不会再发生）
- MALFORMED_TOOL_JSON（args 不是对象）
- SCHEMA_VALIDATION_FAILED（必填字段缺失）
- ALIAS_RESOLUTION_FAILED

下面这些**计入**预算（每次失败 +1 但不会触发 ban）：

- TOOL_EXECUTION_FAILED（实际执行失败：文件不存在、shell exit≠0）
- PERMISSION_DENIED（用户拒绝）
- SAFETY_POLICY_DENIED（runtime 安全规则拒绝）

---

## 6. NativeProfile 责任边界（DeepSeek 专属）

把当前散落在 `native_agent_loop.rs` 的 DeepSeek 修补搬到 NativeProfile。
AgentKernel 的代码里**不应再出现 `if family == DeepSeek`**。

### 6.1 DeepSeekProfile 责任

| 责任 | 现状 | 目标 |
|---|---|---|
| DSML XML 跨 chunk 过滤 | 在 `record_live_visible_stream_event` 里嵌入 `DsmlChunkFilter` | 移到 `DeepSeekProfile::sanitize_visible_chunk()` |
| `reasoning_content` replay | 散落在多处 | `DeepSeekProfile::prepare_request()` 注入 |
| 工具调用从 content 兜底提取 | 部分存在 | `DeepSeekProfile::extract_content_tool_calls()` |
| Streaming arg 累加 | 已有 | `DeepSeekProfile::accumulate_streaming_tool_call()` |
| Cache prefix 稳定 | 部分 | `DeepSeekProfile::stable_prompt_prefix()` |
| Token budget（Pro/Flash 区分） | `deepseek_runtime_max_tokens_for_prompt` | `DeepSeekProfile::max_output_tokens(role, route)` |

### 6.2 QwenProfile 责任

类似 DeepSeek，加上：

- chat-template 探测（serving stack capability check）
- qwen3_coder tool parser 验证
- 窄工具策略（Qwen 在大量工具下表现下降，但**这是模型偏好，不是 manifest 过滤**——
  做法是 NativeProfile 在 system prompt 里建议模型"优先选择 X、Y、Z"，
  manifest 仍然完整）

### 6.3 关键约束

NativeProfile **只能影响**：
- prompt scaffold 措辞
- 模型请求里 `reasoning_content`、`tool_choice` 等字段
- 流式响应的解析与清洗
- 缓存前缀
- token budget

NativeProfile **不能影响**：
- manifest 内容（必须固定）
- 权限决策（属于 PermissionPolicy）
- 预算上限（属于 BudgetPolicy）

---

## 7. Recovery 语义重写

doc 37 第 7.1 节说 "AgentKernel...decide continuation, compaction, finalization,
or escalation"。落地变成了：

```
两次 UNKNOWN_TOOL → loop_guard finalizer → 永久禁用 tool calling
```

这是错的。重写为：

### 7.1 三种 recovery 路径

1. **Naming/Schema/Validation 错误（命名错误）**
   - 不计预算
   - 直接把 ModelReadableToolError 喂回模型
   - 同一名错误重复 N 次也没关系，只要预算还在

2. **Execution 失败（执行错误）**
   - 计预算（+1 tool_call）
   - 把详细 stderr / 错误内容喂回模型
   - 模型自己决定换策略

3. **Context 接近上限**
   - 触发 Compactor（不是 finalizer）
   - 压缩历史摘要 + 保留最近 N 条 + 保留 plan/todo
   - 重注入压缩后的上下文，继续循环

### 7.2 移除的概念

- ❌ `non_progress_recovery_count` 及其 finalizer
- ❌ `loop_guard_recovery_count` 及其 finalizer
- ❌ `repeated_tool_batch` 强制 disable tools
- ❌ `disable_tools_and_request_final_answer` finalizer 路径

保留的概念：

- ✅ `seen_tool_batches` 用于**信息**——给模型一条提示"你刚刚执行过同一调用，
  缓存命中"，不阻止它再次调用
- ✅ observation cache 节省 token
- ✅ 显式 max_iterations、max_tool_calls、max_tokens 预算

### 7.3 turn-end 决策

每轮结束时 AgentKernel 检查：

```rust
match (model_emitted_text, model_emitted_tool_calls, budget_state) {
    (Some(_), [], _) => Done,                              // 模型给出最终答案
    (_, calls, BudgetOk) if !calls.is_empty() => Continue, // 还有预算就继续
    (_, _, BudgetExhausted) => RequestFinalAnswer,         // 预算尽，请模型结尾
    (None, [], _) => RequestVisibleAnswer,                 // 模型既没说话也没调工具
}
```

---

## 8. 具体故障 → 结构性教训

把最近实测的 8 个故障映射到结构性修复。这一节是 v2 落地清单的"为什么"。

| # | 故障 | 现象 | 结构修正 |
|---|---|---|---|
| 1 | 关键词决定 ReadOnly/FastAutoWrite | "试一试 write" 卡 ReadOnly | 删 `*_runtime_tool_exposure_for_prompt`，永远 FastAutoWrite |
| 2 | UNKNOWN_TOOL 计入 non_progress | 打错一次工具名就死 | `iteration_naming_error_results` 不计 non_progress（已落地） |
| 3 | DEFAULT_MAX_TOOL_CALLS=0 当无限 | `0 >= 0` 立即 freeze | `if cap > 0 &&` 守卫（已落地）；长期改 `Option<u64>` |
| 4 | base_hash 要求模型提供 | file.write 总是失败 | runtime 在 dispatch 前注入（已落地） |
| 5 | session_memory 不存 assistant | 多轮丢上下文 | `extract_visible_output_from_jsonl` + 写回 memory（已落地） |
| 6 | DSML 跨 chunk 漏 HTML | 网页源码漏到可见文本 | 状态化 `DsmlChunkFilter`（已落地，应迁入 DeepSeekProfile） |
| 7 | shell.command 仅 testing 可见 | 模型说"没有 shell 工具" | manifest 始终含 shell.command；执行时由 PermissionPolicy 处理 |
| 8 | "plan" 工具未注册 alias | UNKNOWN_TOOL → 死 | 加 alias（已落地） |

第 1、7 这两条是 v2 的核心改动，其它已落地。

---

## 9. 文件级迁移清单

按"立即可做 → 中期重构 → 长期目标"分层。

### 9.1 立即（Phase A，1-2 天）

- [x] `runtime_facade.rs` - 删 `*_runtime_tool_exposure_for_prompt` 关键词分支，
  始终 FastAutoWrite（**本次会话已完成**）
- [x] `runtime_facade.rs` - 删 `deepseek_runtime_prompt_wants_generation`（已完成）
- [x] `native_agent_loop.rs` - `iteration_naming_error_results` 不计 non_progress
  （已完成）
- [x] `native_agent_loop.rs` - `effective_max_tool_calls > 0` 守卫（已完成）
- [x] `native_agent_loop.rs` - DsmlChunkFilter（已完成）
- [x] `native_agent_loop.rs` - file.write `base_hash` 自动注入（已完成）
- [x] `native_agent_loop.rs` - `final_visible_output` + `extract_visible_output_from_jsonl`（已完成）
- [x] `runtime_facade.rs` - session_memory 写回 assistant 响应（已完成）
- [x] `kernel/src/tool.rs` - "plan" alias for plan.enter（已完成）

### 9.2 中期（Phase B，1-2 周）

#### 9.2.1 PermissionPolicy 落地

新增文件：
- `crates/runtime/src/permission_policy.rs` —— 实现 `PermissionPolicy` trait 和
  `default/plan/acceptEdits/dontAsk/bypass` 五种内置实现。

修改：
- `crates/runtime/src/native_agent_loop.rs` —— 在 ToolDispatcher 调用前插入
  `permission_policy.evaluate(&ctx)`；删除 `if !manifest_allowed_tools.contains`
  的过滤分支。
- `crates/runtime/src/tool_contract.rs` —— `is_tool_exposed_in_context` 改为
  始终返回 `tool.enabled_by_default && !gated`；删除 `ReadOnly/FastAutoWrite` +
  `workflow_state` 双重过滤。

#### 9.2.2 manifest 全开

- `crates/runtime/src/native_agent_loop.rs:1311-1325` —— 把 manifest 构造里的
  `manifest_workflow_state` 取消（或固定为 `"executing"`），让所有已注册工具
  进入 manifest。
- shell.command 的"仅 testing 可见"逻辑删掉，依赖 PermissionPolicy 在执行
  时 ask。

#### 9.2.3 NativeProfile 抽离

新增：
- `crates/runtime/src/native_profile/deepseek.rs`
- `crates/runtime/src/native_profile/qwen.rs`
- `crates/runtime/src/native_profile/mod.rs` 定义 trait

把以下逻辑搬过去（不变功能，只换归属）：
- `DsmlChunkFilter`（→ DeepSeekProfile）
- `extract_visible_output_from_jsonl`（→ NativeProfile 通用）
- `*_runtime_max_tokens_for_prompt`（→ NativeProfile::max_output_tokens）
- DeepSeek reasoning replay 的 `last_deepseek_reasoning_content` 处理
- DSML/content tool call fallback 提取

### 9.3 长期（Phase C，2-4 周）

#### 9.3.1 Compactor

新增 `crates/runtime/src/compactor.rs`：
- 监听 token usage，接近 80% 时触发；
- 压缩策略：保留 system + 最近 4 个 user/assistant pair + plan + todo + 摘要；
- 压缩后写一条 `context.compaction.completed` 事件。

替换 `repeated_non_progress` finalizer 路径——后者直接删除。

#### 9.3.2 Subagent dispatch tool

参考 Claude Code 的 Task tool：
- 模型调用 `task.dispatch` → runtime 起一个独立 session；
- 子 session 有自己的 budget 和 manifest；
- 完成时把摘要返回父 session 作为 ToolResult。

这能解决 doc 37 提到的"长 horizon 任务上下文污染"，比 `WorkflowFSM` 强制
state-specific manifest 更接近 Claude Code 的实际做法。

#### 9.3.3 Hook system（可选）

`PreToolUse` / `PostToolUse` 通过外部 shell 命令或 WASM 模块。优先级仅次于
`deny`。这是 Claude Code 的典型扩展点，对企业用户特别有用。

---

## 10. 测试与 eval 调整

doc 37 第 22 节列了 22 项 release-blocking eval。v2 把以下条目**修订**：

### 修订项

- ~~"ProjectStatus does not broad-read by default"~~ → 改为
  "ProjectStatus 优先读 evidence 摘要，但**不强制**——给模型更多自主权"
- ~~"Readstorm finalizer and budget enforcement"~~ → 改为
  "Budget enforcement"。Readstorm 不再走 finalizer，靠 observation cache 抑制。
- ~~"WorkflowFSM 状态决定可见工具"~~ → 改为
  "manifest 全开；PermissionPolicy 决定执行通过/拦截"

### 新增项

- "Naming errors do not consume budget"
- "Manifest is identical across turns of the same session config"
- "Plan-mode write attempt returns clear ModelReadableToolError, not crash"
- "Compaction is invoked when context > 80% threshold"
- "Tool error never disables further tool calling within the same turn"

### 释放阻断（v2 红线）

释放必须 block 如果：

- manifest 内容因 prompt 关键词改变；
- naming 错误进 non_progress 计数；
- "permanently disable tools" 路径仍存在；
- DEFAULT_MAX_TOOL_CALLS=0 仍触发 finalizer；
- base_hash 仍要求模型提供；
- assistant 响应仍不进 session_memory；
- DSML 仍因跨 chunk 漏出 HTML；
- DeepSeek `reasoning_content` 在 tool turn 后丢失；
- shell.command 仅在 `workflow_state=="testing"` 可见。

---

## 11. 与 doc 37 的关系总结

| doc 37 节 | v2 状态 |
|---|---|
| §1-6 (问题陈述、目标、非目标、架构、与 ClaudeCode/OpenCode 关系、与 mega plan 关系) | **保留**，事实未变 |
| §7.1 AgentKernel 责任 | **修订**：加 PermissionPolicy 和 Compactor，删 WorkflowFSM 强主导 |
| §7.2 TCML | **保留** |
| §7.3 NativeModelProfile | **保留**并扩充（加 ReasoningReplayPolicy 等子模块） |
| §8 Turn Routes | **保留**但 routes 仅决定 budget/scaffold，**不决定 manifest** |
| §9 Workflow FSM | **降级**：FSM 仅作为内部记账（plan_pending、edit_in_flight 等状态机标记），**不**用于 manifest 切除 |
| §10 ToolManifest | **保留**，但 manifest 不再被 turn-level state 过滤 |
| §11 Tool Naming/Alias | **保留** |
| §12 Streaming Tool Calls | **保留** |
| §13 Validate-then-repair | **保留** |
| §14 ModelReadableToolError | **保留**并强化（加 plan-required deny reason） |
| §15-16 DeepSeek/Qwen Profile | **保留**，移到 NativeProfile 文件 |
| §17 ProviderCapabilityMatrix | **保留** |
| §18 Context/Evidence | **保留** |
| §19 Parallel Tool Policy | **保留** |
| §20 Event Model | **保留**，新增 `permission.evaluated`、`compaction.started/completed` |
| §21 ToolDoctor | **保留** |
| §22 Eval Gates | **修订**（见 §10 of v2） |
| §23 Codebase 集成 | **保留**，按 §9 of v2 落地清单 |
| §24 Implementation Plan | **重排**：见 §9 of v2 的 Phase A/B/C |
| §25 Architectural Invariants | **修订**：见 §2 of v2 |
| §26 Final Architecture Decision | **保留** |

---

## 12. 一句话总结（架构层）

> doc 37 把 ClaudeCode 的"runtime 严格"和 mega plan 的"工具契约严格"装到了一个
> 三层骨架里。v2 把 doc 37 中"manifest 按 turn-state 切除"这个 OpenCode 风格
> 的设计，换成 Claude Code 的"manifest 不切，权限切"——这样 DeepSeek/Qwen 不
> 再因为关键词启发式或 FSM state 错判而失去工具，模型行为回到符合直觉的状态，
> 同时保留了 Claude 级别的执行时安全护栏和 NativeProfile 对 DeepSeek
> reasoning replay、DSML 漏出、缓存前缀等问题的专属处理。

---

# Part II: Claude-Code-Grade Maturity（架构骨架之外的真正"顺手"）

§1–§12 给了一份**安全护栏**——做完这部分，agent 不会再因为关键词启发式或
FSM state 把工具切掉，也不会因为打错一次工具名就死。

但"顺手"不止是不出 bug。Claude Code 用起来感觉成熟，是因为很多在 doc 37 里没
被命名的**惯例**和**机制**，比如：每一次 tool 调用之后用户能立刻看到结构化
结果而不是 raw dump、长任务由 subagent 隔离、context 即将爆炸时自动压缩、
plan mode 是真正一种 mode 而不只是一个工具调用。

Part II 把这些惯例**逐项命名**，给出当前 ResearchCode 的差距和具体落地办法。

---

## 13. ConversationHistory：保真还是摘要？

### 13.1 Claude Code 的做法

每一轮 turn 的输入是**完整对话历史**：所有 user 消息、所有 assistant 消息（含
tool_use 块）、所有 tool_result 块，按时间顺序排好。系统不"摘要"对话，除非触发
auto-compaction。

这意味着：

- 模型每一轮都看到此前所有 tool 调用的**完整结果**；
- 用户问"上次那个文件长什么样"，模型能直接看到此前 read 的内容；
- 多轮共享同一个文件状态，不需要每轮重读。

### 13.2 ResearchCode 现状

`session_memory` 存的是**摘要**：
- user 消息存了截断到 320 char 的版本；
- assistant 响应**之前根本没存**（本次会话已修复）；
- tool 调用只存 `operational stats`，不存内容。

每一轮新 turn 的 prompt 是 `build_context_bundle()` 拼出来的——拿 session_memory
+ AGENTS.md + 文件状态。这是"curated bundle"，不是"完整历史"。

### 13.3 这两种做法的取舍

| | Claude Code 全历史 | ResearchCode 摘要 bundle |
|---|---|---|
| 多轮一致性 | 强 | 弱 |
| 长 session token 消耗 | 高，依赖 compaction | 低 |
| 突发问题"上次那个 X" | 模型直接答 | 需重读 |
| 实现复杂度 | 历史 + auto-compact | bundle 拼装策略 |

### 13.4 v2 决定

**默认走 Claude Code 路线**：每一轮 turn 把当前 session 的完整对话历史（user
+ assistant + tool_use + tool_result）传给模型；当 token usage > 80% threshold
时触发 Compactor 自动压缩。

但保留 `build_context_bundle` 作为 system 段落的**前置摘要**，用于：

- AGENTS.md / RESEARCHCODE.md 项目说明；
- 工作目录元信息；
- 最近 `git status` 摘要；
- 当前 plan / todo 列表；

`build_context_bundle` 不再尝试代替 conversation history，它和 conversation
history **并列**进 prompt：

```
[system]
  base system prompt
  project context bundle (AGENTS.md, git status, plan, todo)

[user] turn 1
[assistant] turn 1 (text + tool_use)
[tool_result] turn 1
[assistant] turn 1 continuation
[user] turn 2
...
```

### 13.5 落地清单

- [ ] `crates/runtime/src/native_agent_loop.rs` 中拼请求时，把当前 session 的
  完整对话历史（来自 EventLog）转成 messages，而不是只用 session_memory 摘要；
- [ ] `session.rs` 提供 `to_conversation_messages()` 方法把 EventLog 里的
  `user.input_received`、`assistant.response_recorded`、`tool.call.requested`、
  `tool.call.completed` 转成 OpenAI/Anthropic 标准 message format；
- [ ] `build_context_bundle` 改为只生成 system 段落的"项目上下文"部分，不再
  代替 history。

---

## 14. ToolResult Quality Contract

### 14.1 Claude Code 的做法

每个工具的返回值是**结构化、自描述、模型友好**的。例如：

- `Read`: 返回带行号前缀的内容，文件名 header，是否被截断的标记；
- `Bash`: 返回 stdout、stderr、exit_code 三段；
- `Glob`: 返回匹配文件路径列表，按修改时间排序；
- `Edit`: 返回 diff 或"成功"标记，失败时返回精确错误（"old_string not found"，
  并提示是否重复出现）；
- `Grep`: 返回匹配行 + 上下文，用文件分组。

错误是**结构化的 ModelReadableToolError**：包含 `error_code`、`field_errors`、
`fix_hint`、可选 `retry_example`。

### 14.2 ResearchCode 现状

工具结果有 `preview` 和 `detail_json` 两段，但：

- `preview` 通常是截断的字符串，**没有统一格式**；
- `detail_json` 是工具自定义的 schema，模型不一定能稳定解析；
- 错误有时是 raw error string，有时是 ModelReadableToolError，**不一致**。

### 14.3 v2 决定：每个工具一份契约

为每个工具定义"成功返回 + 失败返回"的固定 schema。仿 Claude Code，列举
ResearchCode 当前所有工具应当的返回形态：

| Tool | 成功 preview | detail_json keys | 失败要点 |
|---|---|---|---|
| `file.read` | 行号前缀 + 内容 | `path, start_line, end_line, total_lines, truncated` | path 是目录 → 提示用 list_directory |
| `file.list_directory` | 缩进树 / `name<TAB>type` 列表 | `path, entries[]` | path 不存在 → 提示父目录 |
| `file.list_tree` | 树状缩进 | `root, depth, entries[]` | depth 太大 → 建议缩小 |
| `file.write` | "Wrote N bytes to path" | `path, bytes_written, base_hash, new_hash` | path readonly → 列允许根 |
| `file.edit` | "Replaced 1 occurrence" 或 diff | `path, replacements, base_hash, new_hash` | old_string not found → 提示用 read 确认 |
| `file.multi_edit` | 每个 edit 的 diff | `path, edits[], base_hash, new_hash` | 任一 edit 失败回滚 → 全部不应用 |
| `search.ripgrep` | 文件分组的命中行 | `pattern, files[], total_matches` | 无匹配 → 直接说"无命中"，**不**当错误 |
| `repo.map` | 树 + 文件计数 | `roots[], file_count, lang_breakdown` | 大仓库 → 自动 depth 限制 |
| `git.status` | porcelain 输出 | `branch, ahead, behind, modified[], untracked[]` | 不在 git 仓库 → 明确说 |
| `shell.command` | stdout 头 + tail | `command, stdout, stderr, exit_code, cwd, duration_ms` | exit≠0 不当 ok=false，模型自己看 exit_code |
| `plan.enter` | "Plan submitted: <id>，等待审批" | `plan_id, status: pending` | — |
| `todo.write` | 当前 todo 数量 + 新增 | `items[]` | — |
| `ask_user` | 问题预览 | `question, suspended_at` | — |

### 14.4 关键约束

- `preview` 是**给模型看**的人类可读形式，长度上限 800 char，超出加 `...truncated...` 行；
- `detail_json` 是 stable schema，每个工具定义 TypeScript 风格的类型；
- 失败必须是 `ModelReadableToolError`，**不能**直接 propagate Rust error；
- shell 命令 exit≠0 **不**当 ok=false——这是常见事实（grep 找不到匹配也 exit 1），
  让模型自己读 exit_code 决定如何继续。

### 14.5 落地清单

- [ ] `crates/runtime/src/tool_execution.rs` 每个 `execute_*` 函数检查输出格式，
  统一 preview 800 char 上限和"...truncated..."标记；
- [ ] 新增 `crates/runtime/src/tool_result_schema.rs` 定义每个工具的 detail_json
  字段；
- [ ] shell.command 把 exit≠0 改为 ok=true（除非 spawn 失败）；
- [ ] file.read 的 preview 保证带行号前缀（参考 Claude Code 的 `cat -n` 风格）。

---

## 15. PlanMode：真正的 mode，不只是一个工具

### 15.1 Claude Code 的做法

Plan mode 是 session 的**全局状态**，由系统消息显式告诉模型：

```
[system]
You are currently in plan mode. You have full read access but you cannot execute
any actions that modify the filesystem or run code. Use ExitPlanMode to present
your plan to the user when ready.
```

在 plan mode 下：

- 所有工具仍在 manifest（包括 Write、Edit、Bash）；
- 模型尝试调用写工具 → runtime 在执行点拒绝并返回明确错误；
- 模型调用 `ExitPlanMode` → runtime 暂停 turn，等待用户 approve/reject；
- 用户 approve → 模式从 plan 切到 default（或 acceptEdits）→ 继续执行；
- 用户 reject → 模式留在 plan，模型必须重新规划。

### 15.2 ResearchCode 现状

`plan.enter` 是个工具，调用后会触发 `plan_approval_pending`。但：
- 没有"plan mode is active"的全局状态；
- 模型调用写工具不被 plan mode 拦截，是被 ReadOnly exposure 拦截（v2 已删）；
- 用户 approve 流程在 desktop 是手动的，没接入到模式切换。

### 15.3 v2 决定

引入 `PermissionMode` 枚举，作为 session 的全局状态：

```rust
enum PermissionMode {
    Default,         // 写/shell 需要 ask
    Plan,            // 写/shell 全部 deny，提示 ExitPlanMode
    AcceptEdits,     // 写自动 allow，shell 仍需 ask
    BypassPermissions, // 全部 allow（仅 dev 模式）
}
```

session 持有 `current_mode: PermissionMode`，每个 turn 的 PermissionPolicy
拿 mode 做判断（参 §4.3）。

`plan.enter` 调用 → runtime 把 mode 切到 `Plan`（如果还不是），同时记录
`plan_approval_pending`。用户 approve 把 mode 切到 `AcceptEdits`（默认），用户
也可在 GUI 显式选其他 mode。

System prompt 要包含"You are currently in <mode> mode"段落，这样模型自己也
能据此调整策略（v2 NativeProfile 负责注入这一段）。

### 15.4 落地清单

- [ ] `crates/kernel/src/session.rs` 加 `PermissionMode` 字段；
- [ ] `crates/runtime/src/permission_policy.rs`（新文件）实现五个模式的决策表；
- [ ] `plan.enter` 工具执行体改为：把 session mode 切到 Plan（如果还不是），
  emit `plan.approval.requested`；
- [ ] System prompt 注入由 NativeProfile 完成，含 mode 段落；
- [ ] GUI 加模式切换控件，显示当前 mode badge。

---

## 16. Subagent (Task tool)

### 16.1 Claude Code 的做法

`Task` 工具让父 agent 派发独立子任务给子 agent：

- 子 agent 有**自己的 context window**（系统起步，没有父 history）；
- 子 agent 有**自己的工具集**（可与父不同）；
- 子 agent 完成后返回**单一 string 摘要**给父；
- 父只看到摘要，不看到子的中间过程。

价值：
- 长 horizon 探索（"调研整个 X 模块"）不污染父 context；
- 并行多任务（多个 Task 同时跑）；
- 隔离失败（子失败不拖垮父）。

### 16.2 ResearchCode 现状

无。所有探索都在主 session 中，导致：
- "调查这个 bug"会塞 20+ tool calls 进主 history；
- compaction 不及时就直接耗尽 context；
- 失败传染。

### 16.3 v2 决定

新增 `task.dispatch` 工具：

```ts
type TaskDispatchArgs = {
  prompt: string;          // 给子 agent 的任务说明
  agent_type?: string;     // 子 agent 的角色（"explorer" / "reviewer" / 默认）
  expected_output?: string; // 期望摘要长度/格式
};

type TaskDispatchResult = {
  task_id: string;
  summary: string;
  artifact_refs: string[]; // 子 session 产生的 artifact，父可按需读
  total_tool_calls: number;
  duration_ms: number;
};
```

实现要点：

- 子 session 是**真 session**（有自己的 EventLog、自己的 budget），跑完 archive；
- 子 session 的 manifest 默认是父的子集（read-only / explorer），可由 agent_type 指定；
- 子 session 的 prompt 由父的 prompt + 项目 context bundle 组成（**不**含父
  history，独立起步）；
- 子完成后，父收到的 ToolResult 是 `TaskDispatchResult` 的 detail_json，preview
  是 summary 字符串。

### 16.4 落地清单

- [ ] `crates/runtime/src/task_dispatch.rs`（新文件）实现 `task.dispatch` 工具；
- [ ] `crates/kernel/src/tool.rs` 注册 `task.dispatch` ToolSpec；
- [ ] runtime_facade 提供 `start_subagent_session(parent_id, prompt, agent_type)`；
- [ ] GUI 把 task dispatch 渲染为可展开的"子任务"卡片，点开看子 session 全貌。

---

## 17. Compactor 详细设计

### 17.1 触发条件

- token usage > 80%；
- 或显式 user `/compact` 命令；
- 或 turn 进入第 N 个迭代（N=20 默认）。

### 17.2 保留策略

每次 compaction 保留：

- `system` 段（项目 context bundle 总在重建，不需要从历史复刻）；
- 最近 4 个 user/assistant pair 完整保留；
- 当前活跃 plan（如有）；
- 当前活跃 todo list（如有）；
- 任何被 EvidenceLedger 标记为"key evidence"的 tool result。

### 17.3 压缩策略

中间历史压缩为单条 `[system]` summary block：

```
[system: prior conversation summary]
The user originally asked X. Through N turns of investigation:
- Found Y in file Z.
- Tried approach A, failed because B.
- Currently considering option C.

Key evidence preserved:
- artifact://abc123 (file.read of src/foo.rs:1-200)
- artifact://def456 (search.ripgrep results for "bar")
```

summary 由专门的 compaction 模型调用生成（可以用同一个模型，但走单独的
"summarize this conversation"prompt）。

### 17.4 与 finalizer 的关系

doc 37 有 `repeated_non_progress` finalizer 和 `loop_guard` finalizer。v2
**全删**：

- 它们的目的是"卡住时不要无限消耗 token"——但这事 budget 已经做了；
- 它们的副作用是"突然把工具禁用"——这违反 v2 invariant 17（naming errors are free retries）；
- compactor 提供更优雅的"上下文太满"应对。

### 17.5 落地清单

- [ ] 新增 `crates/runtime/src/compactor.rs`；
- [ ] `crates/runtime/src/native_agent_loop.rs` 删 `repeated_non_progress` 和
  `loop_guard` 两条 finalizer 路径；
- [ ] `compactor.compact_session(session, target_token_count)` 返回新的
  conversation history；
- [ ] Token usage 跟踪写入 `EventLog`，每个 model_call 都包含 `tokens_in / tokens_out`。

---

## 18. 把 native_agent_loop.rs 拆成组件

### 18.1 现状

`native_agent_loop.rs` ~3500 行，一个大函数 `run_native_agent_loop_v2_inner`
做了：

1. session 状态管理；
2. manifest 构建；
3. prompt 拼装；
4. provider 请求；
5. 流式响应解析（DSML、reasoning、tool calls、visible text）；
6. tool 调用 mediation；
7. tool 执行；
8. 各种 recovery（loop_guard, non_progress, empty_visible, plan_approval）；
9. finalizer 路径；
10. event 派发。

读起来像一团毛线。改一处经常牵动十处。

### 18.2 v2 拆分

按**单一职责**拆成组件，每个 < 600 行，loop 主体 < 400 行：

```text
crates/runtime/src/agent_kernel/
  mod.rs                  // pub use 各组件，定义 AgentKernel struct
  loop.rs                 // 主循环 — 只负责按顺序调用各组件
  turn_state.rs           // 替代散落的 *_recovery_count，单一 TurnState
  permission_policy.rs    // §4 PermissionPolicy
  budget_policy.rs        // §5 BudgetPolicy + token tracker
  compactor.rs            // §17
  stream_handler.rs       // 流式响应解析（含 DsmlChunkFilter 调用 NativeProfile）
  tool_batch.rs           // 一批 tool calls 的执行单元
  conversation_history.rs // §13 history 拼装

crates/runtime/src/native_profile/
  mod.rs                  // NativeModelProfile trait
  deepseek.rs             // DSML、reasoning replay、cache prefix
  qwen.rs                 // template 校验、parser flag、token budget
```

### 18.3 主循环骨架

`loop.rs` 应该读起来像这样（伪代码）：

```rust
pub fn run_turn(kernel: &mut AgentKernel, request: TurnRequest) -> TurnResult {
    let mut state = TurnState::new(&request);

    kernel.turn_router.classify(&request, &mut state);
    kernel.budget_policy.set_initial(&mut state);

    loop {
        if state.should_compact() {
            kernel.compactor.compact(&mut state)?;
        }

        let history = kernel.conversation_history.assemble(&state);
        let manifest = kernel.tool_manifest.build(&state); // 静态、不按 turn-state 切
        let request = kernel.native_profile.prepare_request(&state, history, manifest);

        let response = kernel.provider_adapter.send(request, &mut state.stream_handler)?;

        if response.is_final_text() {
            return TurnResult::Done(response.text());
        }

        let validated_calls = kernel.tcml.mediate_all(response.tool_calls);
        let batch = kernel.tool_batch.from(validated_calls);

        for call in batch {
            match kernel.permission_policy.evaluate(&call, &state) {
                Allow => kernel.dispatcher.execute(call, &mut state),
                Ask(req) => return TurnResult::AwaitUser(req),
                Deny(reason) => state.append_tool_result(model_readable_error(reason)),
            }
        }

        kernel.budget_policy.consume(&mut state, &batch);

        if !state.budget_ok() {
            return TurnResult::BudgetExhausted(state);
        }
    }
}
```

读完这 30 行，agent loop 的全部逻辑在脑子里就清晰了——这是 maturity 的核心
特征：**架构本身可读**。

### 18.4 落地清单（分阶段）

阶段 1：**抽出 TurnState**
- [ ] 把 `loop_guard_recovery_count`、`non_progress_recovery_count`、
  `empty_visible_recovery_count`、`tool_call_count`、`model_call_count`、
  `last_tool_batch`、`seen_tool_batches`、`observation_cache`、
  `last_deepseek_reasoning_content` 全合并进一个 `TurnState`；
- [ ] 把所有 `let mut foo` 局部变量改读 `state.foo`；
- [ ] 不改逻辑，只搬位置——保证测试全绿。

阶段 2：**抽出 PermissionPolicy**
- [ ] 新增 `permission_policy.rs`，移过去 `if !manifest_allowed_tools.contains` +
  `mediated.error.is_some()` 的判断；
- [ ] loop 主循环 30 行内只看到一个 `permission_policy.evaluate(&call, &state)`。

阶段 3：**抽出 NativeProfile**
- [ ] DsmlChunkFilter、`extract_visible_output_from_jsonl`、
  `*_runtime_max_tokens_for_prompt`、reasoning replay 全搬到
  `native_profile/deepseek.rs` 或 `qwen.rs`；
- [ ] loop 中所有 `if family == DeepSeek` 删除，全部走 `kernel.native_profile.X()`。

阶段 4：**Compactor + 删 finalizer**
- [ ] 加 `compactor.rs`；
- [ ] 删 `non_progress_recovery_count` 路径；
- [ ] 删 `loop_guard_recovery_count` 路径；
- [ ] turn 结束条件简化为 `Done | AwaitUser | BudgetExhausted`。

阶段 5：**ConversationHistory**
- [ ] EventLog → conversation_messages 转换；
- [ ] 删 `build_context_bundle` 中代替 history 的部分，只留项目摘要。

阶段 6：**Subagent**
- [ ] 加 `task.dispatch` 工具；
- [ ] 子 session 起步逻辑。

每阶段完成后跑一次完整测试套件（含集成测试），确保没回退。

---

## 19. TurnState 类型设计

```rust
/// Single source of truth for one turn's runtime state.
/// Replaces ~12 scattered local variables in the current loop.
pub struct TurnState {
    // identity
    pub session_id: String,
    pub turn_index: u32,
    pub started_at: Instant,

    // routing & budget
    pub route: TurnRoute,
    pub budget: TurnBudget,
    pub mode: PermissionMode,

    // counters (consumed against budget)
    pub iterations: u32,
    pub tool_calls: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,

    // batch tracking (informational, not for ban decisions)
    pub seen_tool_batches: Vec<String>,
    pub observation_cache: ObservationCache,
    pub last_tool_batch: Vec<ToolBatchEntry>,

    // native quirks (delegated to NativeProfile)
    pub last_reasoning_content: Option<String>,
    pub stream_handler_state: StreamHandlerState,

    // permission flow
    pub awaiting_user: Option<AwaitingUserRequest>,

    // emit tracking
    pub emitted_event_count: usize,
}

impl TurnState {
    pub fn budget_ok(&self) -> bool {
        self.iterations < self.budget.max_iterations
            && self.tool_calls < self.budget.max_tool_calls.unwrap_or(u32::MAX)
            && self.tokens_out < self.budget.max_tokens
    }

    pub fn should_compact(&self) -> bool {
        let usage_ratio = self.tokens_in as f64 / self.budget.context_window as f64;
        usage_ratio > 0.8
    }

    pub fn record_tool_result(&mut self, _: &ToolResult) { /* ... */ }
}
```

注意**没有** `loop_guard_recovery_count` 和 `non_progress_recovery_count`。
这是有意的。

---

## 20. 失败恢复决策树（取代 finalizer）

任意 turn 结束时，按这个表决定下一步：

| 模型输出 | 工具结果 | 预算 | 决策 |
|---|---|---|---|
| 有 final text | (任意) | (任意) | `Done(text)` |
| 无 text，有 tool calls | 全 ok | 充裕 | 下一轮 |
| 无 text，有 tool calls | 全错（执行类） | 充裕 | 下一轮（让模型看错误） |
| 无 text，有 tool calls | 全错（命名类） | 充裕 | 下一轮（**不计预算**） |
| 无 text，有 tool calls | 部分错 | 充裕 | 下一轮 |
| 无 text，无 tool calls | — | 充裕 | "请你给一个回答或调用工具"提示，下一轮 |
| (任意) | (任意) | tokens 耗尽 | 触发 Compactor，下一轮 |
| (任意) | (任意) | iterations 耗尽 | `RequestFinalAnswer`（让模型给当下能给的答案） |
| (任意) | (任意) | tool_calls 耗尽 | manifest 临时去掉所有工具，下一轮强制出 text |
| 模型说 plan.enter | — | — | `AwaitUser(plan_approval)` |
| 工具说 ask_user | — | — | `AwaitUser(question)` |
| 用户中断 | — | — | `Cancelled` |

**没有**"两次失败就禁用工具"的路径。**没有**"重复 batch 就 finalizer"的路径。
budget 是唯一的硬性边界。

---

## 21. 系统 Prompt 组成（NativeProfile 责任）

每一轮 turn 的 system 段落由 NativeProfile 拼，模板如下：

```text
{base_system_prompt_for_family}

You are currently in {permission_mode} mode.
{mode_specific_guidance}

# Project Context
{project_context_bundle}

# Active Plan
{current_plan_or_none}

# Active Todos
{current_todos_or_none}

# Tool Calling Rules
- Use only tools from the catalog below.
- Filesystem paths must be raw paths, not markdown links.
- For valid tool errors, follow the retry example exactly.
- {family_specific_rules}

# Available Tools
{tool_catalog_json_or_descriptive_listing}
```

各 mode 的 guidance：

- `Default`: "Writes and shell commands will ask for user approval before execution."
- `Plan`: "You cannot modify files or run shell commands. Investigate freely with read tools, then call ExitPlanMode (i.e., plan.enter) to present your plan."
- `AcceptEdits`: "Edits to files are auto-approved. Shell commands still ask."
- `BypassPermissions`: "All tools execute without prompting. Use caution."

family-specific rules：

- DeepSeek: "Emit OpenAI-style tool_calls JSON; do not use DSML/XML in visible text."
- Qwen: "Use small, precise, patch-sized edits."

---

## 22. Streaming UX 事件目录

GUI 要看到的事件清单（doc 37 §20 的具体化）：

| 事件 | 何时触发 | GUI 渲染 |
|---|---|---|
| `agent.turn_classified` | TurnRouter 分类完成 | 显示 "thinking..." |
| `model.request_prepared` | 请求送出 | "model is responding..." |
| `model.stream_delta` (visible) | 模型流式输出文本 | 实时追加文本 |
| `thinking.chain.delta` | 模型流式输出 reasoning（DeepSeek thinking） | 折叠的"Thinking..."块 |
| `tool_call.assembling` | 流式 tool call 累加中 | "Calling X..." spinner |
| `tool_call.assembled` | tool call 完整 | spinner 替换为参数预览 |
| `tool.permission.evaluated` | PermissionPolicy 决策 | 如果 Ask → 弹 approval |
| `tool.execution.started` | 真正执行 | "Executing X" |
| `tool.execution.completed` | 执行完 | 结果卡片 |
| `tool.error.model_readable` | 工具错误 | 黄色警告卡片（不是红色—模型会重试） |
| `compaction.started` | Compactor 启动 | "Compacting context..." |
| `compaction.completed` | 压缩完 | "Context compacted: X→Y tokens" |
| `agent.turn_summary` | turn 结束 | 摘要 + 总耗时 + token 统计 |

**关键**：`tool.error.model_readable` 不能渲染成"失败"——它是**信息**，不是
终态。当前 GUI 把 UNKNOWN_TOOL 渲染成红色"工具失败"是误导用户。v2 改为黄色
警告 + "agent will retry"标签。

---

## 23. 落地优先级矩阵

按"用户感知改善 vs 实现难度"排序：

| 优先级 | 项 | 用户感知 | 难度 | 阻塞物 |
|---|---|---|---|---|
| P0 | §1-12 v2-A 的修复（已完成大部分） | 高 | 低 | — |
| P0 | shell.command 全开 + manifest 不切（本次会话） | 高 | 低 | 测试需更新 |
| P1 | §15 Plan mode 真实化 | 高 | 中 | PermissionPolicy 模块 |
| P1 | §13 ConversationHistory 全保真 | 高 | 中 | EventLog → message 转换 |
| P1 | §14 ToolResult 统一格式 | 高 | 中 | 改 tool_execution 各分支 |
| P1 | §17 Compactor + 删 finalizer | 高 | 中 | Compactor 实现 |
| P1 | §22 Streaming UX 事件正名 | 中 | 低 | GUI 渲染调整 |
| P2 | §18 native_agent_loop 拆组件 | 低（架构内部） | 高 | 重构周期 |
| P2 | §16 Subagent / task.dispatch | 高 | 高 | 子 session 隔离 |
| P3 | hook 系统 / MCP 集成 | 中 | 高 | 外部接口稳定 |

P0 紧迫，P1 一旦做完用户体验就达到 Claude Code 级，P2 是内部 maturity，
P3 是 ecosystem 扩展。

---

## 24. 哪些 Claude Code 特性**不**搬

为防止过度拷贝，明确不做：

- **Anthropic wire format**：DeepSeek/Qwen 是 OpenAI-compatible，不强求换。
- **ClaudeCode 长 system prompt**：Claude Code 的 system prompt 数千 token，
  对 DeepSeek 太重，NativeProfile 用更精简的版本。
- **Skill / Slash command 完整体系**：可以做，但不是 P1。
- **Memory tool**：Claude Code 的 memory 工具语义上和我们的 session_memory
  重合，不再加新概念。
- **Worktree 自动创建**：Claude Code 在某些场景自动起 worktree——我们靠
  PermissionPolicy + 用户显式触发即可，不需要自动化。
- **Browser/Computer use 工具**：完全超出范围。

---

## 25. 一句话总结（成熟度层）

> v2 Part I 的目标是"不出错"——把关键词启发式、过度严苛的 recovery、按 state
> 切 manifest 这些反模式都拆掉，让 agent 行为符合直觉。Part II 的目标是"用着
> 顺手"——让对话保真、工具结果稳定可读、plan 是真模式、长任务有 subagent、
> 接近上限自动 compact、UI 把工具错误正确分类成"信息"而非"事故"。两层加起来，
> ResearchCode 才能同时做到 Claude Code 级架构成熟度和 DeepSeek/Qwen 友好的
> NativeProfile 优化。
