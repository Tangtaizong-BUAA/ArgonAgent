# Codex 任务指令 · Phase 4 → Phase 8 一次性执行

> 起草：Opus 2026-05-26
> 范围：架构整合计划 Phase 4-8（不含 8.7、8.8、Phase 7.2.a 的 Compactor 扩展决策、Phase 5 shadow 14 天判定——这 4 处必须由 Opus 决定）

---

## 0. 必读文档（按读取顺序）

1. **`docs/architecture_consolidation_plan.md`** — 总体执行计划，本次范围 Phase 4 → Phase 8
2. **`docs/decisions/D1-tauri-only.md` / D2-tokio-transport.md / D3-kernel-rename.md`** — 上游不可违反的决策
3. **`docs/architecture/phase4_deepseek_consolidation.md`** — Phase 4 契约（你的执行手册）
4. **`docs/architecture/phase5_permission_contract.md`** — Phase 5 契约
5. **`docs/architecture/phase6_runtime_facade_decomposition.md`** — Phase 6 契约
6. **`docs/architecture/phase7_router_compactor_integration.md`** — Phase 7 契约
7. **`docs/architecture/phase8_transport_async_migration.md`** — Phase 8 契约
8. **`docs/architecture/native_agent_loop_module_api.md`** — Phase 3.1 边界规范（仍在生效，Phase 4+ 的所有 sibling 改动必须服从）
9. **`docs/doc39_implementation_gap_analysis.md`** — 历史 baseline

读 Phase X 契约时，**先读对应 phase 的"§必须停下来报告"节**——这是你触发暂停的判定基准。

---

## 1. 工程纪律（不变，全部生效）

照搬 `docs/architecture_consolidation_plan.md §0`。**没有任一条款被放松**。本批次特别强调：

- **不混合 PR**：本批次 30+ PR，每个 PR 单一 phase 内单一子任务
- **不偷渡依赖**：Phase 8.1 是**唯一**被允许引入新依赖的 PR；其他 PR 引入新依赖一律拒收
- **双跑替换**：Phase 5、Phase 8 是真双跑（事件流对比）；Phase 4、Phase 6 是 contract test 法（不可双跑）；Phase 7 是 metric watch
- **可回滚**：每个 PR 必须能 `git revert <sha>` 单独回滚不破坏后续 PR
- **boundary 验收**：每个 PR 描述附带"前置 grep 证据"（参考你 Phase 1 的成功模式）

---

## 2. 总 PR 顺序与依赖图

```
Phase 4 (10 PR, 1-2 周)         ← 你立即开始
   └── Phase 6.0 已 done by Phase 1/3
   └── Phase 5 (8 PR, 2 周)     ← Phase 4 完成后启动
       └── Phase 6 (7 PR, 2-3 周) ← Phase 5 完成后启动
           ├── Phase 7 (7 PR, 1 周) ← Phase 6.1.d (ContextService) 后启动
           └── Phase 8 (8 PR, 3-4 周) ← Phase 6.1.f (InterruptService) 后启动
                                           Phase 7 与 Phase 8 可并行
```

如果你在某 phase 卡住超 2 天，跳过未阻塞的子任务，向 Opus 报告，等待裁决。

---

## 3. 各 Phase 执行约束（摘要 + 强调）

### Phase 4 · DeepSeek 收口（10 PR）

按 [契约 §7](phase4_deepseek_consolidation.md) 顺序执行。**禁止**：
- 跨文件批量合并（一次合一个顶层 deepseek_*.rs）
- 未完成旧路径 grep 0 + workspace check/test 前删除 `#[deprecated]` wrapper
- 改任何 fn 签名（只改物理位置 + import 路径）

特别注意：[§5 Qwen 偷渡](phase4_deepseek_consolidation.md) ——本 phase 不解决，但 PR 描述附 qwen ↔ deepseek import 清单作为 Phase 9 资产。

### Phase 5 · 权限五归一（8 PR）

按 [契约 §5](phase5_permission_contract.md) 顺序执行。**核心风险点**：

- **5.4（tool_execution 接入 PermissionGate）是高危 PR**——主路径改动。先写 contract test 覆盖每个 ToolExecutionRequest 形态的预期 PermissionDecision，再动代码。
- **5.5（shadow_compare）必须先于 5.6（删 legacy）至少 14 天**。Codex 不要自行判定 14 天通过——`equal=false` 计数报告交 Opus，由 Opus 决定能否进 5.6。
- 不动 `agent_kernel::PermissionMode` 的 5 个 variant 定义。
- 不引入 LLM-based Ask UI。

