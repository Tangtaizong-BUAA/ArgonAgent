# 重复读取 → Plateau → 空文本 fallback 完整修复方案

> 基于 `runtime_session_1779077180193148000` 日志分析 + Open-ClaudeCode 参考实现
>
> 关联文档：[tool_evidence_accumulation_repair.md](./tool_evidence_accumulation_repair.md)

---

## 问题陈述

桌面应用中观察到稳定可复现的失效模式：

1. 用户输入「继续按照plan」之类的延续指令
2. 模型连续 8~16 次调用 `file.read`，全部命中 `ObservationCache` 的 duplicate 拦截
3. `ConvergenceEnforcer` 触发 `DuplicateDominance{ratio: 1.0, window: 2}`，把循环终止
4. UI 看到的最终输出是：
   > 本轮已完成，但模型没有返回可展示文本。 可在同一会话继续，runtime 会复用已有证据。
5. 用户再次发送任何消息，记忆里携带上一轮的 fallback 文本（"assistant: 本轮已完成..."），形成毒化上下文，下一轮重蹈覆辙

事件日志关键片段（`runtime_events.jsonl` seq 4228–4232）：

```json
{"event_type":"agent.loop_plateau_finalized","payload":{"reason":"duplicate_dominance"}}
{"event_type":"model.stream_delta","payload":{
  "preview":"本轮已完成，但模型没有返回可展示文本。...",
  "stream_id":"native_loop_v2_convergence_fallback_stream_loop_2933"}}
{"event_type":"agent.turn_summary","payload":{
  "completion_status":"completed","tool_batch_size":0,"evidence":[],
  "user_task":"继续按照plan\n...\n[Memory] tool file.read ok=true preview=duplicate observation skipped..."}}
```

---

## 三层根因

### 层 1 — 部署的二进制是中间版本（运维问题）

`"本轮已完成，但模型没有返回可展示文本。"` 字符串在仓库的**任何分支**都不存在。它只以**否定断言**形式出现在测试中（`native_agent_loop.rs:12120, 12197`：`assert!(!event_jsonl.contains(...))`）。

| 版本 | Fallback 文本 | 是否有 `try_visible_finalizer_from_evidence` |
|---|---|---|
| 旧 worktree (`pensive-feynman-297ddb`) | 英文 "I completed this turn but the model returned no visible text." | ❌ |
| **运行中二进制（推断）** | "本轮已完成，但模型没有返回可展示文本。" + `_loop_N` stream id | ❌ |
| 当前 main | `synthesized_visible_finalizer_message`（语境化中文）| ✅ |

→ **立即措施：从 main 重新编译并部署**。这一步能消除字符串本身，并启用模型合成 fallback。

### 层 2 — Plateau 触发时 `last_tool_batch` 为空，模型合成上下文薄弱

`record_native_loop_turn_summary` 看到 `tool_batch_size: 0, evidence: []`。当前 main 的 `complete_native_loop_with_visible_finalizer_or_fallback`（`native_agent_loop.rs:8303`）：

```rust
let continuation_view = continuation_view_for_batch(evidence_ledger, tool_batch);
let has_evidence = !continuation_view.is_empty()
    || !continuation_view.history_digest.is_empty()
    || continuation_view.suppressed_count > 0
    || !tool_batch.is_empty();
```

`has_evidence = true`（history_digest 有 11 轮历史），所以 `try_visible_finalizer_from_evidence` 会被调用。但提供给模型的合成上下文只有：

- 11 轮 file.read 历史的 160 字符 preview
- `suppressed_count`
- 没有实际工具结果可引用

模型被要求"用已有证据写最终答案"，但用户的任务是"继续按照plan"——**需要写代码而不是写总结**。DeepSeek-flash 在这种语境下大概率返回空文本，触发 `Ok(false)` 路径，回落到 fallback。

### 层 3 — 模型不会从"读循环"切换到"写循环"（根本行为缺陷）

会话记忆已经包含全部需要的文件（11 个 Swift/Markdown），但模型仍然反复 `file.read`，原因链：

