# Phase 6 · `runtime_facade.rs` god-object 拆分契约

> 状态：**Opus 设计已签**（2026-05-26）
> 上游：[`docs/architecture_consolidation_plan.md` §Phase 6](../architecture_consolidation_plan.md)
> 依赖：Phase 5 完成（PermissionGate 已统一）；Phase 6.0 完成（TurnController 二选一）
> 下游：Codex 按本契约执行 6.1-6.4

---

## 1. 现状

```rust
pub struct RuntimeFacade {
    workspace_root: PathBuf,
    artifact_root: PathBuf,
    project_policy_store: PermissionRuleStore,
    sessions: Mutex<HashMap<String, RuntimeSessionRecord>>,
    subagents: Mutex<HashMap<String, SubagentSession>>,
    subagent_sessions: Mutex<HashMap<String, AgentSession>>,
}
```

- 文件 6581 行，42 个 pub fn
- 仅 7 个字段（不是字段数膨胀，是方法数膨胀）
- 外部消费者：`local_api_server.rs` 一个 + facade 自身——blast radius 窄

## 2. 拆分目标（5 service + 1 thin facade）

| 模块 | 职责 | 持有的锁 | 行数预算 |
|---|---|---|---|
| `runtime/session_store.rs` | session CRUD + RuntimeSessionRecord 管理 | `Mutex<HashMap<SessionId, RuntimeSessionRecord>>` | ~600 |
| `runtime/subagent_store.rs` | subagent + subagent_session 双 store + status FSM | `Mutex<HashMap<String, SubagentSession>>` + `Mutex<HashMap<String, AgentSession>>` | ~500 |
| `runtime/context_service.rs` | `build_context_bundle` + ConversationHistory 接入（解锁 doc39 Phase 3） | 无锁（无状态） | ~400 |
| `runtime/permission_service.rs` | `PermissionRuleStore` 包装 + PermissionGate 构造 | 无锁（store 内部已 sync） | ~150 |
| `runtime/interrupt_service.rs` | Ctrl-C / cancellation 集中点（Phase 8 tokio 化预留） | `Arc<AtomicBool>`（保留），Phase 8 改 `Arc<CancellationToken>` | ~150 |
| `runtime_facade.rs` 收缩为 | thin API surface：组合上述 service + 转发 | 无字段持锁（只持有 service 引用） | ≤ 800 |

合计 ~2600 行（vs 当前 6581 行）。剩余 ~3000 行是真正的"删/移到 fixture/合并重复"——预计减重 50%。

## 3. 方法分布

按 42 个 pub fn 类别归属：

| 类别 | 数量 | 目标模块 |
|---|---|---|
| session lifecycle (start/end/list/get) | ~10 | `session_store` |
| 主循环驱动 (`run_deepseek_agent_loop_*`) | ~6 | facade 保留（thin 转发到 `AgentKernel::for_request().run_turn(...)`) |
| context (`build_context_bundle`, ConversationHistory) | ~3 | `context_service` |
| permission decision (submit/recall/list) | ~5 | `permission_service` |
| subagent (dispatch/status/cancel) | ~8 | `subagent_store` |
| Ctrl-C / interrupt | ~3 | `interrupt_service` |
| 工具结果读取 (artifact / event_log query) | ~4 | facade 保留（薄读路径） |
| 杂项 (workspace path, file state cache) | ~3 | facade 保留 |

## 4. trait / struct 契约

### 4.1 `SessionStore`

```rust
pub struct SessionStore {
    sessions: Mutex<HashMap<String, RuntimeSessionRecord>>,
}

impl SessionStore {
    pub fn new() -> Self;

    pub fn insert(&self, id: String, record: RuntimeSessionRecord);
    pub fn get_clone(&self, id: &str) -> Option<RuntimeSessionRecord>;
    pub fn remove(&self, id: &str) -> Option<RuntimeSessionRecord>;
    pub fn list_ids(&self) -> Vec<String>;

    /// 加锁范围最小化：仅在闭包内持锁，外部 caller 不接触 MutexGuard
    pub fn with_mut<R>(
        &self,
        id: &str,
        f: impl FnOnce(&mut RuntimeSessionRecord) -> R,
    ) -> Option<R>;

    /// 跨 session 批操作（罕见，长时间持锁需评估）
    pub fn with_all<R>(
        &self,
        f: impl FnOnce(&HashMap<String, RuntimeSessionRecord>) -> R,
    ) -> R;
}
```

