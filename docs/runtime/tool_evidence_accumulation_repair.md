# 工具证据累计回灌导致的无进展循环 — 深度剖析与架构升级

**Scope:** `crates/runtime/src/native_agent_loop.rs` + `crates/runtime/src/agent_kernel/{turn_state.rs, observation_cache.rs}`
**Evidence:** `/Users/gongyuxuan/Documents/deep-code/.researchcode/runtime_desktop/runtime_session_1779068926606284000/events/runtime_events.jsonl` (42 107 events, 23 MB, 8 user-visible turns)
**Status of partial fixes in `main`:**
- 部分修复已存在：`last_tool_batch.clear()` at [`native_agent_loop.rs:3463`](crates/runtime/src/native_agent_loop.rs:3463) + plateau finalize at [`native_agent_loop.rs:4325`](crates/runtime/src/native_agent_loop.rs:4325)。
- 证据日志中 plateau finalize 事件 0 次、累计仍发生 → 修复**结构上不够**。

---

## 0. 一句话定性

> Runtime 把 “上一轮工具结果” 当成可附加的 evidence stream（push-only），而不是一个**每轮重新计算**的 derived snapshot；多个旁路（streaming success、loop-guard recovery、`plan_read_budget_exhausted` 重盐 key、dedupe 仍 append、plateau 在 push 之后才裁决）共同导致即便有 dedupe 信号也无法收束，模型不断收到“更长但语义未变”的上下文，进入读—列—搜的原地循环。

---

## 1. 事实证据（从日志反推）

### 1.1 事件维度

| event_type                              | 计数  | 备注 |
|----------------------------------------|------|------|
| `tool.call_requested`                  | 192  | 8 个 user turn 共 73 个 model 迭代里发出 |
| `tool.call_completed`                  | 192  | |
| `tool.result_recorded`                 | 189  | |
| `tool.duplicate_observation_suppressed`| 25   | **只有 13 % 的工具调用被识别为重复** |
| `model.call_started`                   | 74   | 平均 9 model call/turn |
| `model.continuation_strategy`          | 66   | provider_tool_result_continuation 占 100 % |
| `agent.loop_recovery`                  | 19   | 多次声明 "continue_with_synthetic_tool_result_without_disabling_tools" 但**未真正收束** |
| `agent.tool.streaming_batch_ready`     | 4    | streaming dedupe 命中 4 次（其余 70+ 走非流式后处理） |
| `agent.loop_plateau_finalized`         | **0** | plateau 检测代码存在，**实际未触发一次** |

### 1.2 累计的"硬证据" — `loop_2451` 跟踪

| 迭代 | 本轮新增 tool | continuation `tool_results` | 请求 token | 说明 |
|------|--------------|------------------------------|-----------|------|
| 1    | 2            | —                            | 6 511     | initial |
| 2    | 3            | **2**                        | 5 902     | |
| 3    | 3            | **5**                        | 6 537     | +3 |
| 4    | 1 (含 1 dedupe) | **8**                     | 10 361    | +3 |
| 5    | 2            | **9**                        | 11 157    | +1（dedupe 自身也被 append） |
| 6    | 1 (dedupe)   | **11**                       | 15 243    | +2 |
| 7    | 1 (dedupe)   | **12**                       | 15 744    | +1 |
| 8    | 3            | **13**                       | 16 282    | +1 |
| 9    | 3            | **16**                       | 18 476    | +3 |
| 10   | 4 (含 2 dedupe) | **19**                    | 23 352    | +3 |
| 11   | 1            | **23**                       | 26 508    | +4 |
| 12   | 1 (todo)     | **24**                       | 30 930    | +1 |

**结论：** `tool_results` 在单个 turn 内严格单调递增（2 → 24），请求 token 从 5.9 K 涨到 30.9 K，约 5× 增长，与 “runtime 反复回灌历史工具结果” 完全吻合。该 turn 内 dedupe 命中过 5 次但**累计仍未停止**。

### 1.3 dedupe 失效模式：cache key 自我加盐

dedupe 在 [`observation_cache.rs:55-66`](crates/runtime/src/agent_kernel/observation_cache.rs:55) 的 plan-like 路径做了一个反模式：

