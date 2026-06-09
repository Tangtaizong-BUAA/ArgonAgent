# Doc39 第二轮：从"能用"到"好用"执行计划

> 输入：[`docs/doc39_audit_reports/`](docs/doc39_audit_reports/) 16 份模块审计 + [`docs/runtime/p3_p4_completion_status_2026_05_19.md`](docs/runtime/p3_p4_completion_status_2026_05_19.md)
> 状态背景：P3 AgentKernel 权威边界已落地、P2-B/P2-C 已完成；610 tests passed。**架构层面对齐 doc39 已经基本完成。**
> 本计划锁定的目标：把"产品能跑"提升到"产品好用"。

---

## 0. 总判断

P3 之后架构层的硬骨头已经啃完——AgentKernel 拥有 12 个 service field、权限统一到 PermissionGate、HTTP 重试与瞬态恢复事件就位、ConversationHistory OpenAI JSON 投影上线、640+ 测试通过。

但**审计 06、08、13、14、15、16 集中暴露三类"不好用"病灶**，分别对应你说的三类痛点：

| 你的痛点 | 真正的成因 | 主要报告 |
|---|---|---|
| **消耗 tokens 大** | RoleSplit/Flash 未接（R8 FAIL）、Compactor 不真正释放 EventLog、Zone C 缺工程上下文、温度永远 ≈1.0 → 重试多、history 还是 prompt 段不是 typed message field | 03, 08, 13, 14, 16 |
| **功能缺失** | `task.dispatch` 工具根本不存在、subagent 是硬编 read-only walker、TurnRouter 不存在、ContentToolCallExtractor 不扫描（R4 FAIL，模型 narrative 里写 DSML 静默终止）、5 个 error code 缺失、3 条 repair rule 缺失、Qwen 是 19 行空壳 | 01, 04, 07, 09, 10, 15, 16 |
| **偶发 bug** | 硬编 `:native_turn_0` 导致 turn index 永远不递增、EvidenceLedger 是克隆不共享、3 个最重要工具(file.read/file.edit/shell.command)的 formatter 实现了没接线、ObservationCache 只覆盖 6 个工具、ConversationHistory 投影丢弃 error/permission/compaction 事件 → 模型反复犯同样的错 | 04, 05, 15 |

底层共性问题：**遥测是 100% 死代码**（AgentKernelTelemetry.aggregate_from 在生产 0 调用，17 个 doc39 指标只覆盖 2 个）——所以现在我们其实**无法量化**到底哪里在烧 token、哪里在出 bug。任何"好用度"工作的第一步是先让自己看得见。

---

## 1. 阶段划分（按用户痛点编排，不按 doc39 章节）