1. **重复抑制消息把模型推回读循环**
   现在的 message 是：
   > "duplicate observation skipped for file.read; reuse prior evidence or **inspect a narrower target**"
   
   模型把"换个 target"理解为"读不同文件"，而不是"开始写代码"。

2. **Evidence Ledger 指令是软约束**
   上下文里写着 *"Do not reread covered plan/file ranges; continue implementation from this evidence"*。DeepSeek-flash 在长上下文中的指令遵从率不高，每次都忽略它。

3. **会话记忆毒化**
   `session_memory` 注入了 `"assistant: 本轮已完成..."` 和 `"user: ？"`，模型看到这些当作上轮失败信号，倾向于"先重新摸底"。

4. **路由器粗粒度**
   `deepseek_runtime_tool_exposure_for_prompt`（`runtime_facade.rs:3612`）对一切 prompt 返回 `FastAutoWrite`：

   ```rust
   fn deepseek_runtime_tool_exposure_for_prompt(prompt: &str) -> NativeAgentToolExposure {
       let _ = prompt;
       NativeAgentToolExposure::FastAutoWrite
   }
   ```

   暴露 read+write 全集，模型自由选择，但 plateau 触发后没有任何机制收紧暴露面。

---

## Open-ClaudeCode 的对照设计

### A. `FILE_UNCHANGED_STUB` —— 工具层级的内容级 dedup

`src/tools/FileReadTool/prompt.ts:7`：

```ts
export const FILE_UNCHANGED_STUB =
  'File unchanged since last read. The content from the earlier Read tool_result in this conversation is still current — refer to that instead of re-reading.'
```

`FileReadTool.ts:540-572` 的核心逻辑：

```ts
const existingState = readFileState.get(fullFilePath)
if (existingState && !existingState.isPartialView && existingState.offset !== undefined) {
  const rangeMatch =
    existingState.offset === offset && existingState.limit === limit
  if (rangeMatch) {
    const mtimeMs = await getFileModificationTimeAsync(fullFilePath)
    if (mtimeMs === existingState.timestamp) {
      logEvent('tengu_file_read_dedup', { ... })
      return { data: { type: 'file_unchanged' as const, file: { filePath } } }
    }
  }
}
```

关键设计：
- **mtime 校验**：文件磁盘 mtime 与上次读取记录的 timestamp 匹配才视为重复
- **范围精确匹配**：offset+limit 一致才走 stub
- **stub 明确指向上文证据**：*"refer to that instead of re-reading"*，不模糊地说"换 target"
- **客户端单边实现**：不依赖 server，Bedrock/Vertex/Foundry 都安全
- **可灰度关闭**：`tengu_read_dedup_killswitch`

### B. `SyntheticOutputTool` —— 强制最终回答工具

`src/tools/SyntheticOutputTool/SyntheticOutputTool.ts:50-52`：

```ts
async prompt(): Promise<string> {
  return `Use this tool to return your final response in the requested structured format. You MUST call this tool exactly once at the end of your response to provide the structured output.`
}
```

启用条件 `isSyntheticOutputToolEnabled`：当 `isNonInteractiveSession: true` 时，注入一个名为 `StructuredOutput` 的工具，其 `prompt` 强制模型必须调用一次输出最终答案。它是 `isReadOnly: true`，不影响实际工具集，但提供了一个**强制的写出口**。

### C. 自然终止条件 —— `needsFollowUp` boolean

`src/query.ts:834`：

```ts
needsFollowUp = toolUseBlocks.length > 0
```

主循环只有在 assistant message 包含 tool_use blocks 时才继续；如果模型直接说话，循环自然结束。**没有显式的 plateau 检测**——靠工具层级 dedup stub 让模型自己改变行为。

---

## 修复矩阵

按修复成本和效果排序，分四批落地。

### 批 1：消除毒化与字符串污染（30 分钟，必做）

#### 1.1 重新编译部署 main 二进制

```bash
cd /Users/gongyuxuan/Documents/deep-code
cargo build --release -p runtime --bin <桌面后端可执行文件名>
# 替换桌面应用调用的二进制
```