```rust
if plan_like_path(&path) {
    let attempts = self.file_read_attempts.get(&path).copied().unwrap_or(0);
    if attempts >= 4 {
        self.file_read_attempts.insert(path.clone(), attempts.saturating_add(1));
        self.seen.insert(key, 1);
        return Some(format!(
            "file.read:{path}:plan_read_budget_exhausted:attempts={}",
            attempts + 1
        ));
    }
}
```

日志中可见 `attempts=5` 与 `attempts=6` 形成**不同的 cache key**。每次 dedupe 调用都生成新的 key 字符串（虽然底层不再 re-execute 文件），但是返回给模型的 evidence 是**新的** “duplicate_observation” 记录而非已有那条 — 模型看到的是 “第 5 次尝试被截断 / 第 6 次尝试被截断”，会自然解读成 “系统在变化，再试一次会不会有结果？”。**dedupe 信号被它自己污染了。**

### 1.4 关键不变量被破坏

| 应当成立的不变量                                            | 现实 |
|-----------------------------------------------------------|------|
| continuation 请求只包含**本轮**工具结果                    | 单调累计 |
| dedupe → 不再向 model 暴露该工具的执行结果                  | dedupe 结果照样 append 到 `last_tool_batch` |
| dedupe N 次 → 强制 finalize / route 切换                    | 仅记录 event，循环继续 |
| `seen_tool_batches` 命中 → terminate                       | 仅 `loop_guard_recovery_count++`，>2 后继续 |
| `ToolProgressState.record_iteration` → `Finalize` 时真的 finalize | 日志中 0 次触发（因为每轮总有 ≥1 个 "new evidence" 让 `consecutive_*` 重置） |

---

## 2. 代码层根因

### 2.1 `last_tool_batch` 的语义混乱（**核心结构性 bug**）

`last_tool_batch` 是 [`native_agent_loop.rs:1547+`](crates/runtime/src/native_agent_loop.rs:1547) 声明的 `Vec<(provider_tool_call_id, tool_id, args_json, ToolExecutionResult)>`，它在整个外层 `for iteration in 0..max_iterations` **之外**声明，一直存活。

每次模型 continuation 请求的构造（[`native_agent_loop.rs:1632`](crates/runtime/src/native_agent_loop.rs:1632)）都是 `continuation_batch = last_tool_batch.clone()`。

**重置点（应使 `last_tool_batch` 只代表 "本轮" 结果）：**
- L3168 `last_tool_batch = streamed_tool_batch;`（streaming 成功路径）
- L3409 `last_tool_batch = tool_calls.iter()...collect()`（loop-guard recovery 合成 synthetic error）
- L3463 `last_tool_batch.clear();`（非流式正常路径 — 这是 main 已有的部分修复）

**追加点（在重置之外仍向 `last_tool_batch` push 的位置）：**
- L2122 transport-error fast-auto-write recovery
- L2356 empty-visible fast-auto-write fallback
- L2603 concurrent batch 结果
- L2720 shell→directory alias recovery
- L2772 tool-not-in-manifest recovery
- L2810 tool contract rejected recovery
- L3035 fast-auto-write 正常路径
- L3134 shell permission-required recovery
- L3175 read-only 正常路径
- L3273 final fallback
- L4142+ `execute_duplicate_observation_collect` 返回的 result **依然被 push**

问题：
1. **streaming 成功路径**（L3168）`last_tool_batch = streamed_tool_batch`，**不经过 L3463 的 clear**。下一轮迭代若 streaming 又成功，OK；但若 streaming 又触发任何 recovery push（L2122 等），它们会 **append 到上一轮 streamed batch 之上**。
2. **L3463 的 clear 时点**：发生在每轮 dispatch **之前**、batch_signature 检查之后。一旦本轮 dispatch 内任何 push 发生，本轮 batch 形成；但下一轮进入 streaming 成功路径，重置走的是 L3168（赋值给 streamed），又**绕过**了清理逻辑（其实是直接 replace，并不会累计 — 但 streaming dedupe 命中率只有 5 %，主要靠 L3463）。
3. **dedupe 返回值仍 push 到 last_tool_batch**：[`native_agent_loop.rs:4142-4170`](crates/runtime/src/native_agent_loop.rs:4142) `execute_duplicate_observation_collect` 返回一个 `ToolExecutionResult` 标 `skipped:true,reason:duplicate_observation,next_action_hint:...`，调用方（L3045-3057 等）将该结果一并 push 到 last_tool_batch。这意味着 “dedupe 命中” 在协议层没有抑制作用 —— 反而占了一个 evidence slot，把 hint 喂回模型，让模型 “换个角度再读一次”。