| 阶段 | 主题 | 主要报告 | 锁定动作 |
|---|---|---|---|
| **Q0** | 让自己看得见（Observability First） | 06, 16 | AgentKernelTelemetry.aggregate_from 接到每轮结束；补 6 个 P2-C recovery 字段；ToolDoctor 三条命令；17 项 spec 指标补到 ≥10 项 |
| **Q1-A** | Flash 模型路由（Token 大杀器） | 08, 14, 16 | RoleSplit 真正接到 Compactor / Titler / Summarizer 三条 model call 链；R8 从 FAIL 转 PASS |
| **Q1-B** | Cache Zone C 含工程上下文 | 08, 13 | ContextBundle items 进 Zone C；命中率打开成 zone_a / zone_b 单独度量 |
| **Q1-C** | TemperatureSchedule 落地 | 08, 14 | `PlannedModelCall.temperature_milli` 按 RoleStage 注入；tool calling stage ≤ 0.3，narrative 0.7；HTTP body 真的带 temperature |
| **Q1-D** | OpenAI history 升格为 typed field | 04, 13 | 所有 provider 请求构建器接收 `Vec<OpenAIChatMessage>` 字段，而非 prompt section；最后一处文本拼接消失 |
| **Q2-A** | ContentToolCallExtractor 扫描 | 07, 16 | finish_reason=stop 时扫 visible buffer；R4 从 FAIL 转 PASS（阻止"narrative 含 DSML 但静默结束"这类 silent stuck） |
| **Q2-B** | TurnRouter + BudgetPolicy 按 route 分配 | 01 | 真有一个 router 在 9 个 TurnRoute 上落子；BudgetPolicy 不再单 default；CodeEdit / DebugFailure / RunTests / LongHorizonTask 等 6 个 variants 不再装饰 |
| **Q2-C** | `task.dispatch` 工具 + LLM-driven subagent | 15 | tool 进 manifest；subagent 走独立 EventLog + write_scope 真正生效 + 可取消 + Flash 模型 |
| **Q3-A** | turn_id 与 EvidenceLedger 共享 | 05 | 删除 `:native_turn_0` 硬编 fallback；EvidenceLedger 改为 Arc/Mutex 共享而非克隆 |
| **Q3-B** | ConversationHistory 事件覆盖补齐 | 04 | error / permission / subagent / compaction 事件投影为对应 messages；模型能"看见"上一轮为何失败 |
| **Q3-C** | ResultFormatter 接线 + ObservationCache 兜底 | 04, 15 | file.read / file.edit / shell.command 的 spec formatter 接线；ObservationCache 加 fallback key generator 覆盖未知 read-only 工具 |
| **Q3-D** | 错误目录补齐 | 10, 15 | 4 个缺失 error code variant + 4 个缺失 ModelReadableToolError 字段（field_errors / retry_hint / retry_example / counts_against_budget） |
| **Q3-E** | 3 条 repair rule + R10 base_hash 注入 | 10, 16 | parse_stringified_array / wrap_bare_string_to_array / empty_object_to_array；file.write/edit 的 base_hash 真注入 |
| **Q4** | Qwen profile 补齐 | 09 | 至少 stream processor / reasoning replay / cache prefix 三个核心子模块，否则 Qwen 不是产品级备选 |
| **Q5** | 清债 PR | 11, 02, 08, 14 | V1 legacy ~550 行删除；ToolArgumentReplayMode 死代码 128 行删除；ThinkingChain（423 行）走"接入 or 删除"独立审计 PR；deepseek/budget.rs 重复删除 |

**顺序锁定**：Q0 必须先做（没有遥测无法验证 Q1-Q3）；Q1 四件子任务两两可并行；Q2/Q3 串行做，Q3-A 优先（bug 类）；Q4 Q5 在 Q1-Q3 稳定后做。

---

## 2. Q0 让自己看得见（Observability First）

> 没有这一步，后面所有"省 token"的工作都没法验收。

来自 [06_provider_telemetry](06_provider_telemetry.md) 和 [16_telemetry_eval_gates](16_telemetry_eval_gates.md)：

- `AgentKernelTelemetry.aggregate_from()` 在生产代码零调用，仅测试调用过。
- 17 项 doc39 §19 指标：仅 2 项覆盖（alias.resolution / repair.rule_applied），4 项部分，11 项缺失。
- P2-C 上线的 7 个新 recovery 事件类型 + 13 个 `agent.loop_recovery` 发射点，没有任何聚合字段。

### Q0.1 接线 aggregate_from

| 动作 | 位置 |
|---|---|
| 每个 turn 完成时调用 `AgentKernelTelemetry.aggregate_from(&event_log_for_turn)` | `native_agent_loop.rs` 收尾段（参考 11_monomer Lines 2780-2971） |
| Telemetry snapshot 写入 EventLog `agent.telemetry.turn_summary` 事件 | EventLog append |
| 桌面 RightInspector 把这个 snapshot 渲染到"概览"标签 | `desktop/src/components/RightInspector.tsx` |

### Q0.2 补 6 个 recovery 字段

加到 `AgentKernelTelemetry`：
```
recovery_count               // agent.recovery.started / agent.loop_recovery
recovery_success_count       // agent.recovery.completed / model.http_failure_recovery_succeeded
recovery_blocked_count       // agent.recovery.blocked
http_retry_count             // model.http_retry_scheduled
retry_compact_count          // model.retry_compact_context
fast_auto_write_recovery_count  // agent.fast_auto_write.recovery
```

### Q0.3 补关键缺失指标（≥10/17）

| 指标 | 取数 |
|---|---|
| #4 reasoning.replay_count | `ReasoningReplayManager.inject_if_required()` 调用计数 |
| #5 reasoning.replay_size_kb | 注入 message 的 char 数估算 |
| #7 dsml.leak_recovered | DsmlChunkFilter 命中并替换的事件计数 |
| #12 repair.success_rate | repaired 后 re-validate 通过率（依赖 Q3-E 引入 re-validate） |
| #14 compaction.tokens_freed | `(before - after) / 4` 估算（与现有 token 估算口径一致） |
| #15 role_split.executor_calls | Q1-A 落地后自然产生 |
| #17 role_split.flash_savings_estimate_usd | 简单按 (executor_calls * Pro_rate) - (compactor_calls * Flash_rate) 估算 |

