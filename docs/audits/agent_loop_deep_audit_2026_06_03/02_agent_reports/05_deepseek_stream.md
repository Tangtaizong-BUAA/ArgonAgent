我现在已经全面掌握了整个系统。以下是我的分析报告。

---

## 调查报告：DeepSeek 流式解析器审计

---

### 1. 部分工具参数如何跨块组装？是否存在按索引的累加器？

**是的，存在两层独立的按索引累加器。**

**层级 1：`DeepSeekStreamAssembly`（离线/测试路径）**
- `stream.rs` 第 39-44 行：`DeepSeekStreamToolCall` 包含 `index: usize` 和 `arguments: String`。
- `apply()`（第 66-117 行）：按索引查找或创建条目（第 82-95 行），然后执行 `call.arguments.push_str(&arguments_fragment)`（第 103 行）。
- 索引通过 `index` 字段匹配：如果 delta 提供 `Some(index)`，则使用它；否则回退到 `tool_calls.last().index` 或 0。

**层级 2：`StreamingToolCallAssembler`（实时路径）**
- `stream.rs` 第 787-888 行：使用 `BTreeMap<usize, StreamingToolCallState>` 作为按索引的累加器。
- `ToolCallStarted`：创建或更新 `calls[index]`，设置 `name`，推送 `input_json`。
- `ToolCallArgumentsDelta`：`state.arguments_json.push_str(delta)`。
- `completed_call_if_ready()`（第 856-888 行）：当满足条件时发出完成的调用：必须存在名称，并且参数必须是完整的 JSON 对象，或者（作为回退）`finished` 标志为 true 且参数为空。

**层级 3：`ToolCallPipeline`（统一包装器）**
- `pipeline.rs`：包装 `StreamingToolCallAssembler`，供 `StreamProcessor`（用于 DeepSeek）和 `QwenStreamProcessor` 共享。

**结论：** 按索引的累加是正确的。多个索引被正确分离。单个索引内的参数片段通过 `push_str` 按顺序累积。

---

### 2. 为什么 file.read 可能缺少 "path" 参数？追踪组装路径

**`file.read` 缺少 `"path"` 有三种不同的途径：**

**途径 A：在 `StreamingToolCallAssembler` 中过早完成（风险最高）**
`completed_call_if_ready()`（第 856-888 行）使用 `json_object_complete()` 来确定参数 JSON 是否完整。该函数仅检查括号是否平衡——它不验证必需字段是否存在。参数为 `{}` 的工具调用（语法上有效，但语义上缺少 `path`）将立即被发出。

这种情况发生的具体方式：
1. 在 OpenAI 协议中：`tool_calls[0].function.arguments` 以 `""` 开头（参数为空）→ `input_json = None`。
2. 在 Anthropic 协议中：`content_block_start` 的 `input: {}` 被 `stream.rs` 第 218 行的过滤器 `filter(|value| value.trim() != "{}")` 明确过滤掉。
3. 在极少数情况下，`content_block_delta` 发送 `partial_json: "{}"` 作为一个封闭但空的 JSON 对象。`json_object_complete("{}")` 返回 `true`，并且调用在没有任何增量完成的情况下被发出。

**途径 B：流被截断，未完成的参数永远无法完成**
如果流在工具调用参数完成之前发送 `[DONE]` 或 `message_stop`，则 `StreamingToolCallAssembler` 会持有部分参数，但从不刷新它们。`complete_stream()`（第 336 行）仅检查内容工具调用候选——它不刷新未完成的流式工具调用。证据：
- 第 336-351 行：`complete_stream()` 扫描 `raw_visible_content` 以查找内容工具调用候选。
- 没有调用 `streaming_accumulator_mut()` 或其他方法来刷新未完成的流式调用。
- 在 `native_agent_loop_model_io.rs` 的第 589-691 行的 `send_with_live_visible_stream_events` 中：流处理器在响应完成后被查询，但 `ToolCallPipeline.streaming` 从未被检查未完成的调用。