### 2.2 ToolProgressState 决策点位错误

[`turn_state.rs:51-93`](crates/runtime/src/agent_kernel/turn_state.rs:51) 的 `record_iteration` 设计正确：连续 ≥ 2 次 duplicate-only iteration → `duplicate_tool_observation_plateau`、连续 ≥ 3 次 no-progress → `non_progress_tool_plateau`。

但 [`native_agent_loop.rs:4313-4324`](crates/runtime/src/native_agent_loop.rs:4313) 这样统计：

```rust
let progress_new_evidence_results = last_tool_batch
    .iter()
    .filter(|(_, _, _, result)| result.ok && !is_duplicate_observation_result(result))
    .count() as u32;
```

**问题：**
- 只要本轮 push 了任何一个 `result.ok == true && !is_duplicate_observation` 的 tool result，`new_evidence_results > 0` 就成立。
- 模型只要在 8 个调用里塞**一个新文件读取**（哪怕只读了 1 KB），plateau 计数器即被清零。
- 日志证据：loop_2451 dedupe 命中 5 次但 plateau 从未触发，因为同 batch 内总有 ≥ 1 个 file.read 命中新路径。

`ToolProgressState` 的统计**只看本轮 push 的结果总数**，没有看 “本轮工具调用相对于上一轮是否带来语义新增”。这是把 "是否有 OK 结果" 当成 "是否有进展" 的混淆。

### 2.3 `seen_tool_batches` 的对抗性弱

[`native_agent_loop.rs:3345-3372`](crates/runtime/src/native_agent_loop.rs:3345) 用 `stable_text_hash` 对 batch 内 `tool_id + arguments_json` 完整序列做哈希，**精确匹配**。模型只要：
- 改 `max_bytes` (8000 → 8192)
- 调整 `offset` 或 `limit`
- 添加无意义字段（trailing whitespace、字段顺序、`{ "path": "X" }` vs `{ "path": "X", "max_bytes": 8000 }`）
- 在 batch 里换序

即可逃避。alternating-pattern 检测只覆盖深度 2 与 4，5+ 步循环不抓。

### 2.4 loop_guard 软退让

[`native_agent_loop.rs:3395-3408`](crates/runtime/src/native_agent_loop.rs:3395)：

```rust
if loop_guard_recovery_count > max_loop_guard_recoveries { // > 2
    session.record_runtime_event(
        "agent.loop_recovery",
        ...,
        format!("{{...\"action\":\"continue_with_synthetic_tool_result_without_disabling_tools\"}}"),
    )?;
}
```

`max_loop_guard_recoveries = 2`，超过时**只发事件不退出**。`last_tool_batch = tool_calls.iter()...collect()` 走合成 synthetic error，然后 `continue`。模型下轮看到一批 “该 tool 已被合成错误标记，请改用其他工具” 的 hint —— 但工具菜单未变 —— 它可能换个参数再走一遍，再次进入合成错误。

### 2.5 continuation 永远 full-text replay

`build_native_tool_result_continuation_request` / `build_native_tool_evidence_continuation_request` 把所有 `last_tool_batch` 条目按原始 detail_json/preview 序列化进消息。没有：
- evidence salience scoring（保留新 evidence、drop 已被 supersede 的）
- summarization step（n>k 时 LLM-based summary 或 deterministic compaction）
- range-merge（同一文件多次 partial read → 合并为一段）

`compaction_threshold_tokens = 192 000` 实在太松（日志中最高 30 K，根本到不了）。

### 2.6 streaming size_mismatch 静默接受

`agent.tool.streaming_batch_ready` 4 个事件全部是 `continue_with_streamed_results_size_mismatch`（streamed_count=1 vs parsed_count=2-3）。表面上 “use streamed batch to avoid double execution”，实际是放过了一个语义不一致：模型 visible_content_preview 写了 3 个 tool_calls，runtime 只 stream-execute 了 1 个，剩下 2 个被丢弃。下一轮模型不会知道 “那两个被丢了”，仍会复述自己的计划，对话变得混乱。