### Q0.4 ToolDoctor 三条诊断命令

加到 CLI / 桌面"工具"标签：
- `tool doctor cache-status`：每个 cache zone 的 hash / hit / miss
- `tool doctor alias-stats`：alias_resolutions HashMap dump，找出哪些 alias 最常被触发（暗示模型偏好哪些错误名字）
- `tool doctor repair-stats`：repair_applications HashMap dump + repair_success_rate

### 退出标准

- 每轮 EventLog 必有一条 `agent.telemetry.turn_summary` 事件，包含上述至少 10 个字段。
- 桌面 RightInspector 概览页能看到本会话的 cache hit rate / reasoning replay size / recovery count。
- 在跑一个真实 multi-turn 任务后，能定性说出"这次 200K tokens 主要花在 X 上"。

---

## 3. Q1 Token 效率：四件能直接省钱的事

> Q0 装表后，Q1 的每一项都能在表上看到对比。

### Q1-A RoleSplit 真接到 Compactor / Titler / Summarizer

来自 [08](08_deepseek_cache_role_think.md)、[14](14_profile_rolesplit_integration.md)、[16](16_telemetry_eval_gates.md)：

事实：
- `RoleSplit::deepseek_default()` 把 Executor→`deepseek-chat`、Compactor/Titler/Summarizer→`deepseek-chat-flash`。
- 当前生产**0 处**使用：`PlannedModelCall.role_model_name` 永远 `None`、主循环硬写 `ModelRole::Executor`。
- 压缩调用复用主 endpoint：`build_native_compacted_initial_request(&request.endpoint, ...)`、`build_native_tool_evidence_continuation_request(&request.endpoint, ...)`。
- Titler/Summarizer 完全没生产路径。

动作：

| 步骤 | 位置 |
|---|---|
| `PlannedModelCall` 加 `role: AgentRoleKey` 字段；adapter 把 `role_model_name = role_split.model_for(role)` 写入 | DeepSeek/Qwen/Anthropic 3 个 adapter |
| Compactor 调用前替换 endpoint：`endpoint.with_model(role_split.deepseek_default().model_for(Compactor))` | `build_native_compacted_initial_request` 调用现场 |
| Titler / Summarizer 暂时只接 Compactor → Flash 一条；Titler/Summarizer 留到 Q2-C 真有 subagent 后再接 | — |
| **不引入新 crate**；`role_split.rs` 已经在仓内，直接 import | — |

退出标准：
- R8 eval gate 从 FAIL 转 PASS。
- Q0 表上 `role_split.compactor_calls` > 0 且 `role_split.executor_calls > 0`，比例 > 1%。
- `role_split.flash_savings_estimate_usd` > 0。
- 同一个 fixture 任务跑前后，`compaction.triggers_count` 不变但总账单 token 估算下降 ≥ 30%（Flash vs Pro 价差）。

### Q1-B Cache Zone C 真容工程上下文

来自 [08](08_deepseek_cache_role_think.md) §1、[13](13_context_compaction_integration.md) §1：

事实：
- Zone A（system+tools）和 Zone B（session metadata）已经在用、有 hash、有 cache key。
- Zone C 当前只装 tool catalog。
- `ContextBundleBuilder` 产出的 `AGENTS.md / repo_map / git_status / session_memory / file_state` 是另一个 `<context>` 块，**完全在 cache zone 之外**，所以每轮 cache miss。

动作：

| 步骤 | 位置 |
|---|---|
| `CachePrefixPolicy::build_zones` 增加参数 `context_items: &[ContextItem]` | `native_profile/deepseek/cache_prefix.rs` |
| Zone C 内容 = `tool_catalog + context_items`，按 `(source, stable_key)` 排序保证 hash 稳定 | 同上 |
| 拆 cache zone_a / zone_b 命中率到独立 telemetry 字段（doc39 §19 #1 #2） | `AgentKernelTelemetry` |
| **不要把 conversation history 塞进 Zone C**——它在 Q1-D 升格为 typed field，cache 由 provider 侧处理 | — |

