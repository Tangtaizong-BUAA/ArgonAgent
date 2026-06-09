# `native_agent_loop` 模块边界规范（Phase 3.1 输出）

> 状态：**P3.1 已签订**（2026-05-25）
> 上游决策：[`architecture_consolidation_plan.md` §Phase 3](../architecture_consolidation_plan.md)
> 下游执行：P3.2 由 Codex 按本规范机械重构 `use super::*` → 显式 import；P3.3 由 CI lint 锁死边界

---

## 1. 目标

把 `crates/runtime/src/native_agent_loop.rs` + 11 个 `native_agent_loop_*.rs` 兄弟文件（总 14345 行）从"文本切割"升级为"有强制边界的子模块簇"。

P3.1 不动一行代码，只产出**两份契约**：

1. **可见性契约**：每个 sibling 暴露的 pub 项白名单
2. **依赖契约**：每个 sibling 允许 import 哪些其他 sibling 的项

P3.2 的代码改动必须 100% 落在本规范上；任何偏差需要本文件先 PR 修订。

---

## 2. 三层可见性

放弃当前满地 `pub(super)` 的状态，改为三层：

| 层 | 语法 | 含义 | 适用 |
|---|---|---|---|
| **L1 公共** | `pub` | 整 crate / workspace 可见 | 真正的对外 API，本规范第 4 节穷举 |
| **L2 家族** | `pub(in crate::native_agent_loop)` | 仅 11 个 sibling + 父文件可见 | 跨 sibling 必需的胶水，每个 sibling 自报清单 |
| **L3 私有** | （无修饰） | 仅本 sibling 内部 | 默认；不在 L1/L2 清单上的全部回到 L3 |

**规则**：
- 一个项 **不允许** 同时是 `pub` 和 `pub(super)`/`pub(in ...)`。要么对外、要么对内，二选一。
- L2 清单由本规范 §5 逐 sibling 定义；新增 L2 项必须先改本规范再改代码。
- 现有 `pub(super)` 项若**不在** §5 清单内，P3.2 必须降级为 L3（私有）；若降级会破坏 import，必须先在调用方就地内联或移除。

---

## 3. 依赖层级

11 个 sibling 按严格分层组织，**只能向下依赖**：

```
Layer A · 入口（≤ 2）       entrypoints, resume
                                │
Layer B · 编排（≤ 3）       completion, continuation
                                │
Layer C · 每轮工作（≤ 3）    model_io, tools
                                │
Layer D · 原子操作（≤ 3）    prompt, execution
                                │
Layer Z · 纯函数（1）        util
```

**层级规则**：

1. Layer A 可调用 A 之外的任何层。
2. Layer B 仅可调用 C、D、Z。**禁止** B 调用 A（A 是入口，不应被回调）。
3. Layer C 仅可调用 D、Z。**禁止** C 调用 B 或 A。**禁止 C 内部互相调用**——`tools` 不调 `model_io`，等等。如果有跨 C 需求，把共享逻辑下沉到 D。
4. Layer D 仅可调用 Z。**禁止** D 之间互相调用。
5. Layer Z 仅依赖 std + workspace crate 公共 API + 父文件 `native_agent_loop.rs` 的类型。**禁止** util 调用任何 sibling。
6. 任何层都可以使用父文件 `native_agent_loop.rs` 中定义的**类型与常量**（例如 `NativeAgentLoopV2Request`、`PendingNativeToolExecution`），但**不可** import 父文件的 `fn`——父文件的主循环函数不是被 sibling 调用的接口。

**已知违规**：当前 C 层内互调已经通过 Phase 3.c 收敛；`tools → execution`（D）合法（C→D）。

---

## 4. L1 公共 API 白名单（不可超出）

**整个 `native_agent_loop` 模块对 crate 外的可见接口仅以下条目**：

### 4.1 生产入口（3 函数，定义在 `entrypoints`）