---

## 3. 架构升级方案（分层）

> 设计目标：把 evidence 从 **append-only 流** 改成 **每轮重新计算的不可变 snapshot**，并在 dedupe 之上加 **强制收束** 与 **evidence 退化** 两层。

```
┌─────────────────────────────────────────────────────────────────┐
│  Layer 4: Turn-level outcome (existing)                         │
│  - completion status, transcript, ledger summary                │
└─────────────────────────────────────────────────────────────────┘
                              ▲
┌─────────────────────────────────────────────────────────────────┐
│  Layer 3: Convergence Enforcer  ◄── NEW                         │
│  - Hard plateau: dedupe-ratio, no-new-information, batch-novelty│
│  - Forced finalize, never just "log and continue"               │
└─────────────────────────────────────────────────────────────────┘
                              ▲
┌─────────────────────────────────────────────────────────────────┐
│  Layer 2: Evidence Ledger     ◄── NEW (replaces last_tool_batch)│
│  - Immutable per-iteration snapshot                             │
│  - Range-aware, supersede-aware                                 │
│  - Provides views: "this turn", "this iter", "summary for LLM"  │
└─────────────────────────────────────────────────────────────────┘
                              ▲
┌─────────────────────────────────────────────────────────────────┐
│  Layer 1: ObservationCache (existing, hardened)                 │
│  - Stable cache keys (no attempts= salt)                        │
│  - Coverage-aware (existing)                                    │
│  - Returns SuppressedResult that downstream MUST drop, not push │
└─────────────────────────────────────────────────────────────────┘
                              ▲
┌─────────────────────────────────────────────────────────────────┐
│  Layer 0: Tool dispatch (existing)                              │
└─────────────────────────────────────────────────────────────────┘
```

### 3.1 Layer 1 修复：让 dedupe 真的 dedupe

**问题：** dedupe 命中后仍 push 到 last_tool_batch；cache key 自我加盐。

**改动：**

a) [`observation_cache.rs:52-84`](crates/runtime/src/agent_kernel/observation_cache.rs:52) — 移除 `plan_read_budget_exhausted:attempts=N` 的递增后缀，改为稳定 key：

```rust
fn check_and_record_file_read(&mut self, arguments: &ParsedToolArguments) -> Option<String> {
    let key = observation_key("file.read", arguments)?;
    let path = normalized_observation_path(arguments)?;

    if let Some(count) = self.seen.get_mut(&key) {
        *count += 1;
        return Some(key);  // 稳定返回原 key，不带 attempts
    }
    if let Some(covered_by) = self.covering_file_read_key(arguments) {
        self.seen.insert(key, 1);
        return Some(covered_by);  // 稳定 covered_by key
    }
    // plan-like budget 用独立的 metric 跟踪，但不污染 cache key
    if plan_like_path(&path) {
        let attempts = *self.file_read_attempts.entry(path.clone()).or_insert(0);
        if attempts >= 4 {
            self.seen.insert(key.clone(), 1);
            return Some(key);  // 标准 key + 走 layer-3 plateau enforcement
        }
    }
    // 首次记录
    self.seen.insert(key.clone(), 1);
    if let Some(range) = ObservedFileReadRange::from_arguments(arguments, key) {
        self.file_read_ranges.entry(range.path.clone()).or_default().push(range);
    }
    *self.file_read_attempts.entry(path).or_insert(0) += 1;
    None
}
```

b) 引入 `DedupeOutcome` 枚举替代 `Option<String>`：

```rust
pub enum DedupeOutcome {
    FirstObservation,
    DuplicateExactKey { key: String, prior_seen_count: u32 },
    DuplicateCoveredBy { key: String, covering_key: String },
    DuplicateRateLimited { key: String, attempts: u32 },  // plan-like budget
}
```

下游据此决定 **是否 push、push 什么、是否触发 plateau**。

### 3.2 Layer 2 修复：把 last_tool_batch 升级为 EvidenceLedger