**锁粒度规则**：禁止把 `MutexGuard` 跨函数边界传递。所有 mutate 必须走 `with_mut` 闭包模式，闭包内不调用任何可能再次锁 sessions 的 fn（防嵌套死锁）。

### 4.2 `SubagentStore`

```rust
pub struct SubagentStore {
    subagents: Mutex<HashMap<String, SubagentSession>>,
    sessions: Mutex<HashMap<String, AgentSession>>,
}

impl SubagentStore {
    pub fn dispatch(&self, parent_id: &str, task: SubagentTask) -> SubagentId;
    pub fn status(&self, id: &str) -> Option<SubagentStatus>;
    pub fn cancel(&self, id: &str) -> Result<(), String>;
    pub fn with_session_mut<R>(
        &self,
        id: &str,
        f: impl FnOnce(&mut AgentSession) -> R,
    ) -> Option<R>;
}
```

**锁顺序规则**：当需要同时锁 `subagents` + `sessions` 时，**强制先锁 subagents 再锁 sessions**。违反顺序的代码 PR 直接拒收。

### 4.3 `ContextService`

```rust
pub struct ContextService;  // 无状态

impl ContextService {
    pub fn build_context_bundle(
        &self,
        session: &AgentSession,
        workspace_root: &Path,
    ) -> Result<ContextBundle, String>;

    /// doc39 Phase 3 接入点
    pub fn build_conversation_history(
        &self,
        session: &AgentSession,
    ) -> Vec<ConversationMessage>;
}
```

**新增职责**：调用 `agent_kernel::conversation_history::conversation_messages_from_event_log` 注入到 ContextBundle——这是 doc39 Phase 3（ConversationHistory 真接入）一并完成的设计。

### 4.4 `PermissionService`

```rust
pub struct PermissionService {
    rule_store: Arc<PermissionRuleStore>,
    workspace_root: PathBuf,
}

impl PermissionService {
    pub fn new(workspace_root: PathBuf) -> Result<Self, String>;

    /// 为一个 session 创建一个 PermissionGate（每 session 独立 denial 计数）
    pub fn new_gate(
        &self,
        session_id: &str,
        mode: PermissionMode,
        inline_policy: PermissionRuleSet,
    ) -> agent_kernel::PermissionGate;

    pub fn rule_store(&self) -> Arc<PermissionRuleStore>;
}
```

### 4.5 `InterruptService`

```rust
pub struct InterruptService {
    flag: Arc<AtomicBool>,
}

impl InterruptService {
    pub fn new() -> Self;
    pub fn handle(&self) -> Arc<AtomicBool>;
    pub fn interrupt(&self);
    pub fn is_interrupted(&self) -> bool;
}
```

**Phase 8 占位**：Phase 8 引入 tokio 后，新增 `pub fn cancel_token(&self) -> CancellationToken` 方法；不破坏现有 `handle()` API（保持 dual）。

### 4.6 收缩后的 `RuntimeFacade`

```rust
pub struct RuntimeFacade {
    workspace_root: PathBuf,
    artifact_root: PathBuf,
    sessions: Arc<SessionStore>,
    subagents: Arc<SubagentStore>,
    context: Arc<ContextService>,
    permissions: Arc<PermissionService>,
    interrupt: Arc<InterruptService>,
}

impl RuntimeFacade {
    pub fn new(workspace_root: PathBuf) -> Result<Self, String>;

    /// 主驱动入口（thin 转发到 AgentKernel）
    pub fn run_deepseek_agent_loop_request_with_interrupt<T: LiveHttpTransport>(
        &self,
        transport: T,
        request: NativeAgentLoopV2Request,
    ) -> Result<NativeAgentLoopResult, String>;

    /// 暴露 service 给 local_api_server 等高层 caller
    pub fn sessions(&self) -> Arc<SessionStore>;
    pub fn subagents(&self) -> Arc<SubagentStore>;
    pub fn context(&self) -> Arc<ContextService>;
    pub fn permissions(&self) -> Arc<PermissionService>;
    pub fn interrupt(&self) -> Arc<InterruptService>;
}
```

**facade 不再持锁**——所有锁下沉到 service。

