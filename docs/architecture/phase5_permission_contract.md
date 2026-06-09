# Phase 5 · 权限五归一契约

> 状态：**Opus 设计已签**（2026-05-26）
> 上游：[`docs/architecture_consolidation_plan.md` §Phase 5](../architecture_consolidation_plan.md)、记忆 [`project_permission_refactor`]
> 下游：Codex 按本契约执行 5.2-5.5

---

## 1. 目标态（5 → 3 件套）

| 角色 | 物理位置 | 职责 |
|---|---|---|
| **Mode + 评估** | `agent_kernel::PermissionPolicy` | 5 模式 evaluate：BypassPermissions / Plan / AcceptEdits / DontAsk / Default → 返回 `PolicyDecision { Allow / Ask / Deny }` |
| **守门 + 10 步算法** | `agent_kernel::PermissionGate` | 持有 RuleStore + Mode + 路径上下文；调用 PermissionPolicy 后跑 dangerous-path / rule-store / inline-policy / safety-check / denial-tracking 10 步 |
| **规则存储** | `permission_policy::PermissionRuleStore` | TSV 规则文件解析 + 工作区 inline policy（保持现状） |

**删除**：
- `permission.rs`（359 行，`classify_command_with_reasons` 等）—— 其分类逻辑迁入 `PermissionGate::check` 内部 step
- `permission_resolver.rs`（1174 行，`evaluate_permission_request` + `PermissionResolver`）—— 整体退场；当前 `PermissionGate` 内部已经包了 `PermissionResolver`，逻辑下沉到 Gate

## 2. 反转依赖方向

**当前违规**：

```
agent_kernel/permission_policy.rs ──→ permission_resolver.rs (反向：kernel 层依赖 runtime helper)
agent_kernel/permission_gate.rs ──→ permission_resolver.rs (同上)
```

**目标**（合规）：

```
permission_resolver 不存在
agent_kernel/permission_policy.rs    (pure mode evaluation, 0 deps on runtime/*)
agent_kernel/permission_gate.rs ──→ permission_policy (RuleStore)
  └ 内部含原 PermissionResolver 的 10 步算法
runtime 调用方 ──→ agent_kernel::PermissionGate
```

## 3. trait / struct 契约

### 3.1 `PermissionPolicy`（保持现状，0 改动）

```rust
pub enum PermissionMode {
    BypassPermissions, Plan, AcceptEdits, DontAsk, Default,
}

pub enum PolicyDecision { Allow, Ask, Deny { reason: String } }

impl PermissionPolicy {
    pub fn evaluate(
        mode: PermissionMode,
        request: &PermissionRequest<'_>,
    ) -> PolicyDecision;  // 现状已实现 100%
}
```

### 3.2 `PermissionGate`（吸收 PermissionResolver）

```rust
pub struct PermissionGate {
    rule_store: Arc<PermissionRuleStore>,
    inline_policy: PermissionRuleSet,
    mode: PermissionMode,
    workspace_root: String,
    session_id: String,
    // 来自 PermissionResolver 的内部状态：
    denial_count: u32,
    consecutive_denials: u32,
}

impl PermissionGate {
    pub fn new(
        rule_store: Arc<PermissionRuleStore>,
        inline_policy: PermissionRuleSet,
        mode: PermissionMode,
        workspace_root: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self;

    /// 10 步算法（吸收原 PermissionResolver::evaluate_request）：
    /// 1. policy mode evaluate (PermissionPolicy::evaluate)
    /// 2. dangerous path check
    /// 3. rule store (TSV) match
    /// 4. inline policy match
    /// 5. safety check (custom per-tool)
    /// 6. workspace path containment
    /// 7. write_scope enforcement (subagent)
    /// 8. risk classification (kernel ToolRisk)
    /// 9. denial tracking + fallback
    /// 10. final decision
    pub fn check(
        &mut self,
        request: PermissionRequest<'_>,
        tool: &dyn PermissionCheck,
    ) -> PermissionDecision;

    pub fn denial_count(&self) -> u32;
    pub fn consecutive_denials(&self) -> u32;
    pub fn record_denial(&mut self);
    pub fn record_success(&mut self);
    pub fn should_fallback(&self) -> bool;
}

pub enum PermissionDecision {
    Allow,
    AllowWithLog { reason: String },
    Ask { reason: String, suggestion: Option<String> },
    Deny { reason: String, suggestion: Option<String> },
}
```

