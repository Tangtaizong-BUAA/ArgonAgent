# Phase 7 · TurnRouter + Compactor 真接入契约

> 状态：**Opus 设计已签**（2026-05-26）
> 上游：[`docs/architecture_consolidation_plan.md` §Phase 7](../architecture_consolidation_plan.md)
> 依赖：Phase 6.1.d（ContextService）完成
> 下游：Codex 按本契约执行 7.1-7.3

---

## 1. TurnRouter 现状（核验）

`agent_kernel/turn_router.rs` 82 行，**已能路由全部 8 个 TurnRoute variant**：
ProjectStatus / DirectAnswer / ReadOnlyExplore / CodeEdit / DebugFailure / RunTests / LongHorizonTask / Review。

**当前生产调用者：零**（grep `TurnRouter::classify` 在 `agent_kernel/turn_router` 之外无命中）。

旧 gap 分析里的"只能路由 3/8"已过时——routing 逻辑实现完整，**只缺接入**。

## 2. TurnRouter 接入：5 个 hook 点

### 2.1 调用点（必须新增）

在 `native_agent_loop_entrypoints.rs` 主入口（`run_native_agent_loop_v2_deepseek_inner`）的最前面：

```rust
let turn_route = agent_kernel::TurnRouter::classify(
    &request.user_prompt,
    request.previous_turn_summary.as_deref(),
    request.turn_index.unwrap_or(0),
);
session.append_event(KernelEvent {
    event_type: "turn.route.classified".into(),
    payload_json: json!({ "route": format!("{:?}", turn_route) }).to_string(),
    ...
});
```

这一段已经"几乎存在"（`native_agent_loop.rs:291` 已有类似 `format!` telemetry 代码），只是 route 结果没被消费。

### 2.2 manifest exposure 切片

`native_agent_loop_prompt.rs::build_native_loop_tool_manifest` 和 `native_loop_manifest_exposure` 当前签名：

```rust
pub(in crate::native_agent_loop) fn native_loop_manifest_exposure(
    prompt: &str,
) -> NativeAgentToolExposure;
```

改为：

```rust
pub(in crate::native_agent_loop) fn native_loop_manifest_exposure(
    prompt: &str,
    route: TurnRoute,
) -> NativeAgentToolExposure;
```

route 决定 exposure：

| TurnRoute | Manifest Exposure | 理由 |
|---|---|---|
| ProjectStatus | `ReadOnly` | 只查状态，不应写 |
| DirectAnswer | `ReadOnly`（甚至 `NoTools`） | 纯回答 |
| ReadOnlyExplore | `ReadOnly` | 显式 |
| CodeEdit | `FastAutoWrite` | 写编辑工具 |
| DebugFailure | `FullToolset` | 需要 shell/edit/read |
| RunTests | `FullToolset` | 需要 shell |
| LongHorizonTask | `FullToolset` | 不限 |
| Review | `ReadOnly` | code review 通常只读 |

### 2.3 删除 25 个中文关键词分支

`prompt.rs` 内的关键词逻辑（旧 `deepseek_runtime_tool_exposure_for_prompt` 25 关键词）当前已在 §5.D `prompt` sibling 标记为"私有内部"等待 Phase 7 替换。

删除目标：
- `native_prompt_wants_file_generation`
- `native_prompt_is_long_running`
- `native_prompt_wants_write_or_edit`
- `native_prompt_wants_tool_inventory`
- `native_loop_write_directive_for_prompt`
- 等所有 `native_prompt_wants_*` 系列 fn

替换为 `match route { TurnRoute::CodeEdit => ..., ... }` 模式。

### 2.4 测试新增

每个 TurnRoute → Exposure 映射至少 1 个 unit test（共 8 个）。

### 2.5 不改变 fast_auto_write 等次级行为

`should_finalize_fast_auto_write` 等 fn 不改 ——它们看 `tool_batch` 状态而非 prompt 文本，与 router 正交。

## 3. Compactor 真接入（**2026-05-26 修订**）

### 3.0 修订背景

原契约要求 `Compactor::compact` 接收 `&mut EventLog` 并真删旧事件。Codex 在 7.2.a 启动前正确触发"必须停下来"，核验后发现现状是 **projection-only 设计**：

- `Compactor::compact(&self, event_log: &EventLog, ...) -> CompactionResult`
- 返回 `CompactionResult { projection, summary, token_estimate_before/after, marker }`
- EventLog 保持不可变（event sourcing 原则）

**裁决**：保持 projection-only 设计。**Mutate EventLog 是错误方向**：
- 违反 event sourcing（EventLog 应是不可变真相源）
- 失去 replay / audit / 时间倒带能力
- 与 doc39 Phase 3 ConversationHistory（依赖完整 EventLog 回放）冲突
- 与 Phase 6 ContextService 设计冲突

我（Opus）写原契约时把"compact"误解为"删事件"，沿用 Phase 8（sidecar vs reqwest 真两套实现）的 mindset，错了。

### 3.1 现状（修订后）

| 组件 | 状态 |
|---|---|
| `Compactor::compact` projection-only 设计 | ✅ 已实现 |
| `context.compaction.started/projected/completed/blocked` 事件 | ✅ 已在 `native_turn_controller.rs` 发出 |
| `CompactionResult.projection` 包含 summary_text + preserved_messages | ✅ 已构造 |
| `token_estimate_before/after` 字段 | ✅ 已有 |
| **真正 gap：projection 注入到下一次模型请求** | ❌ **未确认** — Codex 7.2.a 必须核验 |
| `context.compaction.skipped` 事件（阈值未达） | ❌ 缺失 |

