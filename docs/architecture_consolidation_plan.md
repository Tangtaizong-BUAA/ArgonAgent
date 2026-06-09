# 架构整合修复执行计划

> 基于 2026-05-25 架构深度诊断。目标是把"加新代码不删旧、加文件不加边界、加抽象不加调用"的状态收敛回 doc39 北极星。

---

## 0. 工程纪律（贯穿全程，无例外）

1. **local-first**：每个 phase 在独立 git worktree 完成，本地 `cargo test -p researchcode-runtime --release` 全绿后才进入双跑/PR。
2. **不混合 PR**：一个 phase = 一个 PR；bug fix、refactor、rename 不夹带；删除老路径必须单独 PR。
3. **不偷渡依赖**：任何新 crate 依赖（含 dev-dep）必须独立 PR，PR 描述列出引入理由 + 引入者 + 替代方案。
4. **双跑替换**：新路径上线时旧路径保留至少 7 天，运行双跑对照断言（事件序列 diff = 0、permission decision 一致、token 使用差 < 2%）才允许删除旧路径。
5. **权威边界验收**：每个 phase 的 PR 必须附带 (a) 删除代码行数 (b) `cargo check` + `cargo test` 输出 (c) 双跑断言结果 (d) 一条 grep 证据证明旧路径无残留调用。
6. **不引入新功能**：本计划全程**不增**任何用户可见功能；只做边界整理、死代码消除、单一真相源化。
7. **可回滚**：每个 PR 必须能用 `git revert <sha>` 单独回滚不破坏后续 PR。如果出现"必须连带回滚"，说明 phase 拆得不够细。

---

## Phase 0 · 三个不可逆决策（先决，0 代码）

不做这三个决策，后续 phase 全部踏空。

### D1 · Desktop 宿主二选一：**Tauri**

证据：`desktop/src-tauri/src/main.rs` 已 1552 行 + 13 个 `#[tauri::command]` 直接调用 `RuntimeFacade`；Electron 路径还在 spawn 已废弃的 Python `local_api_server.py`。Tauri 路径短、跨平台 binary 更小、与 Rust runtime 同进程取消信号直达。

输出：[`docs/decisions/D1-tauri-only.md`](decisions/D1-tauri-only.md)。**状态：已决定**。

### D2 · HTTP transport 路径：**Phase 8 引入 tokio + reqwest，sidecar 保留 90 天双跑**

证据：每次模型请求 fork 一个 python3 子进程（`sidecar_http_transport.rs:225,270`）；Cancellation 是被 poll 的 `&AtomicBool`、sidecar 无 kill；Windows/容器部署摩擦大。

修正：实测 `provider_http_sidecar.py` 2166 行内含 SSE 流式解析与 API key 隔离的安全边界，不能仓促替换。改为 feature-flag 并存 + 90 天双跑。

输出：[`docs/decisions/D2-tokio-transport.md`](decisions/D2-tokio-transport.md)。**状态：已决定**。

### D3 · `crates/kernel` 定位：**改名 `kernel-types`；`agent_kernel/` 提升延后**

证据：`crates/kernel` 11 文件 2692 行全是 serde DTO；真正的 agent 抽象在 `crates/runtime/src/agent_kernel/` 17 文件 7162 行。命名误导是 doc39 落地的最大幻觉。

修正：实测 `agent_kernel/` 反向依赖 11 个 runtime 内部模块，物理提升会产生循环依赖。本轮只做改名，提升前置条件清单列入 Future Work。

输出：[`docs/decisions/D3-kernel-rename.md`](decisions/D3-kernel-rename.md)。**状态：已决定**。

---

## Phase 1 · 死代码删除 + 边界冻结（1 周，风险低）

唯一目的：把"已经没人用、但还在编译"的代码删干净，给后续 phase 一个干净起点。