### 3.3 `PermissionCheck` trait（保持现状，0 改动）

`permission_resolver.rs` 中的 `PermissionCheck` trait + `check_dangerous_path` 等纯函数——**搬到 `permission_policy.rs` 末尾**，因为它们是与 RuleStore 同层级的工具函数。

## 4. 代码搬移路线图

### 5.2 · 合并 `permission_resolver.rs` 进 `agent_kernel/permission_gate.rs`

1. `PermissionResolver` struct 字段（denial_count、consecutive_denials 等）→ `PermissionGate` 字段
2. `evaluate_request` 方法 → `PermissionGate::check` 内部实现
3. `PermissionCheck` trait + `check_dangerous_path` + `PermissionRequest` struct → 搬到 `permission_policy.rs`
4. 旧 `evaluate_permission_request(...)` 自由函数 → 保留为 1 行 wrapper:
   ```rust
   #[deprecated(note = "use agent_kernel::PermissionGate::check")]
   pub fn evaluate_permission_request(...) -> ... {
       // 内部 new PermissionGate + check
   }
   ```
5. 14 天后删除 wrapper

### 5.3 · 删除 `permission.rs`

逐 fn 评估：

| fn | 处置 |
|---|---|
| `classify_command_with_reasons` | 移到 `PermissionGate::check` step 5 (safety check) 内部 |
| `NativeAgentPermissionMode` 等 enum | 删除（与 `agent_kernel::PermissionMode` 重复，统一用后者） |
| 其他 utility | 评估后或删或并入 `permission_policy.rs` |

### 5.4 · 接入 `tool_execution.rs`

当前 `tool_execution.rs:139` 的 `if spec.permission_required` 就地检查替换为：

```rust
let decision = permission_gate.check(request, tool_check);
match decision {
    PermissionDecision::Allow | AllowWithLog { .. } => /* 执行 */,
    PermissionDecision::Ask { .. } => /* 走原 ApplyWithPermission 流程 */,
    PermissionDecision::Deny { .. } => /* 返回 denied result */,
}
```

`PermissionGate` 由 caller（`native_agent_loop_tools.rs` / `AgentKernel`）传入引用。`tool_execution` 自己不持有 Gate 状态。

### 5.5 · 覆盖率矩阵 + sustained green（**2026-05-26 修订**）

#### 修订背景

原契约要求 PermissionGate 内部 shadow_compare 老路径。Codex 在 5.5 启动前正确触发"必须停下来"，指出**原契约自身矛盾**：

- spike/next-step baseline 上 `evaluate_permission_request` **本来就只有 16 行 wrapper**，真逻辑在 `PermissionResolver::evaluate_request` method 里
- 5.2.a 把那个 method 搬入 `PermissionGate`——所以新 = 老 = 同一份代码
- shadow_compare 必然 100% equal，0 reconcile 信息
- 复活"legacy snapshot"等于重新发明一个不存在过的并行实现，伪双跑

**根本原因**：Phase 5 是 refactor 不是 rewrite。Phase 8 的 dual-run 模型（sidecar vs reqwest 真两种实现）不适用 Phase 5（代码搬家不改语义）。

#### 修订后 5.5 流程

放弃 shadow_compare，改用 **Phase 6 同款 contract test 法 + sustained green**：

##### 5.5.a · 覆盖率矩阵证据

5 个 `PermissionMode` × 6 个 `PermissionRequestType` = 30 cells。每个 cell 至少 1 个测试覆盖。

PR 描述附 grep 矩阵：

```bash
for mode in BypassPermissions Plan AcceptEdits DontAsk Default; do
  for ty in WriteFile ExecuteCommand ReadFile NetworkCall PlanApproval AskUser; do
    cnt=$(rg -l "PermissionMode::$mode" crates/runtime/src/ --type rust 2>/dev/null | \
          xargs rg -l "PermissionRequestType::$ty" 2>/dev/null | \
          xargs rg -l "#\[test\]" 2>/dev/null | wc -l)
    echo "$mode × $ty: $cnt test files"
  done
done
```

**目标 ≥ 24/30 cells 覆盖（80%）**。低于阈值则补 contract test 到 `agent_kernel/permission_gate.rs::tests`。

##### 5.5.b · sustained green 证据

Codex 报告 5.3.b 出现"中间一次 local_api_server 并发红"，flakiness 可能掩盖真问题。要求连续 **3 次** `cargo test --workspace` 全绿，对比 spike baseline 0 net new red：