**问题：** `last_tool_batch` 是 push-only Vec，没有 "上一轮 vs 本轮" 边界，多个旁路无序追加。

**改动：** 在 `agent_kernel/` 新增 `evidence_ledger.rs`：

```rust
/// Per-turn evidence accumulator.  Every iteration produces an
/// `IterationEvidence` snapshot; the ledger keeps the *last* iteration's
/// snapshot as the canonical "what to send back to the model" view, plus
/// a compacted history for in-turn cross-iter reasoning.
pub struct EvidenceLedger {
    iterations: Vec<IterationEvidence>,
    suppressed: Vec<SuppressedEvidence>,  // dedupe results never sent to model
    range_index: BTreeMap<String, Vec<ObservedFileReadRange>>,
}

pub struct IterationEvidence {
    iter_index: u32,
    started_at: Instant,
    items: Vec<EvidenceItem>,        // ONLY this iteration's new results
    novelty_score: NoveltyScore,     // computed at finalize time
}

pub struct EvidenceItem {
    provider_tool_call_id: String,
    tool_id: String,
    arguments_json: String,
    result: ToolExecutionResult,
    classification: EvidenceClass,   // NewEvidence | Recovery | Error | Suppressed
}

impl EvidenceLedger {
    /// Open a new iteration window.  The previous iteration is sealed
    /// and becomes immutable.
    pub fn begin_iteration(&mut self, iter_index: u32) -> IterationHandle { ... }

    /// Record a tool result.  Suppressed results are stored in `suppressed`
    /// and DO NOT appear in `view_for_continuation`.
    pub fn record(&mut self, h: &IterationHandle, item: EvidenceItem) { ... }

    /// The slice the model sees on its next continuation: ONLY the most
    /// recent sealed iteration's items, plus a deterministic 1-2-line
    /// summary of older iterations ("you previously read X (4k bytes), Y").
    pub fn view_for_continuation(&self) -> ContinuationView { ... }

    /// True if the last K iterations have produced no NewEvidence-classified
    /// items.  Used by Layer 3 enforcer.
    pub fn no_new_evidence_streak(&self) -> u32 { ... }

    pub fn duplicate_streak(&self) -> u32 { ... }
}
```

调用点替换（搜 `last_tool_batch.push` 全部改为 `ledger.record(&h, ...)`；搜 `last_tool_batch = ` 全部删除；continuation 构造从 `ledger.view_for_continuation()` 取）。

**关键不变量：**
- `view_for_continuation()` 返回的 `tool_results` 数量在单 turn 内**不应随 iteration 单调增长**。
- 所有 dedupe outcome 走 `suppressed`，**绝不进入 view**。它们在 `agent.tool.duplicate_observation_suppressed` event 中可观测，但不污染 continuation。

### 3.3 Layer 3 新增：Convergence Enforcer

`ToolProgressState` 的位置正确但**判据错**。改造：

```rust
pub enum ConvergenceVerdict {
    Continue,
    // 模型连续 N 次发出与上一轮 95% 重合的 tool batch
    BatchNoveltyPlateau { novelty_ratio: f32, threshold: f32 },
    // 工具结果有 ≥ 70% 是 suppressed/duplicate
    DuplicateDominance { ratio: f32, window: u32 },
    // tools used 都返回 OK 但 evidence ledger 的 distinct-key 数量没增长
    InformationStagnation { distinct_keys_growth: u32, window: u32 },
    // hard ceiling (existing)
    BudgetExhausted,
}

impl ConvergenceEnforcer {
    pub fn observe_iteration(
        &mut self,
        ledger: &EvidenceLedger,
        current_batch_signature: &str,
        recent_signatures: &[String],
    ) -> ConvergenceVerdict { ... }
}
```

调用时机：每轮 dispatch 结束、构造下一轮 continuation **之前**。Verdict 非 `Continue` 时**必须**调用 `complete_native_loop_with_visible_fallback`，**禁止** "记录 event 后 continue"。

### 3.4 Layer 4 修复：continuation 内容退化

`build_native_tool_result_continuation_request` 改为接受 `ContinuationView` 而非裸 Vec：

