# doc39 落地差距完整分析

> 基于 8 个子代理的逐代码深度分析，2026-05-16

---

## 总体诊断

| 指标 | 数值 |
|---|---|
| 新代码总量 (agent_kernel + native_profile) | **4,628 行** |
| 主循环实际接入 | **~600 行 (13%)** |
| 死代码 (写了但未接入) | **~3,600 行 (78%)** |
| 重复代码 (新旧文件内容相同) | **~600 行 (13%)** |
| 主循环行数 (native_agent_loop.rs) | **9,200 行 (目标 <400)** |
| tcml/ 目录 | **不存在 (0/5 文件)** |
| Eval Gates 通过 | **2/10 (R2, R3)** |
| 公开符号被主循环 import | **8/50+ (16%)** |

**核心结论**：doc39 的"骨架"搭了（目录 + 类型定义），但"血肉"几乎全在旧单体内。执行偏差——Phase 1 的原则是"搬代码不改行为"，实际变成了"写新代码不删旧代码"。结果形成两套并行系统：旧的在 `native_agent_loop.rs`（9200行）继续运行，新的在 `agent_kernel/` + `native_profile/`（4628行）等待接入。

---

## 一、Phase 1: Foundation Refactor — 完成度 ~25%

### 1.1 agent_kernel/ 逐文件分析

#### mod.rs (57 行)
- 正确 re-export 所有子模块类型
- 导出了远超 Phase 1 规划的 13 个子模块（Phase 1 只需 6 个）

#### kernel.rs (74 行) — 对齐度 10%

| 问题 | 详情 |
|---|---|
| `run_native_turn()` | 不是编排器，只是一行转发到 `run_native_agent_loop_v2_deepseek()` |
| `run_native_turn_with_event_sink()` | 同上，加了一个闭包参数 |
| `prepare_turn()` | 仅设置 TurnState + 调用 `classify_route()`，不接触主循环 |
| `classify_route()` | 只能路由 3/8 个 TurnRoute 变体 (ProjectStatus/DirectAnswer/ReadOnlyExplore)。**永远无法路由到** CodeEdit、DebugFailure、RunTests、LongHorizonTask、Review |

当前 `AgentKernel` 是一个 `()` 单元结构体，无状态，不编排任何东西。

#### turn_state.rs (191 行) — 对齐度 65%

`TurnState` 字段与 doc39 规范的对比：

| doc39 要求 | 现状 |
|---|---|
| session_id, turn_index, started_at | ✅ 存在 |
| route, mode, role, budget | ✅ 存在 |
| iterations, tool_calls_used, tokens_in/out, reasoning_tokens | ✅ 存在 |
| seen_tool_batches, observation_cache, last_tool_batch | ✅ 存在 |
| emitted_event_count | ✅ 存在 |
| reasoning_replay: ReasoningReplayState | ❌ 缺失 |
| stream_state: StreamProcessorState | ❌ 缺失 |
| provider_capabilities: ToolCallingCapabilities | ❌ 缺失 |
| awaiting_user: Option<AwaitingUserRequest> | ❌ 缺失 |

**字段使用率**：11 个字段中仅 5 个在主循环中被实际读取（55% 死字段率）。`session_id`、`turn_index`、`route`、`role`、`mode`、`progress` 被赋值但从未被读取。

`ToolProgressState` 的 plateau 检测逻辑正确，`ToolProgressDecision` 状态机有测试覆盖。但主循环使用自己的散落变量替代 `ToolProgressState`。

#### budget_policy.rs (33 行) — 对齐度 60%

- `should_compact()` 逻辑简单：`tokens_in > 192_000`，与 doc39 一致
- 缺少路由感知预算（不同 TurnRoute 应有不同预算限制）
- 未被主循环使用——`context_budget::allocate_native_context_budget()` 是独立的

#### permission_policy.rs (106 行) — 对齐度 90%（实现）/ 0%（接入）

5 模式 `evaluate()` 逻辑完全符合 doc39 §3.4：