```bash
for i in 1 2 3; do
  echo "=== Run $i ==="
  cargo test --workspace 2>&1 | tail -3
done
```

baseline reference：spike/next-step (commit `134a03ce`) 已知 **10 个 facade tests 在 Opus 本地环境红**（known issue #1，环境敏感非代码 bug，与 Phase 5 无关）。Codex 环境跑全绿。任一方在 5.5 引入 net new red 即视为 regression。

##### 5.5.c · 决策事件 telemetry（永久保留）

`PermissionGate::evaluate` 接入点的 caller 必须写 `permission.decision.recorded` 事件——长期 telemetry，不依赖 shadow：

```rust
session.append_event(KernelEvent {
    event_type: "permission.decision.recorded".into(),
    payload_json: json!({
        "tool_id": ...,
        "mode": format!("{:?}", request.mode),
        "request_type": format!("{:?}", request.request_type),
        "decision": format!("{:?}", decision),
        "denial_count_after": gate.denial_count(),
    }).to_string(),
    ...
});
```

这是未来排查权限误判（用户报告"为什么我被 deny"）的必备 trace，5.6 删 legacy 后仍保留。

### 5.6 · 删除 legacy（取消 14 天等待，5.5 通过即可执行）

修订理由：取消 shadow_compare 后无 14 天 metric，无等待理由。5.5.a + 5.5.b + 5.5.c 全部通过后立即可做。

删除目标：
- `permission.rs` 文件
- `permission_resolver.rs` 中 `evaluate_permission_request` wrapper
- `NativeAgentPermissionMode` 全部引用清零
- 任何 `#[deprecated(since = "phase 5")]` wrapper

## 5. PR 拆解

| 顺序 | PR | 风险 |
|---|---|---|
| 5.2.a | `PermissionGate` 吸收 `PermissionResolver` 内部状态（添加字段、迁移 check 逻辑） | 中 |
| 5.2.b | `PermissionCheck` trait + `PermissionRequest` 搬到 `permission_policy.rs` | 低 |
| 5.2.c | `evaluate_permission_request` 改为 thin wrapper + `#[deprecated]` | 低 |
| 5.3.a | `permission.rs::classify_command_with_reasons` 迁入 Gate step 5 | 中 |
| 5.3.b | 删除 `NativeAgentPermissionMode` 引用，统一用 `agent_kernel::PermissionMode` | 中 |
| 5.4 | `tool_execution.rs:139` 接入 `PermissionGate::check`；caller 传 Gate | **高**（核心路径） |
| 5.5.a | 覆盖率矩阵 grep + 补缺测试 | 低 |
| 5.5.b | 3 次连续 cargo test --workspace 全绿 | 低 |
| 5.5.c | `permission.decision.recorded` telemetry 接入 caller | 低 |
| 5.6 | 5.5 通过即可执行：删 legacy（无 14 天等待） | 低（删除型） |

## 6. 验收

| 指标 | 目标 |
|---|---|
| permission 相关文件总行数 | 2210 → ≤ 800 |
| `rg "permission_resolver::" crates` 命中数 | 0（5.6 完成后） |
| `rg "NativeAgentPermissionMode" crates` 命中数 | 0 |
| `permission.decision.recorded` 事件覆盖每次 Gate 决策 | 1:1 |
| 5.5.a 覆盖率矩阵 | ≥ 24/30 cells |
| 5.5.b 连续 3 次 `cargo test --workspace` | 0 net new red（vs spike baseline） |

## 7. 不在 Phase 5 范围

- 不动 `agent_kernel::PermissionMode` 的 5 个 variant 定义
- 不动 PermissionRuleStore 的 TSV 文件 schema
- 不引入新的 permission 模式
- 不接入 LLM 自然语言权限（Ask 决策的 UI 改进）—— 另开 epic

## 8. 必须停下来报告

1. 5.5.a 覆盖率矩阵 < 24/30 cells，且补 contract test 后仍达不到
2. 5.5.b 3 次跑出现 net new red（相对 spike/next-step baseline 多出来的红测）
3. `PermissionGate::check` 的 10 步算法与原 `PermissionResolver::evaluate_request` 顺序不一致
4. `permission.rs::classify_command_with_reasons` 中有不被现有 step 覆盖的语义（如 special-case env-var 检查）
5. 任何 cli/desktop/test 调用方依赖 `evaluate_permission_request` 的精确 enum 形状