```rust
pub struct ContinuationView {
    /// 本轮（最近一个 sealed iteration）的完整 tool results — full-text
    pub current_iteration_items: Vec<EvidenceItem>,
    /// 历史摘要：之前迭代浓缩成 1-2 行 / 条
    pub history_digest: Vec<HistoryDigestEntry>,
    /// supersession 信息：哪些历史 read 已被本轮更大范围覆盖
    pub superseded: Vec<SupersededEntry>,
}
```

- `current_iteration_items.len()` 在单 turn 内**不再单调增长**（与 evidence ledger 配合）。
- `history_digest` 由 deterministic compactor 生成，例如：
  - `"prior iters 1-3: read plan/VoiceNote-AI-实施计划.md (8000B), read plan/技术实现要点.md (8000B); 5 dedupes suppressed"`。
- 当 prompt token 超过 30 K，强制走 LLM-based summarization（已有 `compaction.rs`，把阈值从 192 K 降到 24 K + per-turn）。

### 3.5 Layer 5 加固：seen_tool_batches 抗对抗

`stable_text_hash(tool_id + arguments_json)` 改为 `tool_id + canonicalized_arguments`，其中 canonicalize 流程：
1. 用 `ObservationCache::observation_key(tool_id, args)` 生成稳定语义 key。
2. 若 None（无法语义化的工具），fallback 到 JSON 字段排序后再 hash。
3. 对 `file.read` 把 `offset/limit/max_bytes` clamp 到与 ObservationCache 相同的桶。

这样 "改 max_bytes 8000→8192" 不再绕过检测。

alternating-pattern 检测扩展为 sliding window：

```rust
fn batch_in_recent_window(sig: &str, history: &[String], window: usize) -> bool {
    history.iter().rev().take(window).any(|h| h == sig)
}
```

window = 6（默认）。

---

## 4. 阶段性实施计划（5 个 PR）

### Phase A — 立刻止血 (1 个 PR, ~200 LOC)

**目标：让 dedupe 真的 dedupe；让 plateau 真的 finalize。**

1. `observation_cache.rs`：移除 `attempts=N` salt（3.1 a/b 子集）。
2. `native_agent_loop.rs:3045-3057, 4142+`：dedupe outcome **不再 push 到 `last_tool_batch`**。改为：
   - 仍 record `tool.duplicate_observation_suppressed` event。
   - 给 model continuation 一个 **单条** "you have observed N duplicate fetches this turn" 摘要（而不是 N 条 hint 各占一个 tool_result 槽）。
3. `native_agent_loop.rs:4313-4324`：`progress_new_evidence_results` 的判据从 "本轮有 ≥1 个 OK 非 dup result" 改为 "本轮在 ObservationCache 中新增了 distinct key"。新增需要 `EvidenceLedger` 也行，但 phase A 用一个简单的 `HashSet<String> evidence_distinct_keys` 就够。

**验证：** 重跑 Argon-Agent-test/VoiceNote-AI 用例，断言 `tool_results` 在单 turn 内**不再单调增长**、`agent.loop_plateau_finalized` 应当至少触发 1 次。

### Phase B — EvidenceLedger 落地 (1 个 PR, ~500 LOC)

3.2 全部。把 `last_tool_batch` 从 `Vec<(...)>` 改为 `EvidenceLedger`，所有 push/clear 调用点切到 ledger API。保留 `last_tool_batch` 作为 deprecated alias 暂时不删，给 doc39 其他切片缓冲一周。

### Phase C — ConvergenceEnforcer (1 个 PR, ~300 LOC)

3.3 全部。把 L3395 的 “log and continue” 替换为 enforcer.observe → verdict → finalize。

### Phase D — Continuation View (1 个 PR, ~400 LOC)

3.4 全部。`build_native_tool_*_continuation_request` 接受 `ContinuationView`。把 `compaction_threshold_tokens` 默认值从 192 K 降到 32 K（per turn 累计 evidence 上限）。

### Phase E — Anti-adversarial dedupe (1 个 PR, ~150 LOC)

3.5 全部。

---

## 5. 测试矩阵