**途径 C：缺少 `ToolCallFinished` 事件（仅限 OpenAI 协议）**
在 OpenAI 协议中，没有 `content_block_stop`。`StreamingToolCallAssembler` 依赖于 `json_object_complete` 作为其主要完成检测器。如果参数增量恰好产生一个语法上完整但语义上空的对象，则调用完成。此外，索引已经处于 `completed: BTreeSet` 中，因此后续的参数增量被忽略。

**证据碎片（测试证实了基本路径）：**
- `stream.rs` 第 590-599 行：`assembles_tool_call_delta_independently` 测试 2 块组装。
- `stream_processor.rs` 第 442-465 行：`stream_processor_assembles_tool_call_arguments` 测试 `ToolCallStarted` + `ToolCallArgumentsDelta`。
- 两者都假设参数增量的结束通过 `json_object_complete` 完成——它们不测试截断的场景。

**根本原因：** `json_object_complete` 是一个仅检查句法的守卫。一个语法上完整但缺少必需字段的 JSON 对象将会被发出。没有针对截断流的安全网。

---

### 3. 在 Anthropic 兼容模式下，DeepSeek 是否会产生异常的工具块？

**是的，但解析器能够正确地将它们规范化。**

Anthropic 兼容模式下的异常行为：

**1. content_block_start 使用空输入**
DeepSeek 发出：`content_block_start` 带有 `input: {}`（第 673 行测试）。
stream.rs 第 212-220 行使用 `.filter(|value| value.trim() != "{}")` 显式过滤掉 `{}`，因此 `arguments_fragment` 为空。然后 `StreamProcessor.ingest_chunk` 发出带有 `input_json: None` 的 `ToolCallStarted`。这是预期的，因为真正的参数随后会作为 `input_json_delta` 增量到达。

**2. content_block_delta 使用 input_json_delta（非标准的 Anthropic）**
标准的 Anthropic API 工具使用增量应使用 `input_json_delta` delta 类型。stream.rs 第 222-228 行正确地从 `partial_json` 字段中提取。此 delta 产生一个 `DeepSeekStreamDelta::ToolCall`，其中 `id: None` 和 `name: String::new()`——这被 `StreamProcessor.ingest_chunk` 中的守卫 `id.is_some() || !name.is_empty()` 正确识别，因此它被路由到 `ToolCallArgumentsDelta` 事件。

**3. content_block_stop 被 ingest_chunk 处理，确保正确的排序**
`ingest_chunk`（第 78-157 行）做了三件事：
1. 处理 `content_block_start`（第 78-88 行）
2. 解析并应用来自同一行的所有 SSE delta（第 90-136 行）
3. 处理 `content_block_stop`（第 137-151 行）

对于工具使用块，此排序意味着工具调用在 `content_block_stop` 发出之前被注册（通过 delta 解析），然后 `ToolCallFinished` 触发完成检查。这在测试中有效；但如果在步骤 2 解析之前没有收到工具块 delta，则排序可能成为一个问题（因为 `parse_deepseek_sse_line_all` 在同一 `data:` 行上查找 `"type":"tool_use"`，而 `content_block_start` 的 block_type 是单独提取的）。

---

### 4. 架构修复应相对于流式组装发生在哪里？

**架构修复发生在组装之后，执行之前。**

具体顺序如下：

```
[SSE 行] -> parse_deepseek_sse_line_all() 
    -> [DeepSeekStreamDelta] -> StreamProcessor.ingest_chunk()
        -> [LiveHttpStreamEvent] -> StreamingToolCallAssembler.apply()
            -> [CompletedStreamingToolCall] -> <发出给事件处理程序>
                -> mediate_tool_call_with_provider_id() [tcml/contract.rs:238]
                    -> parse_tool_arguments()
                    -> apply_low_risk_repairs()
                    -> validate_required_arguments()
```