退出标准：
- 单测：同一 session 跨 turn 时 zone_c_hash 稳定（无新 file_state 时）。
- Q0 表上 `cache.zone_a_hit_rate` 和 `cache.zone_b_hit_rate` 分别可读。
- 跨 3 turn 跑同一任务，第二 turn 起 prompt_tokens_cached_hint > 0 且持续 > 60%。

### Q1-C TemperatureSchedule 真注入

来自 [08](08_deepseek_cache_role_think.md) §3、[14](14_profile_rolesplit_integration.md) §3：

事实：
- 3 个 adapter 全设 `temperature_milli: None`。
- HTTP body 不含 temperature → provider 默认 ≈ 1.0。
- 工具调用阶段以 1.0 跑，会产生更多"格式不严"的 tool_call，触发更多 TCML repair 与 retry。
- `RoleSplit.temperatures`（Routing=0.0、Executing=0.2、NarrativeAnswer=0.7）已经定义，0 处读。

动作：

| 步骤 | 位置 |
|---|---|
| `PlannedModelCall.role_stage: RoleStage` 字段引入 | 3 个 adapter |
| `temperature_milli = role_split.temperatures.get(role_stage)` | 同上 |
| HTTP body builder（DeepSeek OpenAI / DeepSeek Anthropic / Qwen 3 处）真带 `"temperature": x.xxx` | model io 层 |
| 工具批阶段强制 ≤ 0.3，narrative finalizer 0.7，其他保留默认 | RoleStage 表 |

退出标准：
- Q0 表上每轮 `model.request` 事件 payload 含 `temperature`。
- 同一 fixture 任务跑前后，`repair.rule_applied_count_by_rule` 下降；retry_count 下降。
- B4 / R4 sensitivity gate 从 FAIL 转 PASS（注意 R4 是 ContentToolCallExtractor，不是这条；这里是 B4）。

### Q1-D OpenAI history 升格为 typed message field

来自 [04](04_conversation_history.md) §"Provider request history is a first-class typed message vector everywhere | PARTIAL"、[13](13_context_compaction_integration.md) §4：

当前是 P2-B 的过渡形态：history 作为 `# Conversation History (OpenAI JSON)` 段塞进 prompt 字符串。功能正确但：
- 仍有一次 JSON 序列化 + 模型再次解析的额外 token 开销。
- 模型有时把这个块当 user content 误回复。
- Provider 侧无法对 history 做 cache（每次嵌在 user prompt 里）。

动作：

| 步骤 | 位置 |
|---|---|
| Provider 请求 builder（DeepSeek OpenAI / DeepSeek Anthropic / Qwen）入参增加 `prior_messages: Vec<OpenAIChatMessage>` | 3 处 builder |
| `RuntimeFacade` 直接传 `conversation_messages_to_openai_json()` 结果作为 `prior_messages`，不再拼到 prompt 字符串 | facade live-loop 入口 |
| 删除"# Conversation History (OpenAI JSON)" 字符串注入路径 | facade prompt 构造 |
| 双跑 diff 一周（feature flag `history_as_typed_field`）：对比同一 session 在 typed-field vs section 两种模式的 prompt_tokens、cache_hits、reply quality | 新增 telemetry `history.shape` |
| 稳定后删 section 路径 | 清债 PR |

退出标准：
- typed field 模式下 prompt_tokens 下降（同一任务可量化）。
- 不再出现"模型把 Conversation History 块当任务回复"的 bug 行为。
- 跨 turn cache hint 命中率上升。

---

## 4. Q2 功能缺失：补三件让模型真"行"的事

### Q2-A ContentToolCallExtractor 扫描（R4）

来自 [07](07_deepseek_stream_reasoning.md) §1 / §Key Gap、[16](16_telemetry_eval_gates.md) R4：

事实：
- `content_extractor.rs` 是 3 行 wrapper，没有 `ExtractedContentCall` struct、没有 `scan()`、没有 `ContentToolCallCandidate` 事件。
- 现象：模型有时把 DSML tool call 写在 **visible content** 里（特别是 narrative 段或 reasoning 收尾），不在 `tool_calls` 字段。当前 stream 处理只压制不扫描 → finish_reason=stop 时没人扫，**任务静默终止**。
- 这就是你说"偶发功能缺失 / 任务卡住不结束"的高频原因之一。

动作：