验证：跑一个新 session，确认 `agent.loop_recovery` 之前出现 `agent.visible_finalizer.completed` 事件，stream_id 不再带 `_loop_N` 后缀。

#### 1.2 过滤 plateau fallback 不进入 session_memory

文件：`crates/runtime/src/native_agent_loop.rs:emit_runtime_visible_finalizer_fallback`（main 中的 `synthesized_visible_finalizer_message` 调用点 L8086-8116）

把 `assistant.message` 事件的 `payload` 增加 `do_not_persist_to_memory: true` 字段，并在 session_memory 写入端跳过带该标志的消息。

**预期效果**：避免下一轮 prompt 注入 "assistant: 本轮已完成..."，切断毒化链。

### 批 2：提升 dedup 消息精度（移植 Open-ClaudeCode 模式，1 小时）

#### 2.1 改写 dedup preview 文本

文件：`crates/runtime/src/native_agent_loop.rs:5747`

```rust
// 旧：
preview: format!(
    "duplicate observation skipped for {tool_id}; reuse prior evidence or inspect a narrower target"
),

// 新（移植自 Open-ClaudeCode FILE_UNCHANGED_STUB）：
preview: format!(
    "File unchanged since the earlier {tool_id} call in this conversation. \
     The previous tool_result is still current — refer to it instead of re-reading. \
     If you have all evidence needed to complete the user's task, proceed to write/edit \
     instead of further reads."
),
```

关键差异：
- 去掉 "inspect a narrower target"——这正是模型的歧路诱因
- 加上 "proceed to write/edit instead of further reads"——显式给出下一步动作

#### 2.2 增加 mtime 验证（参考 Open-ClaudeCode 的 `mtimeMs === existingState.timestamp`）

文件：`crates/runtime/src/agent_kernel/observation_cache.rs`

在 `ObservedFileReadRange` 结构里加 `mtime_ns: u64` 字段。`check_and_record_file_read_outcome` 命中前用 `std::fs::metadata().modified()` 对比。如果 mtime 变了就**不**作为 duplicate，强制重新读取。

**预期效果**：用户在外部编辑器修改了文件后，dedup 不会错误地继续阻挡。

#### 2.3 在 `next_action_hint` 里删除 "increase max_bytes" 之类的暗示

文件：`crates/runtime/src/native_agent_loop.rs:5750`

```rust
// 旧：
next_action_hint: "This observation was already returned or its file range is already covered. Keep tools available, but choose a genuinely new subdirectory/file/offset/search, increase max_bytes only if the missing suffix is essential, or produce the final answer from collected evidence."

// 新：
next_action_hint: "This file/path was already observed in this conversation. Do not call file.read on covered ranges again. If you have collected enough evidence for the user's task, switch to write/edit/patch tools. If you genuinely need new evidence, choose a path NOT present in the Evidence Ledger."
```

### 批 3：移植 `SyntheticOutputTool` 模式 —— 强制最终回答（4–6 小时）

当 plateau 即将触发或刚触发时，**不要直接降级到字符串 fallback**，而是给模型一个最后的、强制的"final_answer"工具。

#### 3.1 新增工具：`agent.final_answer`

文件位置建议：`crates/runtime/src/tools/final_answer_tool.rs`（新增）

```rust
pub const FINAL_ANSWER_TOOL_ID: &str = "agent.final_answer";

pub fn final_answer_tool_prompt() -> &'static str {
    "Use this tool to return your final response when you have completed the user's task \
     OR when you cannot make further progress with available tools. You MUST call this tool \
     exactly once with a `message` field containing the user-facing answer. Do not call \
     read-only tools after this."
}

pub struct FinalAnswerArgs {
    pub message: String,
    pub status: FinalAnswerStatus, // Completed | BlockedNeedsUser | PartialProgress
}
```

#### 3.2 触发条件 —— 渐进暴露

在 `ConvergenceEnforcer::observe_iteration`（`agent_kernel/convergence_enforcer.rs`）增加新状态 `ForceFinalAnswerExposure`：