| 任务 | 目标文件 | 验收 |
|---|---|---|
| 1.1 | 删除 `apps/desktop/`（与 `desktop/` 同名 stub） | `find apps/desktop -type f` 返回空 |
| 1.2 | 删除 `apps/mission_frontend/`（仅剩 `dist/` 孤儿） | 同上 |
| ~~1.3~~ | ~~删除 `agent_kernel/turn_controller.rs`~~ | **移出 Phase 1**——见下方修订记录 |
| ~~1.4~~ | ~~删除 `tool_orchestration.rs`~~ | **移出 Phase 1**——见下方修订记录 |
| 1.5 | 删除 `crates/runtime/src/tcml/streaming_accumulator.rs`（8 行转发 stub）+ 直接 `pub use` 在 `tcml/mod.rs` | `wc -l tcml/streaming_accumulator.rs` 不存在 |
| 1.6 | 标记并删除 `crates/runtime/src/*.rs` 中**真正** 0 调用的私函数（标 `#[allow(dead_code)]` 找出来 → grep 验证 0 调用 → 删） | rustc warning 数下降 ≥ N，PR 描述给每个删除项的"0 callers found"证据 |

**约束**：本 phase 不动任何还在运行的代码、不动任何接口。仅删**真正**的死代码。

**风险**：低（移除 1.3/1.4 之后）。

### 1.x 修订记录（2026-05-25，由 Codex 边界验收触发）

原 Phase 1 计划把 `agent_kernel/turn_controller.rs` 和 `tool_orchestration.rs` 列为死代码。Codex 执行 PR-1 前的 grep 验证发现两个文件均为活跃生产组件：

| 文件 | 真实状态 | 关键证据 |
|---|---|---|
| `agent_kernel/turn_controller.rs`（1751 行） | `NativeLoopTurnController` 是 `AgentKernel.turn_controller` 字段，由 `AgentKernel::for_request` 实例化 | `native_agent_loop.rs:262`、`runtime_facade.rs:2162/2738/2740/2975/2977` 全部活路径 |
| `tool_orchestration.rs`（349 行） | `execute_batch` 是并发只读批次的真实执行路径；`partition_tool_calls` 被 `AgentKernel::run_turn` 调用 | `native_agent_loop_tools.rs:495` 真调用 `execute_batch`；`agent_kernel/kernel.rs:139` 真调用 `partition_tool_calls` |

**审计错误溯源**：
- 2026-05-16 gap 分析时 `turn_controller.rs` 是 476 行的字节级重复
- 2026-05-16 → 2026-05-25 期间该文件长到 1751 行，新增 `NativeLoopTurnController` 类并被 `AgentKernel` 接入到生产路径——这正是 doc39 "加新代码不删旧代码"病的活样本
- 2026-05-25 我的 subagent 审计说 tool_orchestration "main loop zero calls"——审计漏看了 `native_agent_loop_tools.rs`（family 一员，本身就是 main loop）

**重新归类**：

| 原任务 | 真实性质 | 推迟到 |
|---|---|---|
| 1.3（unify turn controllers） | 新旧并行系统，需替换-then-删除型重构 | **Phase 6**——runtime_facade 拆分时 `AgentKernel` 的实际角色定型后，统一 `NativeLoopTurnController` vs `NativeTurnController` |
| 1.4（unify tool batch execution） | 新旧并行系统：`tool_orchestration::execute_batch`（concurrent）vs `tool_execution::execute_tool`（single） | **Phase 3**——sibling 边界化把 `native_agent_loop_tools.rs` 对 tool_orchestration 的依赖暴露到 L2 API，那时一并裁决 |

**教训**：未来"死代码"判定必须验证构造调用链，不能只验证"import 来自哪里"。审计输出的"main loop 零调用"必须明确"main loop"是否包含 sibling family。

---

## Phase 2 · Test / Fixture 出生产 crate（拆为 2.a + 2.b）

### 2.x 修订记录（2026-05-25，由 Codex 边界验收触发）

原 Phase 2 把以下文件统一列为"测试/fixture，应移出生产 crate"，验证后发现归类错误：