| 步骤 | 位置 |
|---|---|
| `ContentToolCallCandidate { call: ParsedToolCall, source_span, confidence: f32 }` 真实 struct | `tcml/content_extractor.rs` |
| `scan(&visible_buffer) -> Vec<ContentToolCallCandidate>`：复用 `parse_tool_calls`，confidence 由 marker 完整度评分 | 同上 |
| StreamProcessor 在 `StreamCompleted { finish_reason: "stop" }` 时调 scan，发射 `ContentToolCallCandidate` 事件 | `native_profile/deepseek/stream_processor.rs` |
| 主循环捕获该事件：**不自动执行**——发 `agent.content_tool_call.detected` 事件 + 进 TCML pipeline 评估；若 confidence ≥ 0.8 直接走 TCML，< 0.8 加 hint 让下一轮模型自己再喊一次 | `native_agent_loop_stream.rs` |
| 单测：narrative 末尾含 `<tool_call>...</tool_call>` 的 stream 不静默结束 | 新增 fixture |

退出标准：R4 PASS；该类静默终止 bug 报告消失。

### Q2-B TurnRouter + BudgetPolicy 路由化

来自 [01](01_kernel_core.md) §"No TurnRouter Exists" / §"BudgetPolicy not route-aware"：

事实：
- `TurnRoute` 9 个 variant 是装饰用，从不参与决策。
- `BudgetPolicy` 只有单个 `default_budget`，从不依赖 route。
- 后果：CodeEdit 任务和 ReadOnlyExplore 任务用同一份预算和同一份 prompt scaffold。

动作（保守路线——不引入 LLM 路由调用，避免违反 local-first）：

| 步骤 | 位置 |
|---|---|
| 实现 `TurnRouter::classify(prompt, history_hint) -> TurnRoute`：先用规则——含 `write|edit|patch|create|delete|rename|fix` → `CodeEdit`；含 `test|cargo test|npm test` → `RunTests`；含 `debug|fail|error|panic` → `DebugFailure`；含 `complete|status|完成` → `ProjectStatus`；含 `review` → `Review`；含"long horizon" 提示词或 turn_index ≥ 5 → `LongHorizonTask`；空 → `DirectAnswer`；fallback → `ReadOnlyExplore` | `agent_kernel/turn_router.rs`（新增） |
| `BudgetPolicy.for_route(route)` 返回不同 TurnBudget（CodeEdit 给更多 reasoning + tool budget；ReadOnlyExplore 收紧；LongHorizonTask 抬高 compaction_threshold） | `agent_kernel/budget_policy.rs` |
| AgentKernel.for_request 走 router；TurnState.route 填真值并被 BudgetPolicy 读 | `kernel.rs:229` |
| **LLM 路由作为 P0-A2/Q5 增量**，需要时再加 model_router + RoleStage::Routing；这里先保持 deterministic | — |

退出标准：
- TurnRoute 9 个 variant 都至少被路由产生过（fixture 覆盖）。
- BudgetPolicy 不再是死代码（[01](01_kernel_core.md) §4 当前判定）。
- LongHorizonTask 任务的 compaction_threshold 大于 ReadOnlyExplore（具体值待定，关键是"不同 route 不同预算"）。

### Q2-C `task.dispatch` 工具 + LLM-driven subagent

来自 [15](15_phase6_7_toolresult_subagent.md) §4 / §5 / §6：

事实（坏到没法接受）：
- `task.dispatch` 工具：不在 `core_tool_specs()`、不在 manifest、没有 dispatcher handler、没有 schema。
- `run_subagent_task`：硬编 read-only walker，跑 repo.map + 可选 file.read/search + git.status，**没有 LLM turn**。
- Isolation：独立 EventLog 没、write_scope 不应用（只验证）、不可取消、worktree 是计划态从不执行。

这是真正的"功能缺失"——doc39 §3.5 Phase 7 的 task.dispatch 是 agent 自主性的核心。

动作：