| 模式 | 逻辑 | 正确性 |
|---|---|---|
| BypassPermissions | 全部 Allow | ✅ |
| Plan | 状态变更 → Deny | ✅ |
| AcceptEdits | 文件编辑 → Allow，其他状态变更 → Ask | ✅ |
| DontAsk | 只读 → Allow，其他 → Deny | ✅ |
| Default | 状态变更 → Ask，其他 → Allow | ✅ |

**关键问题：`PermissionPolicy::evaluate()` 从未被调用。** 只在 `permission_resolver.rs` 的一个注释中被引用。主循环使用自己的 `NativeAgentPermissionMode`（3 变体）和旧的 `permission_policy::PermissionPolicy`（372行）。

#### compactor.rs (436 行) — 对齐度 30%

Phase 1 要求 stub，实际实现了完整的 436 行。但有关键缺陷：

- `compact()` 方法是**只读**的——生成 `CompactionSummary` 但**不修改 EventLog**
- `preserve_latest_reasoning: true` 字段已定义但**从未在 compact() 中使用**
- 无 `ReasoningReplayManager.compact_old_reasoning` 连接
- 主循环从不调用——零外部引用
- 看起来可用但并不真的能工作——对维护者是陷阱

#### turn_controller.rs (476 行) — 对齐度 0%

**与 `native_turn_controller.rs` 逐字节完全相同。** 所有调用方仍通过 `use crate::native_turn_controller::*` 导入旧文件。`agent_kernel/turn_controller.rs` 是死代码。

关键 bug：`turn_id` 硬编码为 `format!("{session_id}:native_turn_0")`，`turn_index` 永远为 0。

#### conversation_history.rs (431 行) — 对齐度 85%（实现）/ 0%（接入）

实现质量高：
- `conversation_messages_from_event_log()` 正确处理 `model.stream_delta`、`tool.call.assembled`、`tool.call_requested`、`tool.result_recorded`
- `provider_tool_call_id` 优先级处理正确
- `conversation_messages_to_openai_json()` 生成正确的 OpenAI 格式

**零外部调用。** `runtime_facade` 的 `build_context_bundle` 仍自己拼接 context 字符串。

#### observation_cache.rs (145 行) — 对齐度 90%，已接入

- 为 6 个只读工具生成去重 key
- 已在主循环中使用（5 处引用点）
- 缺少：新只读工具的 `_=> None` 通配分支会静默泄漏重复

#### provider_capability.rs (718 行) — 对齐度 90%/0%接入

实现了 B7（strict mode 探测）和 B8（跨 provider capability 探测）。离线探测 + 文件缓存 + TTL 24h 机制完整。有 8 个测试。

**零外部调用。** 718 行全死。

#### telemetry.rs (214 行) — 对齐度 40%/0%接入

`aggregate_from()` 是正确的事件日志扫描器，但仅在自己的测试中被调用。`summary_line()` 输出格式化正确。

doc39 §19 的 15 个指标中仅 4 个有对应字段。

#### tool_inventory.rs (129 行) — 对齐度 85%，已接入

主循环 import 了 4 个函数（`should_finalize_tool_inventory`、`tool_inventory_gated_attempt_count`、`tool_inventory_observation_count`、`tool_inventory_summary_message`）。正确分类只读 vs 门控工具。

#### tool_argument_policy.rs (172 行) — 对齐度 80%/0%接入

`replay_mode_for_tool()` 正确返回 `Full`（只读）或 `SummaryOnly`（副作用）。`safe_side_effect_argument_summary_json()` 正确脱敏 content/command 字段。

**零外部调用。**

#### write_constraints.rs (107 行) — 对齐度 80%，已接入

`validate_file_write_line_count()` 和 `requested_line_count_policy()` 在主循环的生产代码中使用。是唯一完全接入的 agent_kernel 模块。

### 1.2 native_profile/ 逐文件分析

#### mod.rs (70 行) — 对齐度 80%/0%接入

`NativeProfile` trait + `NativeProfileInstance` 枚举 + `profile_for_family()` 工厂函数定义完整，有测试。