### Phase 6 · runtime_facade 拆分（7 PR）

按 [契约 §7](phase6_runtime_facade_decomposition.md) 顺序。**核心要求**：

- **PR 6.1.a 必须先做**：80+ contract test 全写完（红的）才能动 facade 代码
- **锁粒度规约**（[契约 §5](phase6_runtime_facade_decomposition.md)）违反一条 PR 拒收
- service 抽出顺序固定：SessionStore → SubagentStore → ContextService → PermissionService → InterruptService → 最后 facade 收缩
- ContextService（6.1.d）顺带接入 `agent_kernel::conversation_history::conversation_messages_from_event_log`——这是 doc39 Phase 3 一并解锁的副产品
- 不引入 trait 抽象（service 都是 concrete struct）

### Phase 7 · TurnRouter + Compactor（7 PR）

按 [契约 §4](phase7_router_compactor_integration.md) 顺序。**特别约束**：

- **PR 7.2.a 必须先停下来报告**——你的任务是核验 `Compactor::compact` 当前是否真 mutate `&mut EventLog`。如果不是（即仅生成 summary），停下来等 Opus 给扩展契约。**不要自己加 mutate 逻辑**。
- TurnRouter 接入：删除 25 个关键词 fn 时，PR 描述列出每个删除 fn 的"被替换 TurnRoute 分支"证据
- 假事件清理：[契约 §3.5](phase7_router_compactor_integration.md) 的 4 个事件命名表是强制规范，新增 `compaction.skipped` variant

### Phase 8 · transport 异步化（8 PR，Codex 只做 8.1-8.6）

按 [契约 §6](phase8_transport_async_migration.md) 顺序。**核心约束**：

- **PR 8.1 是不可逆点**——动 Cargo.toml 之前确认 D2 决策仍生效（grep `D2-tokio-transport.md` 状态字段必须是"已决定"）
- **PR 8.1 严禁夹带 .rs 改动**——0 字节 .rs diff
- **PR 8.3 分 3 子 PR**：DeepSeek（先）/ Anthropic / OpenAI（各独立 PR），按契约 §3.3 表格分派
- **PR 8.5 dual-run 后 Codex 停下**——14 天 metric 报告交 Opus 决定是否进 8.7
- **安全约束**（[契约 §5](phase8_transport_async_migration.md)）：API key 永不进 event_log / detail_json / log。**每个 PR 描述必须附 4 项安全审计 checklist**

### 不在你范围内（4 项 Opus-only 决策）

| 任务 | 触发条件 | 你做什么 |
|---|---|---|
| Phase 5 shadow 14 天判定 | 5.5 完成 + 14 天 metric 出来 | 报告 `equal=false` 计数，等 Opus 决定能否进 5.6 |
| Phase 7.2.a Compactor 扩展契约 | 你核验 `compact()` 当前不 mutate | 停下报告，等 Opus 给扩展契约 |
| Phase 8.7 默认 transport 切换 | 8.6 完成 + 14 天 dual-run 出 metric | 报告 dual-run diff_report，等 Opus 决定 |
| Phase 8.8 sidecar 删除 | 8.7 决定后 90 天 | 不在本批次范围 |

---

## 4. 累积"必须停下来报告"清单（共 13 条）

历史 9 条（保持）+ Phase 4-8 新增 4 条：

**历史（不变）**：

1. ~~任何"死代码"判定，候选符号被构造/赋值——停~~ ✅ 已成功触发 1 次
2. ~~测试/fixture 文件被 PROD 路径 import——停~~ ✅ 已成功触发 1 次
3. 任何调用方依赖某 deprecated wrapper 的精确 enum/struct 形状
4. cli/desktop 内部某 use 路径被多次重写仍触发 unresolved import
5. `cargo test --workspace` 在某 PR 后出现新红
6. 需要新增任何外部依赖（除 Phase 8.1）
7. 发现 D1/D2/D3 任一决策与现状不符
8. P3.1 的 layer/可见性规范违规无法通过下沉/升层解决
9. cli/main.rs 中非 dev 子命令的代码被波及

**Phase 4-8 新增**：

