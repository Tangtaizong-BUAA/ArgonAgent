# Phase 4 · DeepSeek 单向依赖收口契约

> 状态：**Opus 设计已签**（2026-05-26）
> 上游：[`docs/architecture_consolidation_plan.md` §Phase 4](../architecture_consolidation_plan.md)
> 下游：Codex 按本契约执行 4.2-4.4

---

## 1. Owner 决定

**`native_profile/deepseek/` 是 DeepSeek profile 的唯一 owner。** 顶层 5 个 `deepseek_*.rs` 在 Phase 4 结束时全部消失（合并 / 拆解）。

## 2. 当前双向依赖图（必须打破）

```
deepseek_adaptation.rs ──→ native_profile::deepseek::reasoning::ReasoningReplayManager
native_profile/deepseek/stream_processor.rs ──→ deepseek_stream::DeepSeekStreamDelta
native_profile/deepseek/reasoning.rs ──→ deepseek_reasoning::sanitize_reasoning
```

**方向约束（不可违反）**：合并后只能存在 `native_profile/deepseek/ → 外部 runtime`，禁止外部 runtime → `native_profile/deepseek/` 中的实现细节。外部调用者只能用 `NativeProfile` trait / `profile_for_family` 工厂获得 DeepSeek 行为。

## 3. 文件归宿映射

| 顶层文件 | 行数 | 目标位置 | 处理方式 |
|---|---|---|---|
| `deepseek_stream.rs` | 697 | `native_profile/deepseek/stream.rs` (306) + `stream_processor.rs` (431) 内合并 | 把 `DeepSeekStreamDelta` 等 type 移入 `stream.rs`；把流式解析逻辑移入 `stream_processor.rs`；二者已是 owner，吸收 |
| `deepseek_adaptation.rs` | 666 | **新建** `native_profile/deepseek/adaptation.rs` | 整体搬入，搬入后改 import `super::reasoning::ReasoningReplayManager`（去掉 `crate::native_profile::` 前缀，因为已同目录） |
| `deepseek_cache_planner.rs` | 221 | `native_profile/deepseek/cache_prefix.rs` (216) 内合并 | 两者都是 cache 相关；合并后单文件约 ~400 行可接受 |
| `deepseek_reasoning.rs` | 136 | `native_profile/deepseek/reasoning.rs` (149) 内合并 | `sanitize_reasoning` 等 fn 合入 owner 文件 |
| `deepseek_runtime_policy.rs` | 162 | **新建** `native_profile/deepseek/policy.rs` | 整体搬入 |

**合并完成后** `native_profile/deepseek/` 结构：

```
native_profile/deepseek/
  mod.rs              (re-export 表 + DeepSeekProfile impl NativeProfile)
  stream.rs           (~700)  — DeepSeekStreamDelta + 流式 type + 拼装
  stream_processor.rs (~450)  — ToolCallPipeline 入口（已用）
  adaptation.rs       (新, ~666)
  cache_prefix.rs     (~437) — 3-zone prompt + 缓存策略
  reasoning.rs        (~285) — ReasoningReplayManager + sanitize_reasoning
  role_split.rs       (79)   — Executor/Compactor 角色枚举
  policy.rs           (新, ~162)
```

合并后顶层 `deepseek_*.rs` 5 个文件**全部删除**。

## 4. 外部调用者重写（11 个文件）

合并完成后这些外部站点必须改 import 路径：

| 文件 | 当前 import | 改为 |
|---|---|---|
| `native_response_normalizer.rs` | `use crate::deepseek_stream::*` | `use crate::native_profile::deepseek::stream::*` |
| `agent_loop_driver.rs` | 同上 | 同上 |
| `model_transcript.rs` | 同上 | 同上 |
| `provider_response_adapter.rs` | 同上 | 同上 |
| `qwen_stream.rs` | 同上 | 同上 |
| `native_agent_loop.rs` | `use crate::deepseek_{stream,adaptation,reasoning,...}` | 全部改 `crate::native_profile::deepseek::...` |
| `native_profile/qwen/reasoning.rs` | `use crate::deepseek_reasoning::sanitize_reasoning` | `use crate::native_profile::deepseek::reasoning::sanitize_reasoning` |
| `native_profile/deepseek/reasoning.rs`（自身） | `use crate::deepseek_reasoning::sanitize_reasoning` | 合并后变本文件内函数，删除 `use` |
| `native_profile/deepseek/stream_processor.rs`（自身） | `use crate::deepseek_stream::DeepSeekStreamDelta` | 同上 |