`apply_low_risk_repairs()` 在 `contract.rs` 第 395 行被调用——在 `parse_tool_arguments()` 之后，但在 `validate_required_arguments()` 之前。这会添加诸如默认值、关系修复（例如 `FileReadRelationalDefault`）和类型转换等修复。

对于 `file.read` 特别：
- `repair_catalog.rs` 第 38-39 行：`can_repair_field` 允许修复许多 `file.read` 字段（`limit`、`max_bytes` 等）。
- 但文件路径修复（`path`）未被列为可修复字段——这是正确的，因为路径含义不能凭空合成。
- `relational_resolver.rs`：`file_read_relational_default` 处理 `limit`/`offset` 关系（如果一个存在而另一个不存在），但不处理 `path`。

**关键差距：** `json_object_complete` 的括号检查是调用进入架构修复之前的唯一完整性检查。如果 `arguments = "{}"` 通过 `json_object_complete`，它将到达 `mediate_tool_call`，然后对缺少 `path` 工具调用失败并显示 `schema_validation_failed_error`。此错误被转发到模型，但已经发生了完整的往返——并且该工具调用已被计入周转预算。

---

### 5. StreamProcessor 如何处理 `tool_calls[0].function.arguments` 分 5 个单独的块到达？

**它可以正确处理。3 个块的过程是相同的：**

1. **块 1**（名称 + 部分参数）：`parse_openai_tool_call_deltas` 产生 `ToolCall { name: "file.read", arguments_fragment: "{"path\":", index: Some(0) }`。在 StreamProcessor 中：发出 `ToolCallStarted { input_json: Some("{"path\":") }`。StreamingToolCallAssembler 创建索引 0，设置 name="file.read"，推送 `"{"path\":"`。`json_object_complete` 返回 false。

2. **块 2-4**（仅片段参数）：`parse_openai_tool_call_deltas` 产生 `ToolCall { name: "", arguments_fragment: fragment }`。StreamProcessor 路由到 `ToolCallArgumentsDelta` 事件。StreamingToolCallAssembler 推送增量。`json_object_complete` 在每次添加后检查。

3. **块 5**（完成参数）：当最后一个片段完成 JSON 时，`json_object_complete` 返回 true。调用已发出。索引 0 被添加到 `completed: BTreeSet`。后续块（块 6+）被忽略。

**边缘情况：** 如果 `name` 出现在块 2（第一个块没有名称）中，则 `args_fragment` 仅在 `id.is_some() || !name.is_empty()` 为 false 且 `!arguments_fragment.is_empty()` 为 true 时才被路由到 `ToolCallArgumentsDelta`。如果不是这样，它将命中 `ToolCallStarted` 路径。但 `StreamingToolCallAssembler.apply` 为 `ToolCallStarted` 处理 `name`，这没有问题。

**未测试的行为：** `name` 和 `arguments` 在不同块中到达的 5 块场景——标准 OpenAI 流式格式通常在前 2 个块中包含名称和 ID，但理论上，如果 `name` 出现在块 3，它将是 `ToolCallStarted`（重置参数）或 `ToolCallArgumentsDelta`（不设置名称）。**在持有关键字段（名称、ID、参数）的同时在块之间切换的 `StreamingToolCallAssembler` 行为没有明确的测试。**

---

### 6. 当 finish_reason 到达但 args_buffer 不是有效的 JSON 时会发生什么？

**工具调用被静默丢弃。**

序列：
1. 包含 `finish_reason` 的有效负载也包含 `tool_calls` delta。
2. `parse_deepseek_sse_line_all` 产生 `StopReason(...)` 和 `ToolCall{...}` 两者。
3. 在 `StreamProcessor.ingest_chunk` 中：`StopReason` 匹配 `_ => {}`（第 132-134 行）——**完全忽略**。
4. `ToolCall` delta 被处理：参数片段被推送到 `StreamingToolCallAssembler`。
5. 如果 `json_object_complete` 返回 false，`completed_call_if_ready` 返回 `None`。
6. 流以 `[DONE]` 或 `message_stop` 继续 → `DeepSeekStreamDelta::Done` → StreamProcessor 设置 `completed = true`。
7. `complete_stream()`（第 336 行）仅检查 `raw_visible_content` 以查找内容工具调用候选。它没有检查 `StreamingToolCallAssembler` 是否有未完成的调用。