```rust
pub fn run_native_agent_loop_v2_deepseek<T: LiveHttpTransport>(
    transport: T,
    request: NativeAgentLoopV2Request,
) -> Result<NativeAgentLoopResult, String>;

pub fn run_native_agent_loop_v2_deepseek_with_event_sink<T: LiveHttpTransport>(
    transport: T,
    request: NativeAgentLoopV2Request,
    event_sink: Box<dyn FnMut(&KernelEvent) + Send>,
) -> Result<NativeAgentLoopResult, String>;

pub fn run_native_agent_loop_v2_deepseek_with_interrupt<T: LiveHttpTransport>(
    transport: T,
    request: NativeAgentLoopV2Request,
    interrupt: &AtomicBool,
) -> Result<NativeAgentLoopResult, String>;
```

### 4.2 Resume 入口（1 函数，定义在 `resume`）

```rust
pub fn resume_native_agent_loop_after_external_decision<T: LiveHttpTransport>(
    transport: T,
    request: NativeAgentLoopResumeRequest,
) -> Result<NativeAgentLoopResult, String>;
```

### 4.3 数据类型（5 个，定义在父文件 `native_agent_loop.rs`）

```rust
pub struct NativeAgentLoopV2Request { ... }
pub struct NativeAgentLoopResult { ... }
pub enum NativeAgentLoopStatus { ... }
pub enum NativeAgentToolExposure { ... }
pub struct PendingNativeToolExecution { ... }
```

### 4.4 Resume 配套类型（按需保留）

```rust
pub struct NativeAgentPermissionDecision { ... }
pub struct NativeAgentLoopResumeRequest { ... }
pub struct NativeAgentLoopV2ResumeRequest { ... }  // 与上者关系待 P3.2 核实，可能 deprecate 其一
```

`NativeAgentPermissionDecision` 是 `NativeAgentLoopV2Request::provided_permission_decisions`
的公开字段类型，已经属于对外 request API 的组成部分；Phase 3 不改 request
签名或字段形状，因此保留为 L1 `pub`。

### 4.5 Fixtures（**全部** `pub`，但 Phase 2 已计划移出 production crate）

P3.1 当前不动 fixtures 的可见性——它们将在 Phase 2 整体迁移到 `crates/runtime-fixtures` 或 feature-gated。P3.2 不要给 `native_agent_loop_fixtures.rs` 改可见性。

```rust
pub struct ScriptedNativeAgentLoopFixtureResult { ... }
pub struct NativeAgentLoopExternalDecisionPackage { ... }
pub struct NativeAgentLoopExternalDecisionPackageResumeResult { ... }
```

### 4.6 禁止扩张

任何不在 4.1-4.5 的项 **不允许** 是 `pub`。当前父文件 `native_agent_loop.rs` 中违规的 `pub` 项：

| 项 | 当前可见性 | 目标 |
|---|---|---|
_无。_

P3.2 必须用 `rg "crate::native_agent_loop::"` 确认每个 `pub` 项的真实外部用法，决策记录到 PR 描述。

---

## 5. L2 家族 API 清单（逐 sibling）

每个 sibling 的清单约束：

- 第二列"对家族暴露"是 `pub(in crate::native_agent_loop)` 的白名单——可被其他 sibling 或父文件 import
- 第三列"私有内部"——不许跨 sibling 调用，必须降级为 L3
- 第四列"允许调用"——本 sibling 可 import 的 sibling 集合（不在列表中的 sibling 一律禁止 import）

> 表中只列**项名**，不列签名；P3.2 实施时签名维持现状，仅改 visibility 与 use 路径。

### 5.A · `entrypoints`（Layer A）

| 角色 | 项 |
|---|---|
| 对家族暴露 | _（无；entrypoints 是入口，自己不被任何 sibling 调用）_ |
| 私有内部 | 所有 helper fn |
| 允许调用 | parent (`native_agent_loop.rs`)、`completion`、`continuation`、`model_io`、`tools`、`prompt`、`execution`、`util` |

特殊约束：entrypoints 是 Layer A，可调下面所有层；但 **不能调 `resume`**（resume 是独立入口）。

### 5.A · `resume`（Layer A）

| 角色 | 项 |
|---|---|
| 对家族暴露 | _（无；resume 是入口，自己不被任何 sibling 调用）_ |
| 私有内部 | 主入口 fn 自身（已 `pub`） |
| 允许调用 | parent、`completion`、`continuation`、`model_io`、`tools`、`prompt`、`execution`、`util` |