**零 production 调用。** `profile_for_family()` 仅在自己的测试中使用。主循环直接实例化 adapter，完全绕过工厂。

#### deepseek/stream.rs (341 行) — 对齐度 95%，已接入

`DsmlChunkFilter` 和 `StreamingToolCallAssembler` 均已在主循环中使用。split-marker 跨 chunk 测试通过。DSML 开始/结束标签检测正确。

#### deepseek/reasoning.rs (122 行) — 对齐度 90%，已接入

`ReasoningReplayManager` 通过 `deepseek_adaptation` 接入主循环。`capture()`、`latest()`、`inject()` 完整。`compact_old_reasoning()` 已实现但未被 compaction 触发（因 Compactor 未接入）。

#### deepseek/cache_prefix.rs (144 行) — 对齐度 85%/0%接入

`ThreeZonePrompt` + `CachePrefixPolicy` + `deepseek_cache_zones()` + `deepseek_system_prompt()` 全部实现，排序稳定测试通过。

**零外部调用。** `prompt_assembler.rs` 有自己的 `deepseek_system_prompt()`（`format!()` 拼接），不使用这个版本。

#### deepseek/role_split.rs (79 行) — 对齐度 90%/0%接入

`RoleSplit`（Executor→Pro, Compactor→Flash, Reviewer→Pro, Titler→Flash, Summarizer→Flash）+ `TemperatureSchedule`（Routing=0.0, Executing=0.2, NarrativeAnswer=0.7 等）完整。

**零外部调用。** 主循环的 temperature 始终为 `None`（使用 provider 默认值），model 选择硬编码不按角色分发。

#### deepseek/thinking.rs (423 行) — 对齐度 95%/0%接入

`ThinkingChain` 状态机（Idle→Streaming→Completed）+ `ThinkingChainEvent`（Started/Delta/Completed）+ TUI 显示格式化 + `serde_json_string_literal()`。12 个测试全部通过。

**零外部调用。** 主循环用原始的 `LiveHttpStreamEvent::ThinkingDelta` + 手动脉冲计数处理 thinking。

#### deepseek/budget.rs (43 行) — 对齐度 50%/0%接入

**与 `context_budget.rs` 中的私有函数完全重复。** 主循环使用 context_budget 版本。native_profile 版本是死代码。

#### qwen/mod.rs (18 行) — 对齐度 10%

仅实现 `family()` 和 `profile_name()`。`supports_reasoning_replay()` 走 trait 默认值 `false`。

无 chat-template 探测。无 stream/reasoning/cache_prefix/role_split 子模块。

#### qwen/budget.rs (72 行) — 对齐度 50%/0%接入

与 `context_budget.rs` 中的私有函数完全相同。死代码。

### 1.3 tcml/ 目录 — 0%

**目录不存在。** 5 个文件全部缺失：

| 文件 | 应从何处提取 | 提取难度 |
|---|---|---|
| alias_registry.rs | `tool_call_parser.rs::normalize_tool_id()` (第 294-381 行) | 低 |
| repair_catalog.rs | `tool_contract.rs::apply_low_risk_repairs()` (第 768-928 行) | 中 |
| relational_resolver.rs | `tool_contract.rs` 偏移/limit 默认值 (802-900行) + `native_agent_loop.rs` base_hash 注入 (6222行) | 中 |
| content_extractor.rs | `tool_contract.rs::extract_content_tool_call_candidates()` (1 行包装) | 低 |
| error_factory.rs | `tool_contract.rs::ModelReadableToolError` (第 43-49 行) + `native_agent_loop.rs` 中分散的错误构造 | 低 |

当前别名覆盖约 80 个唯一形式（`normalize_tool_id()` 中约 55 个 + `provider_aliases` 中约 55 个），但无 `AliasRegistry` 结构体。JSON 修复仅覆盖 2/5 类错误。

### 1.4 单体拆分状态

native_agent_loop.rs 的 9200 行按功能分布：