| 步骤 | 位置 |
|---|---|
| 在 `core_tool_specs()` 注册 `task.dispatch` 工具，schema：`prompt: string, write_scope: PathScope (optional), model_role: "compactor"\|"executor"\|"reviewer" (optional)` | `crates/runtime/src/tool_specs.rs`（或对应文件） |
| Manifest 把 `task.dispatch` 暴露给 CodeEdit / FastAutoWrite / ReadOnly 三档（CodeEdit/FastAutoWrite 允许写 scope；ReadOnly 强制 None） | `tcml/manifest.rs` |
| Dispatcher handler `dispatch_subagent_task`：构造独立 AgentSession + 独立 EventLog（**不共享 parent EventLog 句柄**，避免事件污染）；走 Q1-A Flash 模型 | `runtime_facade.rs` 新增 |
| `write_scope` 真生效：subagent PermissionGate 注入额外的 scope check，超出 scope 的 file.write/edit 一律 Deny | `agent_kernel/permission_gate.rs` |
| Abort handle：parent turn 可发 `task.dispatch.cancel(task_id)`；child loop 检查 cancel flag 后立即 finalize | `runtime_facade.rs` |
| 父子事件 merge：child 完成后把 final answer + 关键 tool calls 投影为 1 条 `subagent.completed` 事件 append 到 parent EventLog（不是把所有事件 dump 过去） | `runtime_facade.rs` |
| 单测：Explorer→Worker 切换、write_scope 越权被拒、abort 后 child 在 N ms 内 finalize | 新增 |

退出标准：
- Phase 7 完成度从 5% 抬到 ≥ 60%（task.dispatch 工具存在 + LLM turn + isolation + abort）。
- 桌面端能看到 subagent 子 session 折叠展示。
- 一条 e2e 测试：父任务调 task.dispatch 让 child 修一个小 bug，child 走 Flash 收敛，父继续完成。

---

## 5. Q3 偶发 bug：六件直接消 bug

### Q3-A 删除 `:native_turn_0` 硬编 fallback

来自 [05](05_turn_control.md) §3：

```rust
// native_turn_controller.rs:53
Self::new_with_turn_id(format!("{session_id}:native_turn_0"))
```

若 `session.current_turn_id()` 没值（`begin_interactive_turn` 没调），所有 turn 拿到同一个 ID。这是**事件链断裂**类 bug 的根因——会导致 evidence/observation cache key 跨 turn 错配。

动作：
- 删 fallback；改成 `Result::Err("turn_id missing")`，强制上游必须先 `begin_interactive_turn`。
- 在 RuntimeFacade 入口确保 `begin_interactive_turn` 总是先调（不存在则 spike fail）。
- 单测：跳过 begin_interactive_turn 时立即 panic / Err，不静默 fallback。

### Q3-B EvidenceLedger 共享而非克隆

来自 [05](05_turn_control.md) §4：

> Cloned from kernel at turn start; mutations not reflected back.

后果：跨 turn 看不到上一轮真正的 evidence，convergence 决策基于过期快照。

动作：
- `KernelServices.evidence_ledger: Arc<Mutex<EvidenceLedger>>`（或 `RwLock`）。
- TurnController 持引用而非 clone。
- 单测：turn 2 能读到 turn 1 末尾的 sealed iteration。

### Q3-C ResultFormatter 接线（3 个关键工具）

来自 [15](15_phase6_7_toolresult_subagent.md) §1：

事实：
- file.read / file.edit / shell.command 的 spec formatter 已实现但**未接线**，live 输出还是 inline 文本无 line numbers / 无 diff / 无 elapsed time。
- file.write / file.multi_edit / list_directory / list_tree 已接线。

动作：
- `tool_dispatcher` 或 `tool_executor` 在 tool 完成后调对应 formatter。
- 3 个工具的 fixture 输出比对：含行号 / 含 unified diff / 含 elapsed_ms。
- 模型收到的 tool result 改善，减少"我没看清文件内容再读一遍"的重复读 → 顺带省 token。

### Q3-D ObservationCache 覆盖未知 read-only 工具

来自 [04](04_conversation_history.md) §4：

事实：只 6 个工具有 key generator；其他工具 `observation_key()` 返回 `None`，**完全无 dedup**。

动作：
- 加 fallback key generator：未注册工具的 key = `<tool_id>:<sha256(args_json_canonical)[:16]>`。
- 标记为 `weak_dedup`：fallback key 命中时只发 hint，不抑制执行（避免误伤）。
- 单测：注册 `repo.find_files` 这类未在 6 个 known list 内的工具，连续两次同参不该被强抑制，但 hint 必须出现。

### Q3-E ConversationHistory 事件覆盖补齐

来自 [04](04_conversation_history.md) §3：

> 错误 / 权限 / subagent / compaction 事件被静默丢弃。

后果：模型在 turn 2 看不到 turn 1 为何失败 → 重复犯错。这是"消耗 tokens 大"的隐藏 boss。