**证据碎片：**
- stream_processor.rs 第 132-134 行：`StopReason(_) | Telemetry(_) | Ignored => {}`。
- 第 336-351 行：`complete_stream()` 仅扫描 `raw_visible_content`。

**根本原因：** StreamProcessor 中没有状态字段可以捕获 `finish_reason`。`DeepSeekStreamDelta::StopReason` 被解析但从未被流处理器内部消费。`StreamCompleted` 事件没有传达原因。

**doc39 冲突：** 是的，违反 doc39 §4。如 `doc39_audit_reports/07_deepseek_stream_reasoning.md` 所述："Missing: finish_reason, last_chunk_at fields" 和 "StreamCompleted { finish_reason } → PARTIAL (no reason)"。

---

### 7. DSML 过滤在块之间是否有状态？

**是的，`DsmlChunkFilter` 完全是有状态的并且跨块工作。**

`DsmlChunkFilter` 状态（stream.rs 第 716-784 行）：
- `inside: bool`：当前是否在开始标签和结束标签之间。
- `pending: String`：保存可能跨块边界的部分标记。

**跨块工作的机制：**

1. 当块 N 以部分打开标记结束，例如 `visible <too` 时，`filter()` 方法：
   - 将可见部分 `"visible "` 附加到输出。
   - 将部分标记 `"<too"` 保存到 `self.pending`（通过 `split_pending_marker_prefix`）。
   - 返回 `"visible "`。

2. 在块 N+1 上（`l_call>secret</too`），`filter()` 方法：
   - 检测到 `self.pending` 非空，因此它将块预先添加到组合字符串中：`"<tool_call>secret</too"`。
   - 找到开始的 `<tool_call>` → 设置 `self.inside = true`，跳过 `secret`。
   - 找到不完整的结束标记 `</too` → 未找到完整的结束标记。`self.inside` 仍然为 true。将 `"</too"` 保存到 `self.pending`。返回 `""`（输出为空）。

3. 在块 N+2 上（`l_call> done`）：
   - 组合：`"</tool_call> done"`。
   - 找到结束 `</tool_call>` → 设置 `self.inside = false`，剩余 `" done"`。
   - 输出：`" done"`。

**测试确认：**
- `dsml_filter_hides_cross_chunk_content`（第 957-962 行）：跨 3 个块的可见+隐藏+可见。
- `dsml_filter_buffers_split_markers`（第 964-970 行）：跨 3 个块的分裂标记 `<too`/`l_call>` 和 `</too`/`l_call>`。

**结论：** 一个从块 N 开始到块 N+5 结束的 DSML 标签将被完全过滤。状态引擎（`inside` + `pending`）可以处理任意数量的中间块。

---

### 8. 比较 DeepSeek 流式解析器与 Qwen 流式解析器

**它们是独立的实现，但共享相同的核心工具调用组装机制。**