10. **Phase 4**：合并 deepseek_*.rs 时发现某顶层符号同时被外部 + native_profile 内部用，且语义不同（即不是简单移动）
11. **Phase 5**：shadow_compare 14 天内出现 `equal=false` 且无法解释（非顺序差异、非 enum variant 差异）
12. **Phase 6**：facade 某 pub fn 跨 2+ service 共享 mutable 状态，无法干净分配
13. **Phase 7**：`Compactor::compact` 当前不真 mutate EventLog；或删关键词后某真实用户用例（不是测试）行为改变
14. **Phase 8**：API key 出现在任何事件/log/detail 字段（哪怕脱敏过）——立即停 PR，Opus 现场 review

---

## 5. 路标：Codex 停下报告的 4 个固定时点

| 时点 | 报告内容 |
|---|---|
| Phase 4 全部 10 PR 完成 | 顶层 deepseek_*.rs 已删 + cargo workspace 通过 + qwen ↔ deepseek import 清单 |
| Phase 5.5 完成 + 14 天等待 | shadow_compare `equal=false` 累计计数 + 抽样 diff cases |
| Phase 7.2.a 之前 | `Compactor::compact` 当前实现是否真 mutate EventLog 的 grep + 文件展示 |
| Phase 8.6 完成 + 14 天等待 | dual_run_diff_report.sh 的输出 + 安全审计 4 项 checklist |

每个时点 Codex **停**，把现场（命令、输出、文件路径、行号）发回，等 Opus 裁决再继续。

---

## 6. PR 描述模板（强化版）

```markdown
## Phase X.Y · <title>

### 上游契约
- [phaseN_*.md](../docs/architecture/phaseN_*.md) §Z

### 前置 grep 证据（boundary 验收）
- <对每个待动/待删/待移项的 grep cmd + 结果 + 结论>

### 变更摘要
- 删除：N 行
- 新增：M 行（仅结构性 / 仅 contract test / 仅 import 重写）
- 文件移动：K 个
- 新依赖：0（除 Phase 8.1）

### 验收证据
- `cargo build --workspace`: <output>
- `cargo test --workspace`: <output>
- 契约 §验收指标对照: <逐项达成情况>
- (Phase 3 lint script 若适用): scripts/lint_native_loop_boundary.sh: passed
- (Phase 8 适用): 安全审计 4 项 checklist：<逐项 PASS/FAIL>

### 决策回顾
- 是否违反任何 §0 纪律：否
- 是否触及 §4 "必须停下来"清单 13 条：否（或：是，触发第 X 条→已停下报告）
- 上游 decision 引用：D1/D2/D3 中相关条目

### 回滚预案
`git revert <sha>` 即可，无连带影响。
```

---

## 7. 致 Codex

Phase 1-3 你建立了"grep 验证 → 发现冲突 → 停下报告"的工作方法。Phase 4-8 PR 数量更多、改动面更大，这个方法不能松。

最重要的是：**没有什么 PR 是"显而易见安全的"**。Phase 1 的死代码删除显得简单——你仍然花时间 grep，发现了 1751 行的 NativeLoopTurnController；Phase 2 的 fixture 移动看似机械——你仍然 grep，发现了 cli/main.rs 顶层 import。这个习惯救了项目两次。

Phase 4-8 的隐藏陷阱预测：
- Phase 4：某顶层 deepseek_* 函数可能既被外部 import，又被 native_profile 内部 import，但**语义不同**（不是简单 re-export 能覆盖的）
- Phase 5：旧 `permission_resolver::evaluate_permission_request` 的某调用方可能依赖某 enum variant 顺序（match 顺序敏感）
- Phase 6：facade 内部可能有"隐式共享"的 file_state cache（在 sessions/subagents 之外）
- Phase 7：删除关键词 fn 后，某测试可能依赖关键词触发的 side-effect
- Phase 8：reqwest path 在国内网络可能 SSL handshake 与 sidecar 行为不同

预先警觉、grep 验证、停下报告。

开工。

---

## 8. 不在本批次的后续工作

- Phase 9（Desktop 单一路径）—— 等 Phase 8 完成后启动
- Phase 10（kernel rename）—— 等 Phase 5/6 完成后由 Opus 评估前置条件
- Phase 2.b（runtime fixture 集群）—— 你之前已知，Opus 在 Phase 3 完成后裁决，但仍未给指令；Phase 4 期间 Opus 会决定，但不阻塞 Phase 4

如果完成 Phase 4-8 还有余力，**停下来汇报**，等 Opus 安排下一批。