| 文件 | 我原以为 | 真实状态 |
|---|---|---|
| `research_worker.rs` | fixture | **生产代码**：`tool_execution.rs:16` 调 `run_csv_profile_sidecar` 跑 `research.csv_profile` 工具 |
| `native_agent_loop_tests.rs` | "移到 tests/" | **已被父文件 `#[cfg(test)]` gate 排除 release binary**；用 `super::*` 访问 ~10 个私有 helper（`native_loop_user_prompt_for_event`、`native_prompt_wants_file_generation`、`emit_runtime_visible_finalizer_fallback`、`extract_final_answer_tool_call` 等）。移动会强迫拓宽生产可见性 → 反 P3.1 规范 |
| `harness.rs` / `executor.rs` / `tool_harness.rs` / `recorded_*.rs` / `research_harness.rs` / `native_agent_loop_fixtures.rs` | "移到 crates/runtime-fixtures" | 被 `cli/main.rs:38,47,108,111,116,143` **顶层（非 cfg(test)）** import；服务 cli 的 `*_smoke`/`*_cli` 生产子命令。移走 runtime 侧必须先移走 cli 侧消费者 |

**修正版 Phase 2 分两步**：

### 2.a · cli 开发子命令搬出（**Codex 立刻执行**）

唯一目标：把 `crates/cli/src/main.rs` 中 lines 1674-4795 的 `*_smoke`/`*_cli`/`*_fixture` 函数（约 3120 行 / 约 50 个 fn）抽到新 binary `crates/cli-dev-tools`。

**约束**：
- 新 crate 仅依赖现有 `researchcode-runtime`/`researchcode-kernel`，不可新增 crate.io 依赖
- 新 binary 名建议 `researchcode-dev-tools`，与生产 `researchcode-cli`/`cli` 区分
- `crates/cli/src/main.rs` 的 main dispatch 中对应子命令分支删除或转发到新 binary（推荐删除，让 dev-tools 是独立入口）
- `runtime` 不动；`runtime` 内的 harness/fixture/executor/recorded_*/research_harness 等暂留原位

**验收**：
- `wc -l crates/cli/src/main.rs` ≤ 6500（PR 描述给实测值）
- `cargo build --workspace` + `cargo test --workspace` 全绿
- `cargo build --release -p researchcode-cli` 产物体积下降记入 PR
- 新增 dev-tools binary 可独立 `cargo run --bin researchcode-dev-tools -- <subcommand>` 跑通至少 3 个 smoke 子命令

### 2.b · runtime 侧 fixture/harness 集群处置（**推迟到 Phase 3 完成后**）

`harness.rs`、`executor.rs`、`tool_harness.rs`、`recorded_agent_loop.rs`、`recorded_research_loop.rs`、`research_harness.rs`、`native_agent_loop_fixtures.rs` 共约 4170 行 fixture 集群——在 2.a 完成后，cli 不再是它们的生产消费者，只有：
- 其他 production .rs 文件内的 `#[cfg(test)] mod tests` 块（`replay.rs`、`event_invariants.rs`、`approval_queue.rs`）
- `native_agent_loop_tests.rs`（已 cfg(test) gated）

此时可评估三选一：
- (i) 移到独立 `crates/runtime-fixtures`，runtime 内 `#[cfg(test)] mod tests` 改用 `[dev-dependencies] runtime-fixtures`
- (ii) 在原地加 `#[cfg(any(test, feature = "fixtures"))]` gate，release binary 默认不含
- (iii) 抽出部分（如 `recorded_*`）到独立 crate，其余 cfg gate

裁决需在 Phase 3 完成（sibling 边界化暴露 fixture 真实依赖）之后由 Opus 做。**Codex 不执行 2.b**。

### 2.c · 不在 Phase 2 范围

- **`research_worker.rs`** — 生产代码，不动
- **`native_agent_loop_tests.rs`** — 已 cfg(test) gated，保持原状（继续作 sibling）
- 任何 cli/main.rs 中非 dev 子命令的代码——只动 lines 1674-4795 范围内的 `*_smoke`/`*_cli`/`*_fixture` 函数

---

## Phase 3 · `native_agent_loop_*.rs` 兄弟边界化（2 周）

**Phase 3.0（前置任务，由 Phase 1 修订记录推迟而来）**：统一 `tool_orchestration::execute_batch`（concurrent 批次路径）与 `tool_execution::execute_tool`（单工具路径）。