### 5.B · `completion`（Layer B）

| 角色 | 项 |
|---|---|
| 对家族暴露 | `complete_native_loop_with_visible_finalizer_or_fallback`、`complete_native_loop_with_tool_inventory_summary`、`complete_native_loop_with_fast_auto_write_summary`、`complete_native_loop_with_visible_fallback`、`tool_inventory_summary_message`、`completion_status_from_batch`、`record_native_loop_turn_summary`、`record_visible_assistant_message`、`emit_runtime_visible_finalizer_fallback`、`extract_final_answer_tool_call` |
| 私有内部 | `synthesized_visible_finalizer_message`、`compact_fallback_preview`、`try_visible_finalizer_from_evidence`、`native_loop_visible_finalizer_system_prompt`、`native_loop_visible_finalizer_prompt` |
| 允许调用 | parent、`model_io`、`tools`、`prompt`、`execution`、`util` |

理由：finalizer 模板与 fallback preview 是 completion 自己的实现细节，不应被其他 sibling 复用。如果未来 `entrypoints` 需要复用，先经 PR 修订本规范。

### 5.B · `continuation`（Layer B）

| 角色 | 项 |
|---|---|
| 对家族暴露 | `build_native_tool_evidence_continuation_request`、`build_native_compacted_initial_request`、`build_native_tool_result_continuation_request`、`compacted_prompt_for_model`、`deepseek_reasoning_replay_for_tool_continuation` |
| 私有内部 | `native_tool_evidence_continuation_prompt`、`continuation_messages_for_provider_replay`、`assistant_tool_calls`、`tool_messages`、`deepseek_openai_tool_calls_from_messages`、`deepseek_openai_tool_results_from_messages`、`deepseek_tool_uses_from_messages`、`deepseek_tool_results_from_messages`、`qwen_tool_calls_from_messages`、`qwen_tool_results_from_messages` |
| 允许调用 | parent、`model_io`、`tools`、`prompt`、`execution`、`util` |

### 5.C · `model_io`（Layer C）

| 角色 | 项 |
|---|---|
| 对家族暴露 | `emit_new_session_events`、`send_with_live_visible_stream_events`、`guard_native_loop_prepared_request`、`guard_native_loop_prepared_request_report`、`record_native_loop_model_call_started_for_prepared_request`、`record_native_loop_role_call_event`、`record_live_visible_stream_event`、`flush_live_content_stream_event`、`record_live_content_suppressed_event`、`flush_live_thinking_stream_event`、`DeepSeekCacheZoneTelemetry`、`record_deepseek_cache_zone_telemetry`、`record_deepseek_cache_zone_event`、`extract_cache_zone_hash` |
| 私有内部 | `NATIVE_LOOP_MAX_TRANSIENT_HTTP_ATTEMPTS`（const）、`should_retry_transient_status`、`native_loop_retry_delay_ms`、`record_native_loop_http_retry_scheduled`、`record_native_loop_http_retry_completed` |
| 允许调用 | parent、`prompt`、`execution`、`util` |

P3.3 现状：`emit_new_session_events` 已从 `resume` 下沉到 `model_io`，消除 B/C 对 A 层的反向依赖。

### 5.C · `tools`（Layer C）

| 角色 | 项 |
|---|---|
| 对家族暴露 | `handle_native_stream_tool_event`、`execute_streamed_native_tool_call_collect`、`StreamedToolExecution`、`is_stream_executable_tool`、`is_stream_candidate_provider_tool`、`record_deepseek_tool_call_assembled_event`、`record_deepseek_stream_tool_call_partial_event`、`execute_concurrent_read_only_batch`、`execute_read_only_collect`、`execute_duplicate_observation_collect`、`execute_unsupported_model_tool_collect`、`execute_permissioned_command_collect`、`execute_permissioned_write_collect`、`execute_fast_auto_write_collect`、`execute_fast_auto_write_create_repair`、`replayed_tool_completion_state`、`ReplayedToolCompletionState`、`model_provider_tool_call_id`、`append_stream_mismatch_error_results`、`PermissionedWriteOutcome`、`PermissionedCommandOutcome`、`tool_calls_are_cached_observations`、`unsupported_model_tool_result`、`canonical_json_text`、`contains_executable_dsml_markup`、`text_without_fenced_code`、`next_available_generated_html_path`、`fallback_html_small_program` |
| 私有内部 | `record_task_dispatch_subagent_completion`、`fast_auto_write_permission_gate_result`、`next_fast_auto_write_create_path`、`fallback_nonce`、`merge_fast_auto_write_repair_detail` |
| 允许调用 | parent、`prompt`、`execution`、`util` |