| 行号范围 | 内容 | 估计行数 | 目标文件 |
|---|---|---|---|
| 221-1051 | V1 循环 + 脚本夹具 | ~830 | **删除** |
| 1148-1161 | V2 入口函数 | 14 | kernel.rs |
| 1163-1476 | 流事件处理 | ~310 | turn_controller.rs + stream.rs |
| 1478-3304 | **核心循环** | 1,827 | kernel.rs (重构为 AgentKernel::execute_turn) |
| 3322-3485 | 外部决策恢复 | ~160 | **删除** |
| 3589-4376 | 工具执行/错误收集 | ~790 | 拆分到 tcml/ + turn_controller |
| 4378-4621 | FastAutoWrite | ~240 | kernel.rs |
| 4622-4950 | Manifest/系统提示/约束 | ~330 | kernel.rs + cache_prefix |
| 5017-5568 | 流式工具 + 请求构建 | ~550 | tcml/ + stream.rs |
| 5691-6868 | 最终化/JSON/Shell 辅助 | ~1,180 | kernel.rs + **部分删除** |
| 6869-9200 | 测试 | ~2,330 | 按功能拆分到各模块 |

---

## 二、Phase 2: PermissionPolicy + Manifest 全开 — 完成度 ~15%

### 2.1 三套权限系统并存

| 系统 | 位置 | 行数 | 使用者 |
|---|---|---|---|
| `agent_kernel::PermissionMode` (5 变体) | agent_kernel/permission_policy.rs | 106 | agent_loop_driver, permission_resolver |
| `runtime::PermissionPolicy` (旧) | permission_policy.rs | 372 | runtime_facade, permission_resolver |
| `NativeAgentPermissionMode` (3 变体) | native_agent_loop.rs | 4 | native_agent_loop.rs 自己 |

三套系统**互不引用**。`agent_kernel::PermissionPolicy::evaluate()` 从未被调用。

### 2.2 Manifest 关键词切除状态

- `deepseek_runtime_tool_exposure_for_prompt()` 仍用 25 个关键词（html/css/javascript/小程序/生成/写入/保存 等）决定 ReadOnly vs FastAutoWrite
- `deepseek_runtime_prompt_wants_generation()` 仍用关键词列表指导 max_tokens 分配
- `build_native_loop_tool_manifest()` 仍根据 exposure 切割 manifest

### 2.3 Finalizer 残余

仍在主循环中活跃：

| 变量 | 行号 | 作用 |
|---|---|---|
| `loop_guard_recovery_count` | 1552 | 重复/交替批计数，≥2 时触发合成工具错误 |
| `max_loop_guard_recoveries` | 1553 | 固定阈值 |
| `non_progress_recovery_count` | 1555 | 非进度迭代计数，≥2 时触发事件 |

---

## 三、Phase 3: ConversationHistory — 完成度 ~35%

### 实现状态

`conversation_messages_from_event_log()` 正确处理：
- `model.stream_delta` → user/assistant content 消息
- `tool.call.assembled` → arguments 缓存
- `tool.call_requested` → assistant with tool_calls
- `tool.result_recorded` → tool role with tool_call_id
- provider_tool_call_id 优先级正确

### 接入缺失

`runtime_facade::build_context_bundle()` 在第 1226-1321 行构建 ContextBundle，但**不调用** `conversation_messages_from_event_log()`。对话历史没有被注入到上下文。

接入点：`build_context_bundle()` 中 `builder.build()` 之前，需要调用 `conversation_messages_from_event_log()` 并将结果注入 ContextBundle。

---

## 四、Phase 4: Compactor — 完成度 ~25%

### 实现状态

`compactor.rs` 436 行看起来完整，但：
- `compact()` 只读——生成摘要但不修改 EventLog
- `preserve_latest_reasoning: true` 字段未使用
- 无 `ReasoningReplayManager.compact_old_reasoning()` 连接
- 零外部调用

### 关键发现：假压缩事件

`guard_native_model_request()` 在 token 超阈值时：
1. 发出 `context.compaction.started` 事件
2. 如果超 target_limit → 发出 `context.compaction.blocked` + 返回 Blocked
3. 如果超阈值但未超 target → 发出 `context.compaction.completed`

