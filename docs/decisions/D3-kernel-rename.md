# D3 · `crates/kernel` 重命名为 `kernel-types`；`agent_kernel/` 暂留 runtime 内

> 状态：**决定**（2026-05-25）
> 决策者：architecture review
> 触发：架构整合 Phase 0
> 后续 phase：Phase 10（kernel 重定位）

---

## 决策

1. **Phase 10.A**：`crates/kernel` 改名为 `crates/kernel-types`，包名 `researchcode-kernel` → `researchcode-kernel-types`
2. **Phase 10.B**：`crates/runtime/src/agent_kernel/` **暂不**物理提升到独立 crate。仅在 Phase 5/6 完成后**重新评估**是否可提升
3. 立即输出新 doc39 北极星修正：当前 `crates/kernel` 的角色应被命名为"kernel 数据模型层"，不是"agent kernel"本身

---

## 现状证据

### `crates/kernel` 实际内容

```
crates/kernel/src/
  lib.rs    155 行   pub mod 11 个 + Actor/KernelEvent/PermissionRequestType DTOs
  context.rs 83
  hooks.rs   148
  memory.rs  115
  message.rs 159
  model.rs   398   NativeModelFamily / DeepSeekVariant / ToolCallingReliability 等枚举
  plan.rs    210
  subagent.rs 172
  task.rs    199
  tool.rs    958   ToolSpec / ToolKind / PermissionRequired 等元数据
  transcript.rs 95
```

特征：
- `[dependencies]` 仅 `serde_json = "1"`，**零运行时逻辑**
- 全部是 `serde::{Serialize, Deserialize}` 派生的纯数据
- 被 cli + runtime + desktop/src-tauri 三处导入

结论：当前 `crates/kernel` **物理上是 `kernel-types`**——一个共享 DTO crate，命名误导。

### `crates/runtime/src/agent_kernel/` 实际依赖

17 个文件中，外向 import 涵盖 runtime 几乎所有大模块：

```
kernel.rs            → session, event_log, live_http_transport, live_model_request,
                       native_agent_loop, native_profile, native_turn_controller,
                       permission_policy, tcml, context_budget
turn_controller.rs   → session, native_turn_controller, tcml, patch
permission_gate.rs   → permission_policy, permission_resolver
compactor.rs         → compaction, context_budget, event_log
conversation_history.rs → event_log
evidence_ledger.rs   → tool_execution
observation_cache.rs → patch, tcml
provider_capability.rs → native_provider, patch
telemetry.rs         → event_log
```

加上对 `researchcode-kernel`（当前的 DTO crate）的依赖：

```
compactor.rs        → KernelEvent, Actor
conversation_history.rs → Actor
kernel.rs           → model::NativeModelFamily
mod.rs              → re-export AgentKernel/ContextManager/Finalizer/TcmlService/ToolOrchestrationService
permission_policy.rs → PermissionRequestType
provider_capability.rs → model::{DeepSeekVariant, NativeModelFamily, ToolCallingReliability}
telemetry.rs (test) → Actor, KernelEvent
```

结论：**`agent_kernel/` 已经把 runtime 的内部当作"自己的下层"**，不是独立可剥离的层。物理提升必须先做 Phase 5（权限收口）+ Phase 6（runtime_facade 拆分）+ 抽出 `event_log` / `session` / `tcml` 等子模块成独立 crate，否则就是把循环依赖搬到 crate 边界上。

---

## 候选方案

### 方案 A：现在就提升 `agent_kernel/` 为 `crates/kernel`（激进）

代价：必须同时把 `session`、`event_log`、`tcml`、`native_turn_controller`、`compaction`、`context_budget`、`patch`、`live_http_transport`、`native_provider`、`live_model_request`、`tool_execution` 等 11 个 runtime 子模块拆出独立 crate，否则编译失败。

这是 6 个月的工程。

### 方案 B：完全不动 crate 命名（保守）

代价：doc39 北极星图与代码物理结构永久不一致；新加入工程师必然把 `crates/kernel` 误认为 agent 内核。

### 方案 C：只改名，不挪逻辑（决定）

- `crates/kernel` → `crates/kernel-types`，诚实命名
- `agent_kernel/` 物理位置不变，但在 doc39 北极星图明确标注其当前为"runtime 内部的 kernel 层抽象"，且为"未来 kernel crate 的源头"
- Phase 10 完成后留一份"kernel 提升前置条件清单"（Phase 5 完成、Phase 6 完成、event_log/session/tcml 抽出独立 crate），等条件满足再重新启动 Phase 10.B

---

## 选择：方案 C

理由：

1. **"诚实命名"是零代价收益**：rename 是 IDE 一键操作，cargo 包名一处改 + 三处依赖文件 + 几百个 import 行（机械替换）。
2. **不打破现有工作**：当前 `agent_kernel` 内部的几个真接入组件（`PermissionGate`、`EvidenceLedger`、`ObservationCache`、`AgentKernel::for_request`）继续工作，不被迫为 crate 边界做适配。
3. **拒绝伪 doc39 对齐**：把 `agent_kernel/` 硬塞进 `crates/kernel` 表面上"对齐"了 doc39，实际产生新一轮循环依赖（runtime ↔ kernel 互相 import）——比现状更糟。
4. **保留未来可能性**：Phase 5/6 完成后，`agent_kernel` 的外部依赖会显著收敛（不再依赖 `permission_resolver`、不再依赖 god-object `runtime_facade`），那时再评估提升。