动作（按事件类型映射）：

| 事件 | 投影为 |
|---|---|
| `tool.permission.denied` | tool message with content `"PermissionDenied: <reason>"`, `tool_call_id` 绑定原 call |
| `tool.error` | tool message with `error_code` + `short_message` |
| `subagent.completed` | assistant message with `name: "task.dispatch"`，content 为 final answer 摘要 |
| `context.compaction.completed` | 1 条 system note（不是 user/assistant）：`"[compacted at turn N: M turns folded]"` |

退出标准：fixture 跑一个故意失败的 file.write，turn 2 模型不再重复同样的写。

### Q3-F base_hash 注入（R10）

来自 [10](10_tcml_full.md) §RelationalInvariantResolver、[16](16_telemetry_eval_gates.md) R10：

事实：patch.apply 有注入；file.write/edit 没有。

动作：
- `RelationalInvariantResolver` 内增加 `file.write.base_hash` / `file.edit.base_hash` 注入：当 args 含 `path` 且 base_hash 缺失时，runtime 读当前 mtime+sha256(content[:64KB]) 注入。
- TCML pipeline step 5（已存在）真消费这个字段。
- 防止"模型基于过期内容 overwrite"导致的回归 bug。

---

## 6. Q4 Qwen profile 补齐（可选，看产品形态）

来自 [09](09_qwen_profile_factory.md)：

QwenProfile 是 19 行空壳。Qwen 走主循环时直接 import DeepSeek 的 `StreamProcessor` 和 `ReasoningReplayManager`。

如果产品**只主推 DeepSeek**，Q4 可推迟到 Q5 清债阶段（仅做 Qwen feature flag-off）。如果 Qwen 是真备选：

| 步骤 | 位置 |
|---|---|
| Qwen `stream_processor.rs`：Qwen 的 chat-template detection、tool_calls 解析差异（Qwen 用 `<tool_call>...</tool_call>` 而非 DeepSeek 的 DSML） | `native_profile/qwen/` 新增 |
| Qwen `reasoning.rs`：Qwen 的 thinking 标签（如有）解析 | 同上 |
| Qwen `cache_prefix.rs`：根据 Qwen 是否支持 prompt caching 决定 |  |
| `NativeProfile` trait 加 `stream_processor(&self) -> Box<dyn StreamProcessorBackend>` 真实方法（而非现在的 3 个标签函数） | `native_profile/mod.rs` |
| 主循环不再硬 import deepseek::*，改通过 trait 拿 | `native_agent_loop.rs:50-51` |

退出标准：Qwen endpoint 跑同一个 fixture 不 panic、不静默跳过 reasoning、tool 解析通过。

---

## 7. Q5 清债 PR（独立成 PR，**不与功能修复混**）

按"不混合 PR"纪律单独成 PR，便于 revert：

| 项 | 来源 | 行数 |
|---|---|---|
| V1 legacy 删除（`NativeAgentLoopStep` / `NativeAgentLoopRequest` / `run_native_agent_loop` / `resume_native_agent_loop_after_external_decision` 等） | [11](11_monomer_analysis.md) §V1 Legacy | ~550 |
| `ToolArgumentReplayMode` + `replay_mode_for_tool` + `safe_side_effect_argument_summary_json` 死代码 | [02](02_permission_layer.md) §Dead Code、[12](12_permission_crosscutting.md) §Dead Code | ~128 |
| `deepseek/budget.rs` 与 `context_budget.rs` 重复函数（保留 context_budget 单源） | [08](08_deepseek_cache_role_think.md) §5 | ~45 |
| `stream.rs` 中无人构造的 `StreamEvent` / `StreamProcessorState` | [07](07_deepseek_stream_reasoning.md) §Dead Code | 少量 |
| `ThinkingChain`（423 行）：独立审计 PR，二选一：接到 streaming 路径 / 整段删除（Reasoning capture/inject 已经在用 ReasoningReplayManager） | [08](08_deepseek_cache_role_think.md) §4 | 423 |

每项独立 PR，描述列出未来重建路径。**不在 Q1-Q3 功能 PR 中夹带删除动作**。

---

## 8. 不在本计划内的事项（明确不做）