注：`tools` 已吸收原 `stream` sibling 的 streamed tool execution（2026-05-26 Phase 3.c）。

P3.2 优先级：本 sibling pub 表面最大，**应是 Phase 3.4 "禁止新 sibling" 的最先压力点**——后续若 tools 继续膨胀，必须拆子模块（如 `tools/fast_auto_write.rs`），而不是新增 sibling。

### 5.D · `prompt`（Layer D）

| 角色 | 项 |
|---|---|
| 对家族暴露 | `compact_tool_evidence_summary`、`build_native_loop_tool_manifest`、`native_loop_manifest_exposure`、`native_loop_system_prompt`、`native_loop_prompt_with_turn_directives`、`native_loop_continuation_hint`、`native_loop_tool_execution_error_result`、`final_answer_tool_schema_json`、`provider_tool_name_for_deepseek`、`validate_fast_auto_write_runtime_constraints`、`tool_inventory_records`、`should_finalize_tool_inventory`、`should_finalize_fast_auto_write`、`NativeToolBatch`（type alias）、`record_native_tool_batch_item`、`replace_native_tool_batch_from_legacy`、`continuation_view_for_batch`、`ledger_class_for_tool_result`、`model_readable_error_signature`、`native_loop_write_directive_for_prompt`、`native_prompt_wants_file_generation`、`native_prompt_is_long_running`、`native_prompt_wants_tool_inventory`、`sanitize_http_failure_preview`、`is_tool_budget_refusal_text` |
| 私有内部 | `native_prompt_wants_write_or_edit`、`native_prompt_user_intent` |
| 允许调用 | parent、`util` |

理由：`prompt` 是关键词驱动的提示装配——Phase 7 接入 TurnRouter 后这些"_wants_" 关键词函数应被 route 替代。先标私有，避免在 Phase 7 之前有新调用方依赖它们。

### 5.D · `execution`（Layer D）

| 角色 | 项 |
|---|---|
| 对家族暴露 | `execute_model_readable_error_collect`、`record_tool_call_requested_preserving_provider_id`、`record_tool_call_completed_preserving_provider_id`、`record_tool_result_artifact_preserving_provider_id`、`NativePermissionDecisionOutcome`、`dispatch_pre_tool_use_hook`、`dispatch_post_tool_use_hook`、`tool_args_json`、`execute_patch`、`prepare_patch_execution_args`、`prepare_exact_edit_execution_args`、`ensure_executing`、`execute_pending_tool_after_decision`、`tool_permission_decision`、`tool_args`、`permission_args_json`、`permission_summary_for_tool`、`permission_tool_for_id`、`suggested_permission_denial_replacement` |
| 私有内部 | _（无；现状全部跨 sibling 复用）_ |
| 允许调用 | parent、`util` |

### 5.Z · `util`（Layer Z）

| 角色 | 项 |
|---|---|
| 对家族暴露 | **全部当前 `pub(super)` 项保留为 L2**（util 是公共纯函数池） |
| 私有内部 | 无 |
| 允许调用 | parent、std、workspace crate 公共 API。**禁止** 调任何 sibling |

注意事项：
- util 当前包含 `live_deepseek_endpoint`、`live_qwen_endpoint`——这是配置常量构造，不是真"纯函数"。P3.2 后单独评估是否升到 D 层 `endpoints.rs`。
- `ModelUsageTotals` struct 与 `aggregate_model_usage_from_jsonl` 看起来是 telemetry 聚合——评估是否归 `agent_kernel::telemetry`。本 phase 不动。
- `native_permission_policy_path` 已从 execution 下沉到 util，用于消除 util 对 execution 的反向依赖。