## 5. 锁粒度规约（关键约束）

| 规约 | 强制条款 |
|---|---|
| 单锁原则 | 任一 pub fn 同时持有的 Mutex 数 ≤ 1 |
| 闭包模式 | mutate 走 `with_mut` 模式，禁止返回 MutexGuard |
| 锁顺序 | 跨 store 操作严格按字母序：subagents → sessions（不可反向） |
| 闭包内禁止调用其他 service | `SessionStore::with_mut` 的闭包内不允许调 `SubagentStore::*` |
| 阻塞 I/O 禁止持锁 | 文件读写、HTTP 请求必须在锁外做 |

违反任一条 PR 拒收。lint 由 Phase 6.5 加 CI 脚本断言。

## 6. Contract test 法（替代双跑）

facade 拆分**不可双跑**（Mutex 状态独占）。改用 contract test：

### 6.1 编写时机

在动任何代码前，先写 **80+ contract test**，每个 test 是"facade 行为定义"：

```rust
#[test]
fn session_store_insert_then_get_returns_clone() {
    let store = SessionStore::new();
    let record = make_record("s1");
    store.insert("s1".into(), record.clone());
    assert_eq!(store.get_clone("s1"), Some(record));
}

#[test]
fn subagent_store_lock_order_is_subagents_then_sessions() {
    // 用线程模拟并发，确保按规约锁顺序不会死锁
    ...
}
```

每个 pub fn 至少 2 个 contract test（正常路径 + 边界 / 错误）。

### 6.2 测试位置

新建 `crates/runtime/tests/facade_contract/` 目录：

```
tests/facade_contract/
  session_store.rs      (~20 tests)
  subagent_store.rs     (~15 tests)
  context_service.rs    (~10 tests)
  permission_service.rs (~10 tests)
  interrupt_service.rs  (~5 tests)
  facade_thin.rs        (~20 tests, 集成层)
```

### 6.3 执行顺序

1. 写 contract test（**先红再绿**）
2. 每抽出一个 service，跑全套 contract test
3. 任何 test 红 → 暂停 PR，分析是 contract 错还是实现错

## 7. PR 拆解

| 顺序 | PR | 说明 |
|---|---|---|
| 6.1.a | 新建 `tests/facade_contract/` + 80+ contract test（全红，待实现） | 1 PR |
| 6.1.b | 抽 `SessionStore`（facade 改用） | 1 PR |
| 6.1.c | 抽 `SubagentStore` | 1 PR |
| 6.1.d | 抽 `ContextService` + 接入 ConversationHistory | 1 PR（同时解锁 doc39 Phase 3） |
| 6.1.e | 抽 `PermissionService`（依赖 Phase 5 完成） | 1 PR |
| 6.1.f | 抽 `InterruptService` | 1 PR |
| 6.1.g | `RuntimeFacade` 收缩到 ≤ 800 行 + 锁粒度 lint 接入 | 1 PR |

7 个 PR，2-3 周。每个 PR 完成时 contract test 100% 绿；任何 PR 后 `cargo test --workspace` 必须仍全绿。

## 8. 验收

| 指标 | 目标 |
|---|---|
| `wc -l runtime_facade.rs` | ≤ 800 |
| Facade `Mutex<*>` 数 | 0（全部下沉到 service） |
| 80+ contract test 通过率 | 100% |
| 锁顺序 lint | 通过 |
| `cargo test --workspace` | 全绿 |
| `local_api_server.rs` 改动量 | ≤ 200 行（外部 consumer 受影响小） |

## 9. 不在 Phase 6 范围

- 不动 `local_api_server.rs` 内部结构（仅改 import + service 调用）
- 不动 `AgentKernel` 内部
- 不引入新 trait 抽象（5 个 service 直接 struct，不 trait 化——可测试性靠 contract test 而不是 mock）
- 不引入 async（Phase 8 的事）
- 不动 `RuntimeSessionRecord` / `SubagentSession` / `AgentSession` 字段

## 10. 必须停下来报告

1. 某个 pub fn 跨 2+ 个 service 共享状态、无法干净分配
2. contract test 写到一半发现两个 pub fn 实际语义冲突
3. `local_api_server.rs` 的 import 改动超过 200 行
4. 锁顺序规约与现有调用链冲突，无法不死锁