1. **`run_turn` → `execute_turn` 命名重构**（[01](01_kernel_core.md) Recommendation 1）：纯字面 spec 对齐，[P3 状态文档](../runtime/p3_p4_completion_status_2026_05_19.md) 已确认权威边界到位。属于"权威边界验收纪律"反对的"为命名而改"。
2. **`native_agent_loop.rs` 强行下到 400 行**（[11](11_monomer_analysis.md)）：保留权威边界口径，不追求行数。Q5 删完 V1 legacy 自然降 ~550 行就够。
3. **EventLog 物理 truncate**（[03](03_compactor.md)、[13](13_context_compaction_integration.md) "Compactor 修改 EventLog"）：保持 append-only 是审计 trail；Q1-D + compactor projection 已经让模型侧看到的 token 数受控。物理 truncate 收益小、风险大（事件链 hash 验证、replay）。
4. **LLM-driven Compactor role call**（与 Q1-A 不同）：让 Flash 真去做摘要而非规则压缩。需要稳定 model_router 与配额控制，**延后到 Q1-A 完成后**作为独立增量。
5. **新 crate 引入**（如 `jsonschema`）：本计划全程禁止；schema 真要 JSON Schema 全功能，单独审批。

---

## 9. 验收与节奏

| 阶段 | PR 粒度 | 测试要求 | 看板指标 |
|---|---|---|---|
| Q0 | 1 PR | Telemetry round-trip 测试；ToolDoctor 单测 | RightInspector 概览页能读 |
| Q1-A | 1 PR | Compactor 走 Flash 的 fixture；R8 PASS | `role_split.compactor_calls > 0` |
| Q1-B | 1 PR | Zone C 含 context items 时 hash 稳定 | `cache.zone_c_hit_rate > 0` |
| Q1-C | 1 PR | HTTP body 含 temperature | adapter 单测 |
| Q1-D | 1 PR + feature flag 双跑 1 周 | typed-field vs section diff telemetry | `history.shape` 切换无回归 |
| Q2-A | 1 PR | R4 PASS；narrative-含-DSML fixture | 静默终止 bug 消失 |
| Q2-B | 1 PR | 9 个 TurnRoute 都被产生；BudgetPolicy.for_route 单测 | turn_route 分布看板 |
| Q2-C | 1 大 PR（subagent）+ 1 小 PR（task.dispatch tool 注册） | Explorer→Worker、write_scope 越权拒、abort | Phase 7 完成度 ≥ 60% |
| Q3-A~F | 各 1 小 PR | 各自对应单测 | bug 报告下降 |
| Q4 | 大 PR + feature flag | Qwen fixture | 仅 Qwen 启用时进 |
| Q5 | 多个清债 PR（不混合） | cargo check + 现有测试 | 行数下降 |

每个阶段 PR 完成后**回写本文件状态**（加 ✅ 或日期），让本计划随代码漂移。

---

## 10. 三条纪律重申（与 P0-P3 阶段相同）

1. **Local-first**：Q0 遥测、Q1-A RoleSplit、Q2-A 内容扫描、Q2-B TurnRouter 全部不依赖网络；LLM-driven Compactor / LLM router 延后到独立增量。
2. **不混合 PR**：Q5 清债与 Q1-Q3 功能修复永不同 PR。
3. **不偷渡新依赖**：本计划全程不引入新 crate；schema 用 serde_json 解决；retry 已用现成实现；任何"装个库就快了"的提议都按独立审批走。
4. **双跑替换**：Q1-D（history typed field）和未来任何替换权威路径的变更都走 feature flag + diff telemetry 一周。
5. **权威边界 > 行数**：所有"拆解 native_agent_loop"动作以"哪个 service 拥有决策"验收；不以 wc -l 验收。

---

## 11. 与前一份计划的关系

本计划是 [`audit_reports/round1_doc39_alignment/NEXT_EXECUTION_PLAN.md`](../../audit_reports/round1_doc39_alignment/NEXT_EXECUTION_PLAN.md) 的**后继**：
- 前者覆盖 P0-A 上下文投影 / P0-B 任务状态 / P1-A 权限 / P1-B TCML / P2 native profile / P3 AgentKernel 迁移——架构层。
- 本计划在 P3 完成的基础上，转向 **用户体感层**：token、功能、bug、可观测性。
- 共享同一套纪律 [`feedback_execution_plan_discipline.md`](../../.claude/projects/-Users-gongyuxuan-Documents-deep-code/memory/feedback_execution_plan_discipline.md)。