状态：✅ 已完成 2026-05-26，采用 A 方案：`tool_execution::execute_tool_batch_concurrent` 作为 batch 执行入口；`tool_orchestration` 保留批次规划/partition 与兼容转发边界。

- 完成后：`native_agent_loop_tools.rs` 不再直接依赖 `tool_orchestration::execute_batch`；批次执行入口统一为 `tool_execution::execute_tool_batch_concurrent`
- `tool_orchestration` 保留批次规划/partition 边界，并通过 `pub use` 转发 `ToolCall`/`ToolBatch`/`SiblingAbortController` 以保持既有调用兼容
- Opus 已裁决接受 A；B（把 `tool_orchestration::execute_batch` 升为标准入口）不再执行
- 不可双跑（执行状态独占）。用 contract test：100+ fixture tool batch 在两条路径下结果等价

---



**根问题**：原 sibling 全部 `use super::*;`，pub(crate) 满天飞，编译器把它们当一个模块。

### 3.1 边界设计（必须先 done）

为现存 sibling 各自定义 **显式 pub API**：

- `entrypoints` — 仅公开生产入口 `pub fn run_native_agent_loop_*`
- `prompt` — `assemble_native_prompt(&AgentSession, &TurnContext) -> NativePrompt`
- `model_io` — `request_native_model(...) -> NativeModelOutcome`
- `tools` — `execute_native_tool_batch(...) -> ToolBatchResult`；已吸收原 `stream` sibling 的 streamed tool execution
- `completion` — `finalize_native_turn(...) -> TurnCompletion`
- `continuation` — `prepare_continuation(...) -> ContinuationDecision`
- `resume` — `resume_after_external_decision(...) -> ResumeOutcome`
- `execution` — `record_execution_milestone(...)`
- `util` — 仅暴露 **数据无副作用**的纯函数（5 个以内）
- `fixtures` — Phase 2.b 待裁决，本轮不动

### 3.2 实施

1. 每个 sibling 顶部 `use super::*` → 显式 `use crate::native_agent_loop::{...}`。
2. 所有 `pub(crate)` / `pub(super)` 默认收为 `pub(in crate::native_agent_loop)` 或 private；保留的暴露必须出现在 3.1 的清单里。
3. CI 加 lint：`scripts/lint_native_loop_boundary.sh` 检查 sibling 内 `use super::\*` 命中数 = 0。

### 3.3 不允许新 sibling

`docs/policy/native_agent_loop_freeze.md` 写明：禁止新增 `native_agent_loop_*.rs`。新需求若不属于现有 11 个 sibling 之一，进入 `agent_kernel/` 或新子目录。

**验收**：3 项硬指标——`use super::*` 命中 0；sibling 总 `pub` 项 ≤ 60；sibling 内 fn 跨调用数下降 ≥ 50%（用 `cargo-modules generate graph` 出图对比）。

---

## Phase 4 · DeepSeek 单向依赖收口（1-2 周）

**根问题**：`deepseek_*.rs`（顶层 5 文件）和 `native_profile/deepseek/*.rs`（5 文件）形成双向 import 循环。

### 4.1 方向

`native_profile/deepseek/` 是 owner。顶层 `deepseek_*.rs` 全部合入或拆解到 profile 内部：

| 顶层文件 | 行数 | 处置 |
|---|---|---|
| `deepseek_stream.rs` | 697 | 合入 `native_profile/deepseek/stream.rs` + `stream_processor.rs` |
| `deepseek_adaptation.rs` | 666 | 合入 `native_profile/deepseek/adaptation.rs`（新） |
| `deepseek_cache_planner.rs` | 221 | 合入 `native_profile/deepseek/cache_prefix.rs` |
| `deepseek_reasoning.rs` | 136 | 合入 `native_profile/deepseek/reasoning.rs` |
| `deepseek_runtime_policy.rs` | 162 | 合入 `native_profile/deepseek/policy.rs`（新） |

### 4.2 双跑策略

每个文件合并时：