## 5. Qwen 偷渡问题

`qwen_stream.rs` 与 `native_profile/qwen/*` 当前通过 `tcml/streaming_accumulator` 转发或 `crate::deepseek_*` 直接 import 跨界到 deepseek 实现细节——**Phase 4 不试图解决 qwen 的对应问题**。仅把 qwen 对 deepseek 的 import 路径从顶层改为 `native_profile::deepseek::`，留作 Phase 4-followup 评估"shared profile primitives"是否需要抽出。

PR 描述需附一份"qwen ↔ deepseek import 清单"作为后续 Phase 9 之前的资产。

## 6. 双跑策略

**不可双跑**（合并是结构改动不改行为）。改为：

1. 每个顶层 `deepseek_*.rs` 合并 PR 之前，先用 `#[deprecated(note = "moved to native_profile::deepseek::X")]` 在顶层文件加 `pub use crate::native_profile::deepseek::*::*;` 转发
2. CI 每日跑：
   ```
   cargo check 2>&1 | grep "deprecated.*deepseek_" | wc -l
   ```
   预期 N → 0（每日下降）
3. 本地整库 `rg` 确认旧路径 0 调用、`cargo check --workspace` / `cargo test --workspace` 干净后，可以删除转发 + 删除顶层旧文件

> 2026-05-26 裁决：本仓库当前执行 local-first 架构整合，Phase 4.3 已完成调用方重写且旧路径 grep 为 0；不再强制等待 7 天观察期。若未来恢复外部 crate API 稳定性要求，再重新引入观察窗口。

## 7. PR 拆解

按 §3 的 5 行表格 × 2（合入 + 删除）= **10 个 PR**，每个 PR 一个文件：

| 顺序 | PR | 内容 |
|---|---|---|
| 4.2.a | 引入 owner 的 deprecated re-export | 顶层 deepseek_stream.rs 变薄成 pub use 转发 |
| 4.2.b | 同上 for deepseek_adaptation/cache_planner/reasoning/runtime_policy | 4 个 PR |
| 4.3 | 外部调用者批量改 import 路径 | 11 文件 import 重写，1 个 PR |
| 4.4.a-e | 删除顶层 5 个 deepseek_*.rs | 各自独立 PR，依赖 4.3 完成 + 旧路径 grep 0 + cargo workspace 通过 |

总计 10 个 PR，约 1-2 周。

## 8. 验收

| 指标 | 目标 | 度量 |
|---|---|---|
| 顶层 `deepseek_*.rs` 文件数 | 0 | `ls crates/runtime/src/deepseek_*.rs 2>/dev/null \| wc -l` |
| `native_profile/deepseek/` 入边来自 `crate::deepseek_*` | 0 | `rg "use crate::deepseek_" crates/runtime/src/native_profile` |
| 反向依赖（runtime → native_profile/deepseek 实现细节） | 仅通过 trait / 工厂 | 见 §2 方向约束 |
| `cargo check --workspace` | 干净 | — |
| `cargo test --workspace` | 全绿 | — |
| Eval Gate R2/R3（DSML、tool_calls.delta） | 维持 90% | 现有测试套件 |

## 9. 不动什么

- 不动 `qwen_stream.rs` 与 qwen profile 的实现（仅 import 路径）
- 不动 `tcml/` 任何文件
- 不引入 `Box<dyn NativeProfile>` 动态分发（保持 `NativeProfileInstance` enum 现状）
- 不重命名任何 fn / type
- 不删除任何 fn（仅移动）