| 方面 | DeepSeek | Qwen |
|---|---|---|
| **SSE 解析器** | `parse_deepseek_sse_line_all` (stream.rs:185) | `parse_qwen_sse_line_all` (qwen_stream.rs:156) |
| **组装结构** | `DeepSeekStreamAssembly` (stream.rs:47) | `QwenStreamAssembly` (qwen_stream.rs:48) |
| **Delta 枚举** | `DeepSeekStreamDelta` (stream.rs:9) | `QwenStreamDelta` (qwen_stream.rs:10) |
| **工具调用组装** | `StreamingToolCallAssembler` (stream.rs:787) -- 两者共享 | 相同的（从 tcml 导入） |
| **流式状态机** | `StreamProcessor` (stream_processor.rs:62) | `QwenStreamProcessor` (qwen/stream_processor.rs:58) |
| **协议支持** | Anthropic + OpenAI 兼容 | 仅 OpenAI 兼容 |
| **推理处理** | 两层：sanitized + raw volatile | 单层：仅 sanitized |
| **DSML 过滤** | `DsmlChunkFilter` (stream.rs:716) | 无 |
| **内容抑制** | 前导内容 + 后工具内容 | 仅后工具内容 |
| **块跟踪** | content_block_types BTreeMap + active_content_block_type | 无 |
| **Telemetry** | reasoning_tokens, cache_hit/miss | total_tokens |

**关键差异：**

1. **解析器复杂性：** DeepSeek 的解析器有 6 个格式分支（thinking、text_delta、type:tool_use、input_json_delta、tool_calls、finish_reason），而 Qwen 有 4 个（thinking/content/tool_calls/usage）。

2. **推理特性：** DeepSeek 保持 `reasoning_raw_volatile` 用于回放，而 Qwen 只保持 `thinking_sanitized`。这直接影响 DeepSeek 上的 `ReasoningReplayManager` 和 `DEEPSEEK_REASONING_REPLAY_BUDGET_TOKENS`。

3. **流式状态机复杂性：** `StreamProcessor`（380 行）比 `QwenStreamProcessor`（207 行）更复杂。DeepSeek 需要它来定位内容块（text/thinking/tool_use），防止 DSML 泄漏，并基于 `active_content_block_type` 有条件地抑制前导内容。

4. **后工具抑制：** DeepSeek 在 `had_tool_call` 为 true 时通过 `dsml_filter` 路由后工具文本（stream_processor.rs 第 225-251 行）。Qwen 简单地将后工具文本计数为抑制（qwen/stream_processor.rs 第 143-159 行），无需过滤。

5. **内容工具调用扫描：** 两者都在 `complete_stream()` 期间扫描原始可见内容以查找内容工具调用候选。但 DeepSeek 的 `complete_stream()` 仅在 `!had_tool_call` 时扫描（第 337 行），这意味着**如果发生了流式工具调用，DeepSeek 不会扫描内容工具调用候选**。Qwen 始终扫描（第 192-193 行）。

**代码重复：** `extract_json_string`、`extract_json_u64`、`extract_json_array`、`split_top_level_json_objects` 在 stream.rs 和 qwen_stream.rs 中重复。DeepSeek 的版本处理 Unicode 转义（`\\uXXXX`）；Qwen 的版本不处理。DeepSeek 的 `extract_json_object` 在 Qwen 的解析器中没有对应物。

---

### 综合发现

#### P1：`finish_reason` 和 `[DONE]` 之间未完成的流式工具调用被静默丢弃

- **文件：** `stream_processor.rs` 第 132-134 行，第 336-351 行；`stream.rs` 第 856-888 行
- **根本原因：** `complete_stream()` 没有刷新 `StreamingToolCallAssembler`。`finish_reason` 被完全忽略。
- **分类：** 数据丢失。如果网络或提供程序在一个工具调用的参数增量完成之前发出 `[DONE]`，则该调用将丢失。没有错误记录，没有重试，没有 observable。
- **doc39 冲突：** 是。doc39 §4 需要 `StreamCompleted { finish_reason }` 并在流终止时刷新未完成的调用。`07_deepseek_stream_reasoning.md` 将此标记为 "Missing"。

#### P2：`json_object_complete` 是退出 `StreamingToolCallAssembler` 的唯一门控——缺少语义验证

- **文件：** `stream.rs` 第 891-925 行（`json_object_complete`），tcml/contract.rs 第 346-383 行
- **根本原因：** 语法上有效的 `{}` 通过 `json_object_complete`，到达架构修复，并成功拒绝缺少 `path` 的 `file.read`——但是调用已经被计入预算，并且 `ToolCallAssembled` 事件已经被触发。
- **建议：** 在 `completed_call_if_ready` 中或在完成时立即添加快速的必需字段检查，以在发出 `ToolCallAssembled` 事件之前捕获明显的缺失字段。