### 3.2 真正的接入 gap：projection 必须进 prompt

当前 `compaction.summary` 被算出后塞进 `context.compaction.completed` 事件 payload 字符串里——**但下一次模型请求的 prompt 是否真用了 projection.summary_text 替换原 raw history？**

如果只是"事件 payload 记录了 summary"而模型请求 prompt 不变 → 这是真的 fake compaction（token 没下降）。

Phase 7.2.a 修订任务（见 §4 PR 拆解）：核验 projection 注入路径。

### 3.3 4 个事件命名规范（修订）

| 事件 | 何时发 | 当前状态 |
|---|---|---|
| `context.compaction.started` | should_compact 返回 true，决定走 compact 前 | ✅ 已有 |
| `context.compaction.projected` | projection 已构造（compactor.compact 返回） | ✅ 已有，**保留**（与 §6 telemetry 一致） |
| `context.compaction.completed` | projection 已注入到下一次模型请求 prompt 后 | ⚠️ 当前发于"算完 projection"，应改为"注入 prompt 后" |
| `context.compaction.blocked` | tokens 仍超过 hard limit（projection 注入后），实际不发请求 | ✅ 已有 |
| `context.compaction.skipped` | should_compact 返回 false，阈值未达 | ❌ 缺失，**7.2.b 加** |

注意：保留 `projected` 事件（替代原契约里建议的删除），它是 projection 已生成但还未注入 prompt 的真实信号，对调试有价值。

### 3.4 接入 `ReasoningReplayManager::compact_old_reasoning`

`compact_old_reasoning` 是 mutable state（`ReasoningReplayManager` 是 stateful struct）——可以真 mutate，与 EventLog projection 设计正交。

`Compactor::compact` 后调用：

```rust
if plan.preserve_latest_reasoning {
    profile.reasoning_replay_manager_mut().compact_old_reasoning(plan.preserve_count);
}
```

这一步**不影响 EventLog**，只影响 profile 内部的 reasoning 缓存。

## 4. PR 拆解

| 顺序 | PR | 风险 |
|---|---|---|
| 7.1.a | TurnRouter 接入 entrypoints + 加 `turn.route.classified` 事件 | 低 |
| 7.1.b | `native_loop_manifest_exposure` 改签名带 route | 中 |
| 7.1.c | 删除 25 个关键词 fn + 8 个 route→exposure unit test | 中 |
| 7.2.a | **核验现状** projection 注入路径（不动代码），输出 diff 报告 | 低 |
| 7.2.b | `native_agent_loop_model_io` 接入 compact 调用 | 高 |
| 7.2.c | 清理假事件 + 加 `skipped` variant + 7 天 metric watch | 中 |
| 7.3 | 接入 ReasoningReplayManager.compact_old_reasoning | 中 |

## 5. 验收

### 5.1 TurnRouter

| 指标 | 目标 |
|---|---|
| `turn.route.classified` 事件覆盖率 | 100%（每个 turn 1 个） |
| route → exposure 映射 unit test | 8 个全绿 |
| `native_prompt_wants_*` fn 残留数 | 0 |
| Eval Gate 通过率 | 不下降 |

### 5.2 Compactor（修订）

| 指标 | 目标 | 度量 |
|---|---|---|
| **projection 注入后下一次模型请求 prompt tokens 下降** | ≥ 30%（200K 长 session 用例） | compare `model_call.prompt_tokens` before/after projection injection |
| `context.compaction.completed` 事件数 ≡ projection 注入调用数 | 1:1 | grep + 计数 |
| `context.compaction.projected` 事件数 ≡ `Compactor::compact()` 调用数 | 1:1 | 同上 |
| `context.compaction.skipped` 事件出现（阈值未触发） | ≥ 1 in any normal turn | grep |
| `preserved_reasoning_count` 字段值 | > 0（验证 ReasoningReplayManager 接入） | event payload |
| 长 session smoke test（≥ 50K tokens） | 7 天 P50/P95 延迟稳定（不因 compact 卡顿） | metric |
| **EventLog 总 tokens 在 compact 前后**（不再要求下降） | 仅追加事件，单调递增（event sourcing 正常） | 反证 mutate 行为 |

## 6. 不在 Phase 7 范围

- 不动 TurnRouter 的关键词列表（routing 逻辑暂保持现状）
- 不引入 LLM-based routing（route classify 仍是规则式，未来 epic）
- 不动 Compactor 的 plan 默认值（`deepseek_default_plan` 保持）
- 不接入 GPT-based summarization（compactor 仍走当前实现）

## 7. 必须停下来报告

1. `Compactor::compact` 当前实际不 mutate EventLog（只生成 summary）—— Codex 必须在 7.2.a 之前停下来报告，等 Opus 给扩展契约
2. route → exposure 映射在某个真实 fixture 上导致权限错误（如 CodeEdit 路由但用户只想 read）
3. 删除关键词后某测试期待"用户说'写'就触发 FastAutoWrite"——评估是测试过时还是 mapping 缺漏
4. `preserve_latest_reasoning` 触发后下一个 turn 的 reasoning_content 缺失（说明 compact 边界错）