**但从不调用 `Compactor::compact()`。** 事件是虚假的——记录了"压缩完成"但从未执行压缩。

### 接入点

需要在发送模型请求之前（第 ~1700 行和 ~2030 行），在 `guard_native_model_request` 之前插入真正的 `compactor.compact()` 调用。

---

## 五、Phase 5: NativeProfile 完整化 — 完成度 ~45%

| 组件 | 实现 | 接入 | 评估 |
|---|---|---|---|
| StreamProcessor (DsmlChunkFilter) | ✅ | ✅ 主循环 | 通过 |
| StreamingToolCallAssembler | ✅ | ✅ 主循环 | 通过 |
| ReasoningReplayManager | ✅ | ✅ deepseek_adaptation | 通过 |
| CachePrefixPolicy / ThreeZonePrompt | ✅ | ❌ prompt_assembler 不用 | 未接入 |
| RoleSplit + TemperatureSchedule | ✅ | ❌ temperature 始终 None | 未接入 |
| ThinkingChain | ✅ (423行) | ❌ 主循环用原始事件 | 未接入 |
| ProviderCapabilityMatrix | ✅ (718行) | ✅ | 通过 |
| DeepSeekProfile | ✅ | ❌ 从未实例化 | 死代码 |
| QwenProfile | ❌ (18行空壳) | ❌ | 未开始 |
| Budget (DeepSeek) | ⚠️ | ❌ 与 context_budget 重复 | 死代码 |
| Budget (Qwen) | ⚠️ | ❌ 与 context_budget 重复 | 死代码 |
| profile_for_family() 工厂 | ✅ | ❌ 零 production 调用 | 死代码 |

---

## 六、Phase 6: ToolResult Format + Error Catalog — 完成度 ~10%

### ToolResult 格式缺失

| 工具 | doc39 要求 | 现状 |
|---|---|---|
| file.read | 带行号输出 | 无行号，content 作为原始字符串嵌入 |
| file.edit | unified diff (base_hash→new_hash) | 只有哈希值，无 diff |
| shell.command | 命令 + exit code + 耗时 + stdout + stderr | 有 exit/stdout/stderr，**缺失耗时** |
| 统一错误格式 | 工具名 · ERROR · ErrorCode + 字段 + Retry with | 只有路径错误走 `structured_tool_error_result` |

无 `ResultFormatter` trait 或任何格式化抽象。

### ModelReadableToolError 缺失

当前只定义 3 种错误码：`UNKNOWN_TOOL`、`MALFORMED_TOOL_JSON`、`SCHEMA_VALIDATION_FAILED`。

缺失：`PERMISSION_DENIED`、`SENSITIVE_PATH`、`PATH_ESCAPES_WORKSPACE`、`TOOL_FAILED`、`TOOL_TIMEOUT` 等。无集中式 `ToolErrorCode` 枚举。

---

## 七、Phase 7: Subagent — 完成度 ~5%

### 关键缺失

| doc39 要求 | 现状 |
|---|---|
| task.dispatch 工具 | **不存在** |
| 子代理 LLM 驱动 | `run_subagent_task()` 不调用 LLM，只执行硬编码的 4 个只读工具 |
| 独立 EventLog/AgentSession | 子代理事件写入父 session |
| write_scope 运行时强制 | 无拦截点 |
| 取消机制 | 只设置 `Cancelled` 标志，无 abort handle |
| worktree 隔离 | 仅验证参数，不调用 `git worktree` |
| Flash 模型 | `model_override` 字段存在但从未使用 |

---

## 八、Phase 8: Telemetry + ToolDoctor — 完成度 ~15%

### 遥测指标覆盖

doc39 §19 的 15 个 DeepSeek 关键指标中，仅 **4/15 有对应字段**：