### 5.X · `fixtures` 与 `tests`（Phase 2 待迁移）

P3.1 / P3.2 不动这两个文件的可见性。它们在 Phase 2 整体移出 production crate 后，剩余 sibling 数量减少到 10，可视为本规范的最终形态。

---

## 6. `use super::*` 替换策略（P3.2 操作指南）

P3.2 把每个 sibling 顶部的 `use super::*;` 替换为显式 import。**显式 import 路径必须以 §5 清单为准**。

### 6.1 替换模板

每个 sibling 顶部 `use super::*;` 之后**新增**一段：

```rust
// === native_agent_loop family imports (per docs/architecture/native_agent_loop_module_api.md §5) ===

// Parent types (always allowed)
use crate::native_agent_loop::{
    NativeAgentLoopV2Request, NativeAgentLoopResult, NativeAgentLoopStatus,
    NativeAgentToolExposure, PendingNativeToolExecution,
    // ... 仅 §4.3 列表 + parent 内部 pub(in ...) 项
};

// Allowed siblings (only those in §5.X "允许调用" of this sibling)
use crate::native_agent_loop::util::{...};
use crate::native_agent_loop::prompt::{...};
// ...
```

完成后**删掉** `use super::*;`。

### 6.2 父文件 `native_agent_loop.rs` 改造

父文件目前用 `#[path = "..."] mod xxx;` 引入 11 个 sibling。P3.2 保持声明语法不变，但：

1. 每个 `mod xxx;` 前加注释说明该 sibling 的 Layer（参考本规范 §3）
2. 父文件本身的内部 fn 应改为 `pub(in crate::native_agent_loop)` 或私有；只有 §4 中真正对外的 5 类型 + main 函数保持 `pub`

### 6.3 cross-sibling 违规处置

当 P3.2 发现某 sibling 调用了 §5 "允许调用" 之外的 sibling，**不要悄悄改清单**。流程：

1. 在 PR 描述中列出违规调用点（file:line + 函数名）
2. 由 Opus（架构 review）判定：
   - (a) 下沉公共逻辑到更低 layer
   - (b) 调用方升 layer
   - (c) 修订本规范（罕见，需独立 PR）
3. 不允许的"绕过"：不要新建 `super::super::` 路径、不要把项重新 re-export、不要用 trait 隐藏直接调用

### 6.4 visibility 三层落地

| 当前 | 目标 | 操作 |
|---|---|---|
| 项在 §4 白名单 | `pub` | 保持 |
| 项在 §5 L2 清单 | `pub(in crate::native_agent_loop)` | 把 `pub(super)` 改为 `pub(in crate::native_agent_loop)` |
| 项在 §5 私有清单 | 无修饰 | 删 `pub(super)`；若有跨 sibling 调用方，先内联或下沉 |
| 项不在任何清单 | 视为遗漏 | PR 描述列出，等待 Opus 补本规范 |

---

## 7. CI 强制（P3.3 lint 脚本规范）

`scripts/lint_native_loop_boundary.sh` 必须实现以下断言：