1. 旧文件保留 `#[deprecated(note = "moved to native_profile::deepseek::X")]` 的 `pub use` 转发 7 天。
2. CI 加断言：`cargo check 2>&1 | rg "deprecated.*deepseek_" | wc -l` = N（每周递减）。
3. 7 天后删除转发 + 删除旧文件。

### 4.3 验收

- `cargo-depgraph` 输出 `native_profile/deepseek/` 入度 = 0 来自 `crates/runtime/src/deepseek_*`
- `qwen_stream.rs` 不再通过 tcml stub 偷渡 deepseek 的 `StreamingToolCallAssembler`——要么提升到 `native_profile::shared`，要么 qwen 自带实现

---

## Phase 5 · 权限系统五归一（2 周，风险高）

**根问题**：5 套并行 permission 实现，`permission_resolver.rs` 1174 行的"统一器"自己又是一套。

### 5.1 目标态

- **唯一策略**：`agent_kernel::PermissionPolicy`（5 模式 evaluate）
- **唯一守门**：`agent_kernel::PermissionGate`（10 步算法已 done，Phase A 验过）
- **唯一规则存储**：`permission_policy::PermissionRuleStore`（保留 TSV 规则文件解析）
- **去除**：`permission.rs`、`permission_resolver.rs`、`NativeAgentPermissionMode`

### 5.2 步骤

1. `tool_execution.rs:139` 的就地 permission 检查改为调 `PermissionGate::check(...)`。
2. `permission_resolver.rs::evaluate_permission_request` 内部逻辑切片移入 `PermissionPolicy::evaluate` + `PermissionGate::check`；保留薄 adapter。
3. **双跑**：runtime 同时计算 old/new decision，事件流附带 `permission.decision.shadow_compare`，差异条数计入 telemetry。
4. 双跑 7 天，差异 = 0 才删 adapter。

### 5.3 验收

- `rg "permission_resolver::" crates` 0 命中
- `rg "NativeAgentPermissionMode" crates` 0 命中
- Eval Gate R-perm（新增）：100 个 fixture session 跑过，permission 事件序列 100% 一致

---

## Phase 6 · 拆 `runtime_facade.rs` + 统一 TurnController（2-3 周，最大手术）

**6.0（前置任务，由 Phase 1 修订记录推迟而来）**：统一 `NativeLoopTurnController`（`agent_kernel/turn_controller.rs` 1751 行）与 `NativeTurnController`（`native_turn_controller.rs` 619 行）。

- 当前两者并行：`AgentKernel.turn_controller: NativeLoopTurnController` 由 `AgentKernel::for_request` 实例化，被 `native_agent_loop.rs:262` 与 `runtime_facade.rs` 5 处调用；同时 `NativeTurnController` 被 `native_agent_loop.rs:349` 直接 `new_for_session`
- Phase 6.0 必须**先**裁决谁是真相源，再拆 facade（拆 facade 时 `AgentKernel` 的角色必须明确）
- 双跑：不可（两个 controller 状态独占）。改为 contract test：以 `NativeLoopTurnController` 的 trait 行为为基准，证明 `NativeTurnController` 的行为可用前者替代（或反之）
- 删除一方时同步删除其文件 + agent_kernel/mod.rs 的 re-export

---



**根问题**：6581 行 god-object，44 个 pub fn，3 个 Mutex<HashMap>，集 session 仓库 / context 构建 / 权限规则 / Ctrl-C / subagent 生命周期于一身。

### 6.1 目标拆分

| 新模块 | 职责 | 估计行数 |
|---|---|---|
| `runtime/session_store.rs` | `Mutex<HashMap<SessionId, AgentSession>>` + CRUD + 锁粒度 | ~600 |
| `runtime/subagent_store.rs` | subagent 生命周期 + status FSM | ~500 |
| `runtime/context_service.rs` | `build_context_bundle` + 接 ConversationHistory（解锁 doc39 Phase 3） | ~400 |
| `runtime/permission_service.rs` | `PermissionRuleStore` 包装 + Gate 调用 | ~200 |
| `runtime/interrupt_service.rs` | Ctrl-C / 取消信号集中点（为 Phase 8 tokio 化预留） | ~150 |
| `runtime_facade.rs` 收缩为 | 仅 thin API surface，组合上述 service | ≤ 800 |