| 指标 | 状态 |
|---|---|
| deepseek.cache.zone_a_hit_rate | ❌ 只有全局 hit rate |
| deepseek.cache.zone_b_hit_rate | ❌ |
| deepseek.reasoning.tokens_per_turn | ❌ 只有 total |
| deepseek.reasoning.replay_count | ❌ |
| deepseek.dsml.leak_chunks_count | ❌ 只有总事件数 |
| deepseek.tool_call.partial_chunks_avg | ❌ |
| deepseek.tool_call.assembly_latency_ms | ❌ |
| deepseek.alias.resolution_count_by_alias | ✅ `alias_resolutions: HashMap` |
| deepseek.repair.rule_applied_count | ✅ `repair_applications: HashMap` |
| deepseek.compaction.triggers_count | ✅ `compaction_count` |
| deepseek.role_split.executor_calls | ❌ |
| deepseek.role_split.compactor_calls | ✅ |
| deepseek.role_split.flash_savings_estimate_usd | ❌ |

ToolDoctor 只有 3 个最小检查。无 `cache-status`/`alias-stats` 命令。

---

## 九、Eval Gates R1-R10

| Gate | 描述 | 状态 | 详情 |
|---|---|---|---|
| R1 | reasoning_content 在 tool_use 链中回放 | ⚠️ 70% | 机制完整但缺少多轮集成测试 |
| R2 | DSML 跨 chunk 不漏到 visible | ✅ 90% | DsmlChunkFilter 实现 + split-marker 测试 |
| R3 | tool_calls.delta 跨 chunk 累加正确 | ✅ 90% | StreamingToolCallAssembler 实现 + split-JSON 测试 |
| R4 | ContentToolCallExtractor 不误执行 | ❌ 0% | tcml/ 不存在，无标记逻辑 |
| R5 | AliasRegistry 覆盖 50+ 错名 | ❌ 20% | ~80 别名分散在 2 个函数中，无 Registry 结构体 |
| R6 | RepairCatalog 不修 file.write.content | ❌ 30% | 修复规则存在但不完整，无 Catalog |
| R7 | CachePrefixPolicy 排序稳定 | ✅ 70% | ThreeZonePrompt 实现，排序测试通过，但未接入 |
| R8 | RoleSplit Compactor 用 Flash | ❌ 0% | RoleSplit 定义但未接入 compaction |
| R9 | 192K 触发 compaction | ❌ 25% | 阈值定义+测试，但主循环不调用 compactor |
| R10 | base_hash 由 runtime 注入 | ⚠️ 60% | patch.apply 有注入，无统一 TCML 层注入 |

**通过：2/10，部分通过：4/10，未通过：4/10**

---

## 十、全量 Wiring 表

### agent_kernel/ wiring

| 文件 | 行数 | 主循环使用？ | 引用数 | 状态 |
|---|---|---|---|---|
| mod.rs | 57 | — (re-export) | — | 活跃 |
| kernel.rs | 74 | runtime_facade 代理 | 4 | 空壳 |
| turn_state.rs | 191 | 部分 (5/11字段) | 1 实例化 | 55%死字段 |
| budget_policy.rs | 33 | 否 | 0 | **死代码** |
| compactor.rs | 436 | 否 | 0 | **死代码** |
| conversation_history.rs | 431 | 否 | 0 | **死代码** |
| observation_cache.rs | 145 | 是 | ~6 | **活跃** |
| permission_policy.rs | 106 | 部分 (仅 Mode) | Mode 仅 | 70%死代码 |
| provider_capability.rs | 718 | 否 | 0 | **死代码** |
| telemetry.rs | 214 | 否 | 0 | **死代码** |
| tool_argument_policy.rs | 172 | 否 | 0 | **死代码** |
| tool_inventory.rs | 129 | 部分 (4/6 函数) | ~6 | 部分活跃 |
| turn_controller.rs | 476 | 否 (用旧文件) | 0 | **死代码/重复** |
| write_constraints.rs | 107 | 是 | ~5 | **活跃** |

### native_profile/ wiring