---

## 落地路径（Phase 10 细化）

### 10.A 改名（Phase 10 主体，1 周）

1. 独立 PR：`git mv crates/kernel crates/kernel-types`
2. `Cargo.toml` 包名 `researchcode-kernel` → `researchcode-kernel-types`
3. 全 workspace 替换：
   - `[dependencies] researchcode-kernel` → `researchcode-kernel-types`
   - `use researchcode_kernel::` → `use researchcode_kernel_types::`
   - 影响 3 个 Cargo.toml + 约 200 个 use 站点
4. 不动任何 .rs 内部逻辑、不调任何字段
5. 验收：`cargo build --workspace` + `cargo test --workspace` 全绿
6. 更新 `docs/doc39_implementation_gap_analysis.md`：把"kernel crate"的描述改为"kernel-types crate (DTO 层)"

### 10.B 暂不执行（写入文档作为延期事项）

在 `docs/decisions/D3-kernel-rename.md`（本文件）追加 "Future Work" 节，列出 agent_kernel 提升的前置条件：

| 前置 | 状态 |
|---|---|
| Phase 5 完成（权限五归一） | 待 |
| Phase 6 完成（facade 拆分，god-object 消除） | 待 |
| `event_log` 抽独立 crate 或确认留 runtime | 待评估 |
| `session` / `tcml` 同上 | 待评估 |
| `agent_kernel/` 外部依赖收敛到 ≤ 3 个 runtime 模块 | 待 |

满足全部前置后，开 D3-followup 决策文档重新评估。

---

## 验收

- `find crates -maxdepth 1 -type d -name "kernel"` 返回空
- `find crates -maxdepth 1 -type d -name "kernel-types"` 返回 1
- `rg "researchcode_kernel[^_]" crates desktop apps` 0 命中（旧名称完全消失）
- `cargo build --workspace --all-targets` 通过
- `cargo test --workspace` 通过
- `docs/doc39_implementation_gap_analysis.md` 内的"kernel crate"措辞同步更新

---

## 撤销代价

| 时点 | 撤回代价 |
|---|---|
| 改名后立即 | 低——git revert 单 PR |
| 改名后 30 天内 | 低——一次性 rename PR |
| 改名后 90 天后 | 中——外部文档、决策记录引用了新名 |

改名本身**风险极低**——所有副作用都在编译期暴露，运行时行为零变化。

---

## 反对意见与回应

**反 1**：改名会让 git blame / git log 变难追溯。
**回应**：`git log --follow` 自动跟踪文件移动；rename 不变内容、`git mv` 是 git 原生操作，blame 完整保留。

**反 2**：`researchcode-kernel-types` 名字太长。
**回应**：crate 名只在 Cargo.toml 出现；代码内用 `use researchcode_kernel_types as kt;` 别名即可。诚实命名 > 简短命名。

**反 3**：暂不提升 agent_kernel，doc39 北极星永远落不了地。
**回应**：北极星不是教条，是方向。在 Phase 5/6 没解决前强行提升是反方向。本决策保留了提升的可能性，未否定北极星——只是承认达成它需要先做 7 个 phase 的解耦工作。

**反 4**：既然不挪 agent_kernel，干嘛叫"Phase 10 kernel 重定位"？
**回应**：本计划同步修订——Phase 10 标题改为"kernel-types 重命名"，删除 10.B 的承诺，把 agent_kernel 提升列为 Phase 10 之后的 Future Work。计划文档同步更新见本 PR 配套修改。

---

## Future Work：agent_kernel 提升触发条件

以下条件**全部**满足后，开 `docs/decisions/D3-followup-agent-kernel-promote.md` 重新评估：

1. [ ] Phase 5 完成：runtime 内仅剩 `agent_kernel::{PermissionPolicy, PermissionGate}` + `permission_policy::PermissionRuleStore` 三个权限组件
2. [ ] Phase 6 完成：`runtime_facade.rs` ≤ 800 行，god-object 拆为 5 个 service
3. [ ] `crates/runtime/src/event_log.rs` 重新评估：是否抽出 `crates/event-log` 或留 runtime
4. [ ] `crates/runtime/src/session.rs` 同上
5. [ ] `crates/runtime/src/tcml/` 重新评估：是否抽出 `crates/tcml` 或留 runtime
6. [ ] `cargo-depgraph` 验证 `agent_kernel/` 对 runtime 内部的依赖收敛到 ≤ 3 个模块

满足条件后 Future Work PR 应包含：
- 物理移动 `crates/runtime/src/agent_kernel/` → `crates/kernel/src/`
- 新 `crates/kernel` 依赖 `crates/kernel-types`
- `crates/runtime` 反过来依赖 `crates/kernel`（依赖方向反转）
- 不出现循环依赖（`cargo-depgraph` 强制）

---

## 备注

- 本决策不影响 `apps/desktop`、`apps/mission_frontend`、`apps/open_claudecode_tui_adapter`——它们当前不依赖 `researchcode-kernel`
- `desktop/src-tauri/Cargo.toml` 包名是 `researchcode-desktop-host`，已经依赖 `researchcode-kernel`，Phase 10.A 中同步改名