### 6.2 双跑

不可双跑（Mutex 状态独占）。改用 **contract test 法**：

1. 写 80+ 个 `RuntimeFacade` contract test，覆盖每个 pub fn 的输入/输出/副作用。
2. 每抽一个 service 出去，contract test 全跑过才合 PR。
3. 重构期间禁止给 facade 加任何新 pub fn。

### 6.3 验收

- `wc -l runtime_facade.rs` ≤ 800
- `cargo test -p researchcode-runtime --test facade_contract` ≥ 80 个 case 通过
- service 各文件 `Mutex<HashMap>` 数 ≤ 1

---

## Phase 7 · TurnRouter + Compactor 真接入或删除（1 周）

### 7.1 TurnRouter

现状：`native_agent_loop.rs:291` 调用一次 `classify`，结果只在 `:297-298` `format!` 成 telemetry 字符串。

判定（doc39 §3 已要求 route 切 manifest exposure）：**接入**。

- 把 `tool_manifest` 切片改为 `manifest_for_route(turn_route, ...)`
- ProjectStatus / DirectAnswer 路由跳过 write tools
- 删除 `deepseek_runtime_tool_exposure_for_prompt` 的 25 个中文关键词分支（被 route 替代）

### 7.2 Compactor

现状：发了 `context.compaction.completed` 假事件，从不调 `compact()`。

- 在 `native_agent_loop_model_io.rs` 的 `request_native_model` 前置插入 `if budget.should_compact(state) { compactor.compact(session) }`
- `Compactor::compact()` 改为**真修改 EventLog**（拼接 summary 事件 + 标记 compacted 区间为 archived）
- 接入 `ReasoningReplayManager::compact_old_reasoning`（preserve_latest_reasoning 字段终于派上用场）

### 7.3 验收

- 200K token 长 session 跑过：actual tokens_in 下降 ≥ 30%
- `context.compaction.completed` 事件出现频次与实际 EventLog 修改次数一致（断言：1:1）
- 移除关键词分支后，Eval Gate 通过率不降

---

## Phase 8 · HTTP transport 现代化（5-6 周，最大不确定性）

**全部细节见决策文档 [`docs/decisions/D2-tokio-transport.md`](decisions/D2-tokio-transport.md)。** 本节仅摘要里程碑：

| 子 phase | 内容 | 关键约束 |
|---|---|---|
| 8.1 | 独立 PR 引入 tokio + reqwest 依赖，**0 行 .rs 改动** | 不可逆点，预先 D2 决策确认 |
| 8.2 | `LiveHttpTransport` 扩展 `send_async`，sidecar 用 `spawn_blocking` 包同步实现 | 不打断 sync 调用 |
| 8.3 | 新增 `ReqwestLiveHttpTransport`，与 sidecar fixture 逐 byte 对比 | 每 provider 至少 10 个 fixture |
| 8.4 | `RESEARCHCODE_TRANSPORT=reqwest` env opt-in，**默认仍 sidecar** | 默认行为零变化 |
| 8.5 | 双跑 14 天，事件流 diff 计入 telemetry | `dual_run.diff_count` < 0.1% |
| 8.6 | `CancellationToken` 替换硬编码 `AtomicBool::new(false)` | Ctrl+C 实测 < 200ms |
| 8.7 | 默认 transport 切 reqwest，sidecar 标 `#[deprecated]` | 双跑 diff = 0 才切 |
| 8.8 | sidecar 删除（**90 天后**，独立 PR） | 安全审计 + 1000 次 provider 请求覆盖 |

**与初版差异**：sidecar 保留期从 "30 天 deprecated 后删" 修正为 **"≥ 90 天双跑保留"**——Python sidecar 实际承担流式 SSE 解析与 API key 隔离的安全边界，不是简单 HTTP wrapper。

---

## Phase 9 · Desktop 单一路径（2 周，依赖 Phase 0.D1）