```rust
pub enum ConvergenceVerdict {
    Continue,
    SoftWarning,                  // 新增：1 轮 duplicate dominance
    ForceFinalAnswerExposure,     // 新增：2 轮 duplicate dominance，下次只暴露 final_answer + write 工具
    BatchNoveltyPlateau,
    DuplicateDominance { ratio: f32, window: usize },  // 现状：直接终止
    // ...
}
```

策略：
1. 第 1 轮 100% duplicate → `SoftWarning`，下一次模型上下文里追加一条 system_reminder："You have read all required files. The next assistant turn should either call write/edit tools to implement the task, or call `agent.final_answer` to wrap up."
2. 第 2 轮仍 100% duplicate → `ForceFinalAnswerExposure`，调整 `tool_exposure`：只允许 `agent.final_answer` + write 工具（隐藏所有 read 工具）
3. 第 3 轮仍 duplicate → 直接合成 fallback（现状）

#### 3.3 在 `complete_native_loop_with_visible_finalizer_or_fallback` 前插入一次

文件：`crates/runtime/src/native_agent_loop.rs:8303`

```rust
// 在调用 try_visible_finalizer_from_evidence 之前，先尝试 force_final_answer：
if turn_state.progress.consecutive_duplicate_iterations >= 1 {
    if let Ok(true) = try_force_final_answer(transport, session, ...) {
        return Ok(());
    }
}
// 现状 try_visible_finalizer_from_evidence -> fallback
```

`try_force_final_answer` 与 `try_visible_finalizer_from_evidence` 区别：
- 它把工具集**收紧到只有 `agent.final_answer`**
- system prompt 明确要求："call agent.final_answer with a concrete next-step recommendation; the user is blocked"
- 模型几乎不可能返回空文本（因为唯一可调用的工具就是 final_answer）

### 批 4：路由器路由「继续/continue」类指令到 CodeEdit（2 小时）

文件：`crates/runtime/src/runtime_facade.rs:3612`

```rust
fn deepseek_runtime_tool_exposure_for_prompt(prompt: &str) -> NativeAgentToolExposure {
    let lowered = prompt.to_lowercase();
    
    // 「继续/继续按照/continue with/继续实现」+ 已有 plan/evidence → CodeEdit
    let is_continuation = ["继续", "continue", "接着", "下一步", "go on"]
        .iter()
        .any(|needle| lowered.contains(needle));
    let mentions_plan = ["plan", "实施计划", "实现", "implement"]
        .iter()
        .any(|needle| lowered.contains(needle));
    
    if is_continuation && mentions_plan {
        return NativeAgentToolExposure::CodeEdit;  // 假设此值偏向 write
    }
    NativeAgentToolExposure::FastAutoWrite
}
```

`CodeEdit` 暴露集应当：
- 优先暴露 `file.write`, `file.edit`, `patch.apply`, `shell.cmd`
- 把 `file.read`, `file.list_tree`, `search.ripgrep` 标记为"已读完毕，仅在必要时使用"
- 在 system prompt 里追加："The user has asked you to continue implementation. Evidence has already been gathered (see Evidence Ledger). Your default action is to call write/edit tools."

---

## 测试矩阵

| 场景 | 期望行为 | 验证位点 |
|---|---|---|
| 用户首轮 `继续按照plan`，evidence 为空 | 模型先读必要文件 → 写代码 | session events 中应有 `file.write` 调用 |
| 第二轮 `继续`，evidence 含 11 个文件 | 模型直接写代码，不再读已覆盖文件 | 0 个 `file.read`，≥1 个 `file.write` |
| 第二轮 `继续`，第一轮意外失败 | session_memory 不含 "本轮已完成..." | `record_native_loop_turn_summary` 前过滤 |
| Plateau 触发（1 轮 100% dup） | `SoftWarning` + system_reminder 注入 | runtime_events 有 `agent.soft_warning` |
| Plateau 触发（2 轮 100% dup） | 工具集收紧到 final_answer + write | 下一轮 model.request 的 tools_json 不含 `file.read` |
| Plateau 触发（3 轮 100% dup） | 模型调用 `agent.final_answer` | event 有 `agent.final_answer.called`，UI 看到真实回答 |
| 文件在编辑器中被修改后再 read | dedup 不应阻挡，重新读取 | `tool_result` 含新内容，不是 stub |