| 文件 | 行数 | 主循环使用？ | 状态 |
|---|---|---|---|
| mod.rs | 70 | 否 | **死代码** |
| deepseek/mod.rs | 27 | 否 | **死代码** |
| deepseek/budget.rs | 43 | 否 (重复) | **死代码** |
| deepseek/cache_prefix.rs | 144 | 否 | **死代码** |
| deepseek/reasoning.rs | 122 | 是 | **活跃** |
| deepseek/role_split.rs | 79 | 否 | **死代码** |
| deepseek/stream.rs | 341 | 部分 (3/6) | 部分活跃 |
| deepseek/thinking.rs | 423 | 否 | **死代码** |
| qwen/mod.rs | 18 | 否 | **死代码** |
| qwen/budget.rs | 72 | 否 (重复) | **死代码** |

### 重复代码清单

| 旧文件 | 行数 | 新文件 | 行数 | 关系 |
|---|---|---|---|---|
| `native_turn_controller.rs` | 476 | `agent_kernel/turn_controller.rs` | 476 | **逐字节相同** |
| `context_budget.rs` 内 `deepseek_full_budget` | ~30 | `deepseek/budget.rs` | 43 | **逻辑相同** |
| `context_budget.rs` 内 `qwen_fast/guarded_budget` | ~60 | `qwen/budget.rs` | 72 | **逻辑相同** |
| `prompt_assembler.rs` 内 `deepseek_system_prompt` | ~25 | `cache_prefix.rs::deepseek_system_prompt` | ~55 | **不同实现，都未使用** |

---

## 十一、修复路线图

### 第一批：消除死代码 + 创建 tcml（2-3 周）

这是所有后续工作的前提。**不完成这一步，Phase 2-8 都是空中楼阁。**

1. **删除 `agent_kernel/turn_controller.rs`**——改为将旧 `native_turn_controller.rs` 的内容 re-export 到 agent_kernel 路径，单一真相源
2. **创建 `tcml/` 目录**（5 个文件）：从 `tool_call_parser.rs` 和 `tool_contract.rs` 提取代码
3. **删除 V1 循环残余**（约 1500 行）：`run_native_agent_loop`、`resume_native_agent_loop_after_external_decision`、外部决策包结构
4. **删除重复 budget 文件** 或让 `context_budget.rs` 使用 native_profile 版本
5. **接入 `PermissionPolicy::evaluate()`** 到主循环的 `tool_permission_decision()`（第 6477 行）
6. **删除 `NativeAgentPermissionMode`**，统一用 `agent_kernel::PermissionMode`

### 第二批：接入已实现组件（1-2 周）

7. **接入 Compactor**：在第 1704 和 2040 行 `guard_native_model_request` 之前插入真正的压缩
8. **接入 ConversationHistory**：`build_context_bundle()` 中调用 `conversation_messages_from_event_log()`
9. **接入 CachePrefixPolicy**：`prompt_assembler.rs` 改用 `deepseek_cache_zones()` + `deepseek_system_prompt()`
10. **接入 RoleSplit + TemperatureSchedule**：compaction 用 Flash，temperature 按阶段设置
11. **接入 ThinkingChain**：替换主循环第 1196-1211 行的原始 thinking 处理

### 第三批：功能补全（2-3 周）

12. **Phase 6**：`ResultFormatter` trait + 每个工具的格式化实现 + `ModelReadableToolError` 扩展
13. **Phase 7**：`task.dispatch` 工具 + `run_subagent_task()` LLM 驱动 + 独立 EventLog
14. **Phase 8**：补齐 11 个缺失遥测指标 + ToolDoctor 命令
15. **QwenProfile 落地**：chat-template 探测 + 独立 stream/reasoning/cache_prefix

### 第四批：集成验证（1-2 周）

16. **Eval Gates R1-R10 全部通过**
17. **端到端集成测试**
18. **native_agent_loop.rs 缩减到 <400 行**

---

## 十二、总结

> doc39 写了 4,628 行新代码（agent_kernel + native_profile），但主循环（9,200行）只 import 了 8 个符号。tcml/ 层完全不存在。3,600 行（78%）是死代码。5 个 tcml 文件需要创建。3 套权限系统并存。Phase 1 的真正任务——把单体拆成可独立测试的组件——尚未执行。
>
> 第一要务不是写更多新代码，而是把已写的代码接入主循环，替换旧单体。