#### P2：缺少 `name` 或 `id` 的工具调用块之间的序列问题

- **文件：** `stream_processor.rs` 第 103-128 行
- **根本原因：** 当 delta 没有 `id` 和空白 `name` 但非空 `arguments_fragment` 时，它被路由到 `ToolCallArgumentsDelta`。如果 `ToolCallStarted` 尚未出现（例如，分块的 OpenAI delta，其中 `name` 出现在块 3），则 `StreamingToolCallAssembler` 处理程序 `ToolCallArgumentsDelta { index }` 使用 `index.or(self.current_index).unwrap_or(0)`。如果块 3 添加 `name`，则 `StreamingToolCallAssembler` 通过 `ToolCallStarted` 处理程序设置它。但是块 1 和 2 的参数增量已经被推送到 `self.calls.entry(index).or_default()`，所以尽管 `current_index` 不匹配，但索引必须正确才能在正确的插槽中累积。

#### P3：`extract_json_string` 在 stream.rs 和 qwen_stream.rs 中重复，具有不同的功能

- **文件：** `stream.rs` 第 343-402 行，`qwen_stream.rs` 第 280-309 行
- **根本原因：** DeepSeek 的版本处理 `\\uXXXX` Unicode 转义。Qwen 的版本不处理。如果一个 Qwen SSE 行包含 `\\uXXXX` 转义，Qwen 的解析器将过早终止字符串提取。
- **风险：** 低——Qwen 不太可能在流式 JSON 中使用 `\\uXXXX` 转义，但语义差距表明需要共享实现。

#### P3：`tool_call_pairs()` 回退逻辑可能产生不一致的元组

- **文件：** `stream.rs` 第 126-175 行，`qwen_stream.rs` 第 118-146 行
- **行为：** 如果 `DeepSeekStreamAssembly.tool_calls` 中没有一个条目有名称，则 `tool_call_pairs()` 尝试多个回退，包括来自 `refresh_legacy_tool_fields` 的 `self.tool_name`。这些字段是从 `tool_calls` 的 `iter().find(|call| call.name.is_some())` 设置的——这意味着 `tool_name` 和 `tool_arguments` 属于任意的、第一个有名称的工具调用它们。对于多工具响应，`tool_arguments` 可能属于索引 1，而 `tool_name`（如果与其他调用共享）属于索引 0。

---

### 答案摘要

| # | 结论 | 严重性 |
|---|---|---|
| 1 | 是的，按索引的累加器存在于 `DeepSeekStreamAssembly`（离线）和 `StreamingToolCallAssembler`（实时）中 | -- |
| 2 | 通过 3 种途径缺少 `path`：过早完成（`{}` 通过 `json_object_complete`）、流截断、OpenAI 协议中缺少 `ToolCallFinished` | P1 |
| 3 | 是的，DeepSeek 的 Anthropic 模式产生 `input: {}`，后跟 `input_json_delta`——正确解析 | P3 |
| 4 | 修复在**组装之后**发生（通过 `mediate_tool_call_with_provider_id`），但 `json_object_complete` 是唯一的完整性检查 | P2 |
| 5 | 正确处理——在第 N 块重复 `completed_call_if_ready` 直到 `json_object_complete` | -- |
| 6 | 工具调用被**静默丢弃**（`finish_reason` 被忽略，`complete_stream` 不刷新组装器） | P1 |
| 7 | 是的，DSML 过滤完全是有状态的，跨块边界通过 `inside + pending` 工作 | -- |
| 8 | 独立实现，共享 `StreamingToolCallAssembler`；DeepSeek 的支持 Anthropic 协议/推理/DSML 过滤使其复杂得多 | -- |