| 任务 | 验收 |
|---|---|
| 9.1 删除 `desktop/electron/` 整目录 | `find desktop/electron` 空 |
| 9.2 删除 `scripts/local_api_server.py` | 同上；端口 8765 仅 Rust 绑定 |
| 9.3 简化 `desktop/src/runtime/localRuntimeClient.ts` 仅保留 Tauri + browser fallback | `rg "electronAPI" desktop/src` 0 命中 |
| 9.4 评估 `apps/open_claudecode_tui_adapter/`：保留 = 重命名到 `tools/`；不留 = 删除 | 决策记录到 `docs/decisions/D4-open-tui-fate.md` |
| 9.5 通信通道从 5 → 2（Tauri command + browser HTTP fallback） | 通道清单文档 |

**双跑**：用户侧不可双跑（UI 体验）。改为 `desktop/gui_three_round_smoke.mjs` 升级到 30+ 测例，每次 PR 跑。

---

## Phase 10 · `crates/kernel` 重命名（1 周，依赖 Phase 0.D3）

**全部细节见决策文档 [`docs/decisions/D3-kernel-rename.md`](decisions/D3-kernel-rename.md)。**

### 10.A 改名（本 phase 唯一目标）

```
crates/kernel → crates/kernel-types
researchcode-kernel → researchcode-kernel-types
```

仅 rename，不动任何 .rs 内部逻辑。影响 3 个 Cargo.toml + ~200 个 `use` 站点。

### 10.B（移出本计划）

原计划"把 `agent_kernel/` 提升为新 `crates/kernel`"在 Phase 0 重新评估后**移出本计划**。

理由：`agent_kernel/` 当前依赖 11 个 runtime 内部模块（session、event_log、tcml、native_turn_controller、compaction、context_budget、patch、live_http_transport、native_provider、live_model_request、tool_execution）。物理提升必须先把这些抽出 crate，否则产生循环依赖——比现状更糟。

agent_kernel 提升的前置条件清单见 [`D3-kernel-rename.md` § Future Work](decisions/D3-kernel-rename.md#future-work)：

- Phase 5、Phase 6 完成
- `event_log` / `session` / `tcml` 各自评估剥离方案
- `agent_kernel/` 外部依赖收敛至 ≤ 3 个 runtime 模块

满足条件后开 D3-followup 决策重新启动。

---

## 不在范围

- ❌ 任何 GUI 视觉/交互调整（另开 epic）
- ❌ 新增 model provider 接入
- ❌ doc39 Phase 6（ResultFormatter）/ Phase 7（subagent task.dispatch）/ Phase 8（telemetry 补全）—— 本计划结束后单独立项
- ❌ Eval Gate R1-R10 的功能补全（仅做接入侧改造）

---

## 验收路标

| 时点 | 状态 |
|---|---|
| Phase 1-2 完成 | runtime/src 顶层文件 ≤ 60，cli/main.rs ≤ 6500 行 |
| Phase 3-4 完成 | sibling 0 `use super::*`，deepseek 单向依赖图 |
| Phase 5-6 完成 | 权限单一源，facade ≤ 800 行 |
| Phase 7 完成 | doc39 Eval Gate 通过 6/10 |
| Phase 8 完成 | 模型请求延迟下降，cancellation 真生效 |
| Phase 9-10 完成 | UI 2 通道，kernel crate 物理结构与 doc39 一致 |

---

## 启动顺序

```
Phase 0 (决策, 0 代码)
  ├── Phase 1 (死代码, 解锁所有)
  │    └── Phase 2 (test 出 crate)
  │         └── Phase 3 (sibling 边界)
  │              └── Phase 4 (deepseek 收口)
  │                   └── Phase 5 (权限收口)
  │                        └── Phase 6 (拆 facade)
  │                             ├── Phase 7 (router + compactor)
  │                             ├── Phase 8 (tokio, 可与 7 并行)
  │                             └── Phase 9 (desktop, 可与 7/8 并行)
  └── Phase 10 (kernel 重定位, 最后)
```

预计总跨度：**14-18 周**（按 1 个工程师全职估算；如果 phase 6/8/9 真并行可压到 11-13 周）。