```bash
# 7.1 sibling 内禁止 use super::*
for f in crates/runtime/src/native_agent_loop_*.rs; do
  [[ "$f" == *_tests.rs ]] && continue        # tests 阶段性豁免
  [[ "$f" == *_fixtures.rs ]] && continue     # fixtures 待 Phase 2 迁移
  if grep -q "use super::\*;" "$f"; then
    echo "BOUNDARY VIOLATION: $f still uses 'use super::*'"; exit 1
  fi
  code_lines=$(grep -cE '^\s*[^/[:space:]]' "$f")
  if [[ "$code_lines" -lt 5 ]]; then
    echo "BOUNDARY VIOLATION: $f is an empty/placeholder sibling"; exit 1
  fi
done

# 7.2 父文件外的 pub 项受白名单约束
allowed_pub=$(awk '/^### 4\./{flag=1} /^---/{flag=0} flag && /^```rust/,/^```/{print}' \
              docs/architecture/native_agent_loop_module_api.md \
              | grep -oE 'pub (fn|struct|enum) [A-Za-z_]+' | awk '{print $3}' | sort -u)
actual_pub=$(grep -rE "^pub (fn|struct|enum) " crates/runtime/src/native_agent_loop*.rs \
             | grep -v "_fixtures.rs" | grep -v "_tests.rs" \
             | grep -oE 'pub (fn|struct|enum) [A-Za-z_]+' | awk '{print $3}' | sort -u)
diff <(echo "$allowed_pub") <(echo "$actual_pub") \
  || { echo "PUB SURFACE DRIFT: unauthorized pub items"; exit 1; }

# 7.3 layer 依赖检查（粗粒度：禁止下层 import 上层）
forbidden_pairs=(
  "native_agent_loop_util.rs:native_agent_loop_(prompt|execution|model_io|tools|completion|continuation|entrypoints|resume)"
  "native_agent_loop_prompt.rs:native_agent_loop_(model_io|tools|completion|continuation|entrypoints|resume)"
  "native_agent_loop_execution.rs:native_agent_loop_(model_io|tools|completion|continuation|entrypoints|resume)"
  "native_agent_loop_(model_io|tools).rs:native_agent_loop_(completion|continuation|entrypoints|resume)"
  "native_agent_loop_(completion|continuation).rs:native_agent_loop_(entrypoints|resume)"
)
for pair in "${forbidden_pairs[@]}"; do
  src="${pair%%:*}"; tgt="${pair#*:}"
  hit=$(rg -l "use crate::native_agent_loop::($tgt)" crates/runtime/src/$src 2>/dev/null)
  if [ -n "$hit" ]; then
    echo "LAYER VIOLATION: $hit imports forbidden sibling matching $tgt"; exit 1
  fi
done

# 7.4 新增 sibling 检测
count=$(ls crates/runtime/src/native_agent_loop_*.rs | wc -l)
if [ "$count" -gt 12 ]; then
  echo "NEW SIBLING FORBIDDEN: $count files (max 12 allowed pre-Phase-2.b)"; exit 1
fi
```

P3.3 在 PR CI 中跑；本地 pre-commit 也可挂。

---

## 8. P3.2 PR 拆解建议

为符合"不混合 PR"纪律，P3.2 拆为 3 个独立 PR：

| PR | 目标 | 验收 |
|---|---|---|
| PR-3.2.a | 仅父文件 `native_agent_loop.rs` 的 `pub` 清理 + §4 白名单 enforce | §4 全部白名单项 pub、其余降级；`cargo check` 通过 |
| PR-3.2.b | sibling 全部 `use super::*` 替换为显式 import；可见性降到 L2/L3 | §7.1 + §7.2 lint 通过 |
| PR-3.2.c | 违规 cross-layer import 处置（下沉/升层）+ §7.3 lint 接入 CI | §7 全部 4 项断言通过 |

3 个 PR 必须按顺序合并；中间发现规范错漏 → 暂停 PR，先开规范修订 PR。

---

## 9. 验收（P3 完成定义）

| 指标 | 目标 | 度量方法 |
|---|---|---|
| sibling `use super::*` 计数 | 0（不计 tests/fixtures） | §7.1 lint |
| `pub` 项总数（不计 tests/fixtures） | ≤ 12 | §7.2 lint |
| `pub(super)` 项总数 | 0（全部转为 `pub(in crate::native_agent_loop)` 或私有） | grep |
| Layer 违规 import | 0 | §7.3 lint |
| sibling 文件数 | ≤ 12（pre-Phase-2.b）/ ≤ 10（post-Phase-2.b） | §7.4 lint |
| 跨 sibling fn 调用数变化 | 下降 ≥ 50% | `cargo-modules generate graph` 前后对比 |

任一指标未达标，P3 不算完成。

---

## 10. 不动什么

- 不动任何 fn 签名（参数、返回类型保持不变）
- 不动任何 `impl` 块的方法可见性（仅自由函数受本规范约束）
- 不动 fixtures 与 tests 文件（Phase 2 范围）
- 不重命名任何 fn / type
- 不引入新依赖
- 不删除任何函数（仅改可见性 + 改 import 路径）

P3 是**结构整形**，不是行为改动。任何行为改动需 separate PR。