| 测试名                                              | 设计                                                                      | 期望 |
|----------------------------------------------------|-------------------------------------------------------------------------|------|
| `tool_results_count_does_not_grow_within_turn`     | run loop with 5 iterations of distinct file.read, assert continuation `tool_results == per_iter_count` | pass |
| `dedupe_result_does_not_enter_continuation`         | request same file twice in two iterations, assert iter-2 continuation has 0 duplicate entries | pass |
| `plateau_fires_on_two_consecutive_dedupe_dominated_iters` | mock 2 iters where ≥70 % results are suppressed, assert `agent.loop_plateau_finalized` event with `duplicate_dominance` reason | pass |
| `batch_signature_robust_to_max_bytes_perturbation`  | iter-1: file.read X max_bytes=8000; iter-2: file.read X max_bytes=8192; assert detected as repeated | pass |
| `attempts_n_no_longer_in_cache_keys`                | replay the Argon-Agent-test/VoiceNote prompt, scan event log; assert no `plan_read_budget_exhausted:attempts=` strings | pass |
| `history_digest_keeps_token_budget`                 | 12-iter turn, assert request_tokens stays < 12 K | pass |

回归基线：用 `runtime_session_1779068926606284000` 作为 **negative regression fixture**（"如果未来日志再出现单调 tool_results 增长，CI 必须 fail"）。可以写一个 jsonl 解析脚本断言不变量。

---

## 6. 与 doc39 north-star 的对齐

- doc39 强调 "DeepSeek-first agent kernel"，本修复不破坏 DeepSeek 推理重放 / cache zone 任何机制。
- `EvidenceLedger` 是 agent_kernel 的自然成员，符合 doc39 的 “turn_state / observation_cache / tool_inventory” 切片分层。
- `ConvergenceEnforcer` 与现有 `ToolProgressState` 形成 enforcement-vs-telemetry 的清晰分工（state 持续统计，enforcer 持续裁决）。
- 现有 `agent.loop_plateau_finalized` event 类型可复用，不需新 schema。

---

## 7. 已经在 main 中的部分修复 — 为何不够

| 已存在的代码             | 为何不能单独解决问题                       |
|-------------------------|------------------------------------------|
| L3463 `last_tool_batch.clear()` | 只覆盖非流式路径。流式路径 (L3168) 与所有 recovery push 仍可旁路。 |
| L4325 plateau finalize | `progress_new_evidence_results` 判据太宽（任何 OK result 都算 evidence），日志显示 0 次触发。 |
| L3395 loop_guard recovery | `> max_recoveries` 时只发 event 不退出。 |
| `tool.duplicate_observation_suppressed` 事件 | 信息性的，下游仍 push 到 batch — 表面 "已检测"，实际 "未抑制"。 |

把这些理解清楚后，**Phase A 的 200 LOC 就能解决 80 % 的肉眼可见症状**；Phase B-E 把架构层面的累计漏洞封死。

---

## 8. 决策点（需要 owner 确认）

1. **EvidenceLedger 落在 `agent_kernel/` 还是 `runtime/` 顶层？** 建议 `agent_kernel/`，与 `observation_cache`、`turn_state` 同级。
2. **`compaction_threshold_tokens` 默认值降到多少？** 提案 32 K（current 192 K 显然过松）。可作可配置项保留 192 K 作 hard ceiling。
3. **history_digest 是 deterministic 还是 LLM-based？** 提案 deterministic 优先（L1），LLM compaction 仅在 > 32 K 时介入（L2）。
4. **回归 fixture 放哪里？** 提案 `eval/fixtures/no_progress_loop_1779068926606284000.jsonl`，附 invariant assertion 脚本。

---

## 附：触发本 Bug 的最小复现 prompt 形态

从日志看，本案例的用户 prompt 涉及一个外部目录 `/Users/.../Argon-Agent-test/plan/`，包含若干 markdown 文件（`VoiceNote-AI-实施计划.md`、`技术实现要点.md` 等）。模型被反复指引去阅读它们 + 列目录。最小复现：

```
prompt: 请通读 ${ABS_PATH}/plan/ 下所有 markdown，分析其中提到的技术栈，输出一份汇总。
manifest tools: file.list_directory, file.list_tree, file.read, search.ripgrep, todo.write
max_iterations: 16+, max_tool_calls: 200
```

每轮模型会列目录 → 读文件 → 再列 → 再读，dedupe 命中后看到 `next_action_hint` 又换一种参数读，永远不收束。

---