### 关键回归测试（应当先写）

`crates/runtime/src/native_agent_loop.rs:12000+` 测试文件新增：

```rust
#[test]
fn duplicate_dominance_plateau_invokes_force_final_answer_not_fallback_string() {
    // 模拟 2 轮全 duplicate file.read，验证：
    // 1. final_answer tool 被暴露
    // 2. 模型 mock 调用 final_answer，返回非空 message
    // 3. event_jsonl 不含 "本轮已完成"
    // 4. event_jsonl 含 "agent.final_answer.called"
}

#[test]
fn session_memory_excludes_plateau_fallback_assistant_messages() {
    // 跑一个 plateau-terminated turn，再开新 turn，验证：
    // user_task 不含 "assistant: 本轮已完成"
}

#[test]
fn continue_with_plan_routes_to_code_edit_exposure() {
    assert_eq!(
        deepseek_runtime_tool_exposure_for_prompt("继续按照plan"),
        NativeAgentToolExposure::CodeEdit
    );
}

#[test]
fn file_read_dedup_respects_mtime_change() {
    // 写文件 A，read，再修改 A 的 mtime，再 read：
    // 第二次不应返回 duplicate stub
}
```

---

## 落地顺序（按 ROI 排序）

```
批 1   ─────────►  30 min   ★★★★★  立即消除"本轮已完成..."字符串
  │
批 2   ─────────►  1 h      ★★★★    dedup 消息精度，把模型推向写
  │
批 4   ─────────►  2 h      ★★★★    路由器分流，从源头解决
  │
批 3   ─────────►  4-6 h    ★★★★★  根治：force final answer，永不留空答案
```

**最小可行修复**：批 1 + 批 2 即可让用户立刻看到「写代码」而不是「读循环」。批 3 + 批 4 是结构性解，建议同步完成。

---

## 引用

### Open-ClaudeCode 源码

- `src/tools/FileReadTool/prompt.ts:7` — `FILE_UNCHANGED_STUB` 文本
- `src/tools/FileReadTool/FileReadTool.ts:540-572` — mtime+range 双重校验的 dedup 实现
- `src/tools/SyntheticOutputTool/SyntheticOutputTool.ts:20-52` — 强制最终输出工具
- `src/query.ts:834` — `needsFollowUp` 决策点

### deep-code 当前实现

- `crates/runtime/src/native_agent_loop.rs:5732-5780` — `execute_duplicate_observation_collect`
- `crates/runtime/src/native_agent_loop.rs:8086-8178` — `synthesized_visible_finalizer_message`
- `crates/runtime/src/native_agent_loop.rs:8303-8380` — `complete_native_loop_with_visible_finalizer_or_fallback`
- `crates/runtime/src/native_agent_loop.rs:8382-8543` — `try_visible_finalizer_from_evidence`
- `crates/runtime/src/agent_kernel/observation_cache.rs:1-150` — 当前 dedup 与 `DedupeOutcome`
- `crates/runtime/src/agent_kernel/convergence_enforcer.rs` — `ConvergenceVerdict`
- `crates/runtime/src/agent_kernel/turn_state.rs:42-94` — `ToolProgressState`
- `crates/runtime/src/runtime_facade.rs:3612` — `deepseek_runtime_tool_exposure_for_prompt`

### 证据

- `/Users/gongyuxuan/Documents/deep-code/.researchcode/runtime_desktop/runtime_session_1779077180193148000/events/runtime_events.jsonl`
  - seq 803, 2460, 4229：三处 `model.stream_delta` 含 "本轮已完成..."
  - seq 4232：`agent.turn_summary` 显示毒化的 `user_task`
