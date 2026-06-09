# Audit 06: ProviderCapability + Telemetry vs doc39 (Re-audit)

**Date:** 2026-05-19 | **Re-audit reason:** 纳入 P2-C retry/recovery 事件重新计算覆盖率

## Verdict: Telemetry 仍 100% 死代码

`AgentKernelTelemetry` 编译通过、测试通过、被 re-export，但 `aggregate_from()` 在**生产代码中零调用**。

---

## 1. P2-C 新增 Retry/Recovery 事件清单

### 专用 recovery 事件（7 个新类型）

| 事件类型 | 发射位置 | 含义 |
|---|---|---|
| `model.retry_compact_context` | `native_agent_loop.rs:618,768,1084` | 压缩后重试（3 个策略：rebuild_from_compacted / compact_runtime_evidence / rebuild_initial） |
| `model.http_failure_recovery_succeeded` | `native_agent_loop.rs:832` | HTTP 失败后明文证据重试成功 |
| `model.http_retry_scheduled` | `model_io.rs:177,337`; `live_http_transport.rs:299` | 瞬态 HTTP 状态(408/429/5xx)触发重试 |
| `model.retry_requested` | `tcml/contract.rs:286` | 工具中介因未知工具请求重试 |
| `agent.recovery.completed` | `model_io.rs:429`; `live_http_transport.rs:317` | 重试循环成功完成 |
| `agent.recovery.started` | `native_turn_controller.rs:131,248,271` | 恢复循环启动 |
| `agent.recovery.blocked` | `native_agent_loop_resume.rs:53` | 恢复被阻塞 |
| `agent.recovery.escalated` | `native_agent_loop.rs:1999` | 重复工具契约失败后协议降级 |
| `agent.fast_auto_write.recovery` | `native_agent_loop.rs:1173,1448` | Auto-write 恢复 |

### `agent.loop_recovery` 发射点（13 处）

| 文件:行号 | reason |
|---|---|
| `turn_controller.rs:421` | repeated_tool_batch / duplicate_observation_suppression |
| `turn_controller.rs:501` | empty_visible_response |
| `turn_controller.rs:776` | non_progress_iteration |
| `turn_controller.rs:789` | repeated_non_progress |
| `turn_controller.rs:1054` | BatchNoveltyPlateau / soft_warning |
| `turn_controller.rs:1116` | repeated/alternating_tool_batch |
| `native_agent_loop.rs:1958` | tool_not_in_manifest |
| `native_agent_loop.rs:2025` | tool_contract_rejected |
| `native_agent_loop.rs:2371` | write_tool_result_error |
| `native_agent_loop.rs:2542` | tool_result_error |
| `native_agent_loop.rs:2748` | tool_result_error |
| `native_agent_loop_stream.rs:368` | duplicate_observation_suppression |
| `native_agent_loop_completion.rs:115` | synthesized_visible_fallback |

## 2. doc39 §19 指标覆盖（17 项）

**2/17 完全覆盖，4/17 部分覆盖，11/17 缺失。覆盖率 11.8%。**

| # | 指标 | 字段 | 状态 |
|---|---|---|---|
| 1 | cache.zone_a_hit_rate | cache_hits/misses（合并） | PARTIAL |
| 2 | cache.zone_b_hit_rate | cache_hits/misses（合并） | PARTIAL |
| 3 | reasoning.tokens_per_turn | total_reasoning_tokens | PARTIAL |
| 4 | reasoning.replay_count | 无 | **MISSING** |
| 5 | reasoning.replay_size_kb | 无 | **MISSING** |
| 6 | dsml.leak_chunks_count | dsml_leak_events（仅计数） | PARTIAL |
| 7 | dsml.leak_recovered | 无 | **MISSING** |
| 8 | tool_call.partial_chunks_avg | 无 | **MISSING** |
| 9 | tool_call.assembly_latency_ms | 无 | **MISSING** |
| 10 | alias.resolution_count_by_alias | alias_resolutions: HashMap | **COVERED** |
| 11 | repair.rule_applied_count_by_rule | repair_applications: HashMap | **COVERED** |
| 12 | repair.success_rate | 无 | **MISSING** |
| 13 | compaction.triggers_count | compaction_count | PARTIAL |
| 14 | compaction.tokens_freed | 无 | **MISSING** |
| 15 | role_split.executor_calls | 无 | **MISSING** |
| 16 | role_split.compactor_calls | compactor_role_calls | PARTIAL |
| 17 | role_split.flash_savings_estimate_usd | 无（事件 hardcode 0.0） | **MISSING** |

## 3. AgentKernelTelemetry 字段（11 个）

| 字段 | 映射指标 |
|---|---|
| cache_hits | #1, #2（合并） |
| cache_misses | #1, #2（合并） |
| total_reasoning_tokens | #3（部分） |
| dsml_leak_events | #6（部分） |
| alias_resolutions: HashMap | #10（覆盖） |
| repair_applications: HashMap | #11（覆盖） |
| compaction_count | #13（部分） |
| compactor_role_calls | #16（部分） |
| titler_role_calls | 无匹配指标 |
| summarizer_role_calls | 无匹配指标 |
| unknown_tool_names: HashMap | 无匹配指标 |

## 4. P2-C 事件未被遥测覆盖

`AgentKernelTelemetry` 完全没有追踪 recovery 相关内容。缺失：

| 缺失追踪 | 相关事件 |
|---|---|
| recovery_count | agent.recovery.started, agent.loop_recovery |
| recovery_success_count | agent.recovery.completed, model.http_failure_recovery_succeeded |
| recovery_blocked_count | agent.recovery.blocked |
| http_retry_count | model.http_retry_scheduled |
| retry_compact_count | model.retry_compact_context |
| fast_auto_write_recovery_count | agent.fast_auto_write.recovery |

## 5. Production Wiring

- `aggregate_from()`: **仅测试中调用**（telemetry.rs:158,193,200,210,215,225）
- `summary_line()`: **仅测试中调用**
- `cache_hit_rate()`: **仅测试中调用**
- `ErrorRecoveryState`: 已在 `agent_loop_driver.rs` 中接线，但**不记录遥测聚合事件**

## 6. ToolDoctor

- `run_tool_manifest_doctor()` 存在（`tcml/manifest.rs:118`）—— 仅做工具规范一致性检查
- **缺失命令**: cache-status, alias-stats, repair-stats

## 建议

1. 添加 5 个 recovery 字段到 AgentKernelTelemetry
2. `aggregate_from()` 接线到生产轮次结束处
3. 实现 ToolDoctor 诊断命令
