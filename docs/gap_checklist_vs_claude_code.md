# Deep-Code vs Claude Code 综合差距清单

> 基于 7 个子代理的逐模块对比审查，2026-05-15

---

## 一、架构核心差距 (Critical / P0)

| # | 差距 | 现状 | 影响 | 涉及模块 |
|---|------|------|------|----------|
| 1 | **子代理不是真正的 AI 子代理** | `run_subagent_task()` 只是硬编码的只读工具脚本，不调用模型 | 子代理功能本质上不可用 | runtime_facade, subagent |
| 2 | **event_store 已删除，无事件持久化** | EventLog 仅内存 `Vec`，重启丢数据，无 crash recovery | Session 重启后所有事件丢失 | event_log, session |
| 3 | **无 Rust 原生 HTTP 客户端** | 所有 HTTP 请求通过 Python sidecar 进程（`provider_http_sidecar.py`），每个请求 spawn 新进程 | 进程边界开销 + 无连接池 + 无 TLS 复用 | live_http_transport |
| 4 | **JSON 全部字符串拼接，无 serde** | 4+ 个文件各自实现 `format!()` 拼接 JSON | 格式错误风险高，不可维护 | live_model_request, event_log, payload, compatible_provider |
| 5 | **Payload from_json() 反序列化完全缺失** | `RuntimeEventPayload` 只能 `to_json()`，不能 `from_json()` | 所有 payload 解析依赖手写字符串操作 | payload, event_log, session |
| 6 | **Worktree 仅规划，不执行** | 只验证参数，不调用 `git worktree add/remove` | 多代理并行写入无文件系统隔离 | worktree, subagent |

---

## 二、安全差距 (P0/P1)

| # | 差距 | 严重度 | 说明 |
|---|------|--------|------|
| 7 | **多 Hook 决策冲突无合并策略** | P0 | Hook A Allow + Hook B Deny → 调用方不知道用哪个。Claude Code: Deny > Modify > Warn > Allow |
| 8 | **PermissionPolicyStore 无并发写保护** | P0 | `add_rule` 无文件锁，多线程并发写导致数据丢失 |
| 9 | **Session scope 规则永不过期** | P0 | Session 结束后规则残留在 TSV 文件中 |
| 10 | **System prompt 缺失安全规则** | P1 | 无路径保护、命令注入防范、敏感数据检测等安全指令 |
| 11 | **密钥扫描检测覆盖严重不足** | P1 | 缺少 GitHub/GitLab/Slack/Stripe/JWT/Discord/Telegram token 检测、缺失通用 PEM 私钥头、无熵检测 |
| 12 | **HookDecision::Modify 从未被应用** | P1 | 死代码路径——Modify 被定义和验证但从未实际注入修改后的参数 |
| 13 | **命令分类器子串匹配误报** | P1 | `contains()` 裸子串匹配导致 `echo "using .env"` 被误判 |
| 14 | **Approval queue JSON 解析极脆弱** | P1 | 手写 `extract_json_string()` 无法处理嵌套 JSON/转义引号/Unicode 转义 |
| 15 | **Approval queue 无超时机制** | P1 | 挂起的审批请求永远留在队列中 |
| 16 | **缺少 AskUser 工具的特殊 PermissionCheck** | P1 | `requires_user_interaction()` trait 方法无任何工具实现 |
| 17 | **脱敏仅用于 ModelTranscript** | P2 | EventLog 中 stream_delta 预览文本不走脱敏 |
| 18 | **网络程序(curl/wget)一律 Deny** | P2 | Claude Code 允许只读操作（如 `curl -I`）进入 Ask 流程 |
| 19 | **Allow 命令列表过于保守** | P3 | 缺少 `cat`/`head`/`tail`/`echo`/`git status`/`git diff` 等常见安全命令 |
| 20 | **权限类型 tool_id 映射不完整** | P3 | Network/CloudModel/ArtifactExport/ProtectedPath 等类型未使用 |

---

## 三、Session & 事件系统差距 (P1/P2)

| # | 差距 | 严重度 | 说明 |
|---|------|--------|------|
| 21 | **手写 JSON 解析器替代 serde** | P1 | event_from_json/event_to_json 手写，不支持 `\n`/`\r`/`\t`/`\uXXXX` 等转义 |
| 22 | **can_transition 与实际转换路径不一致** | P1 | `forced_transition`、`set_state` 绕过状态机，`begin_interactive_turn` 允许任意状态恢复 |
| 23 | **merge_events 哈希格式不一致** | P1 | 合并用简化的 `session_id:seq:event_type`，原始 append 用完整格式 |
| 24 | **缺少 session 超时/过期** | P2 | 阻塞在 WaitingForPlanApproval 的 session 永不超时 |
| 25 | **缺少 session 清理/驱逐** | P2 | 无 LRU 驱逐，EventLog 无界增长 |
| 26 | **pending_permission 仅支持单一并发** | P2 | `Option<(String, PermissionRequestType)>` 而非 `Vec`，并发权限请求丢失 |
| 27 | **无事件截断/压缩** | P2 | EventLog 无限增长，无数量上限或时间窗口截断 |
| 28 | **Transcript 与 EventLog 无交叉验证** | P3 | 两套独立系统，content hash 无一致性校验 |
| 29 | **缺失 Compacting 等中间状态** | P3 | 上下文压缩/子代理委托等操作无状态记录 |
| 30 | **缺少 `from_json()` 导致所有 payload 解析无类型安全** | P0 | 与 #5 相同，所有字段提取都用手写字符串查找 |

---

## 四、上下文管理差距 (P0/P1)

| # | 差距 | 严重度 | 说明 |
|---|------|--------|------|
| 31 | **缺少对话历史（ConversationTurn）管理** | P1 | 无 `add_conversation_turn`、无 ConversationTurn ContextItemKind、无历史对话轮次注入 |
| 32 | **chars/4 token 估算不准确** | P1 | 中文严重低估（每个汉字 ~1.5-2 tokens）、代码可能高估。Claude Code 使用 tiktoken 实际计数 |
| 33 | **context_to_text_with_budget 无优先级排序** | P1 | 按传入顺序截断，先到先得，重要内容可能先被丢弃 |
| 34 | **两套压缩实现不一致** | P1 | `compaction.rs` 和 `compactor.rs` 两套独立实现，压缩比例不同（40% vs 25%），输出格式不统一 |
| 35 | **System prompt 缺失关键指令** | P1 | 无代码编辑最佳实践、测试运行指令、git 工作流规则、对话历史理解规则、错误恢复规则 |
| 36 | **工具 catalog 缺少自然语言描述** | P2 | 只有技术参数，Claude Code 有详细描述和使用示例 |
| 37 | **guard_native_model_request 仅对 DeepSeek 生效** | P2 | Qwen 的上下文预算仅 log 不做硬限制 |
| 38 | **pending tools/permissions 无超时** | P2 | 工具永不返回时永久挂起。Claude Code 有 5 分钟超时 |
| 39 | **push_if_fits 无优先级** | P2 | 无 item 替换/更新/淘汰策略（FIFO/LRU） |
| 40 | **DeepSeek 无渐进降级策略** | P3 | Qwen 有 degraded 模式（context < 128K 降配），DeepSeek 只有线性缩放 |
| 41 | **Qwen Guarded vs Fast output_reserve 值不一致** | P3 | Guarded 的 output_reserve 比 Fast 还小（20K vs 18K），不符合"更保守"的语义 |

---

## 五、HTTP 传输 & 模型适配层差距 (P1/P2)

| # | 差距 | 严重度 | 说明 |
|---|------|--------|------|
| 42 | **无 HTTP 重试逻辑** | P1 | 任何网络瞬时错误直接失败返回。Claude Code 有指数退避重试（3次，1s/2s/4s） |
| 43 | **错误分类粒度不足** | P1 | 不区分 429/401/5xx/Network Error/DNS Failure，统一返回 `Err(String)` |
| 44 | **extract_json_* 函数重复 5+ 文件** | P2 | deepseek_stream, qwen_stream, native_response_normalizer, compatible_provider, event_log 各有独立实现 |
| 45 | **兼容提供商无工具/无流** | P2 | CompatibleProviderAdapter 不支持 tools_json 和 SSE 流解析 |
| 46 | **reasoning sanitization 过于简单** | P2 | 仅做字符串替换，不使用 secret_scan 的完整检测 |
| 47 | **max_context_tokens DeepSeek 设为 1M** | P3 | 宣称 1M 但实际有效上下文远小于此，可能导致 prompt 过大被拒绝 |
| 48 | **Thinking 策略硬编码** | P3 | Per-role thinking mode 映射在 `plan_call` 中硬编码，缺少配置化 |

---

## 六、工具系统差距 (P1/P2)

| # | 差距 | 严重度 | 说明 |
|---|------|--------|------|
| 49 | **tool_dispatcher 与 tool_orchestration 功能重复** | P1 | 两套独立的分区逻辑，使用不同的并发安全判定（spec.concurrency_safe vs 硬编码列表） |
| 50 | **tool_orchestration 始终以 permission_decision: None 调用** | P1 | shell.command 和文件写入在 batch 路径中因无权限而全部失败 |
| 51 | **缺失工具** | P2 | task.output, task.stop, team.create/delete/message, powershell.command, file.delete/move/copy/create_directory, notebook.read, git.commit/log/diff/branch, file.symlink |
| 52 | **Gated 工具 provider_aliases 全空** | P3 | 即使未来启用，模型也无法通过别名引用 |

---

## 七、集成路径差距 (P1/P2)

| # | 差距 | 严重度 | 说明 |
|---|------|--------|------|
| 53 | **Python mock 仍是 Electron/TUI 的默认后端** | P1 | 启动的是 `local_api_server.py`（做关键词匹配模拟 agent），不是 Rust LocalApiServer |
| 54 | **Rust LocalApiServer 未接入用户路径** | P1 | 虽然已实现完整，但无入口自动启动它 |
| 55 | **事件类型名称不一致** | P1 | `dist/app.js` 期望 `user_message`/`assistant_message`，Rust 生成 `user.message_submitted`/`assistant.message` |
| 56 | **快照格式不一致** | P2 | Python mock: `{"snapshot":{...}}` 包装，Rust: 顶层字段，前端期望 `snap.events` 数组 |
| 57 | **SSE vs JSON 契约不一致** | P2 | `dist/app.js` 使用 EventSource(SSE)，Rust LocalApiServer 返回 `application/json` |
| 58 | **端点覆盖差异** | P2 | Python mock: 25+ GET 端点，Rust: 12 个。缺少 `/events`/`/summary`/`/model-timeline`/`/session-snapshot`/`/approval-queue` 等 |
| 59 | **流事件格式不一致** | P2 | Rust: 事件是 JSON 字符串（需二次解析），Python mock: 已解析对象 |
| 60 | **EventLog 中事件类型命名 vs 前端期望不一致** | P3 | Rust 用句点分隔 `tool.call_requested`，前端期望下划线 `tool_call_requested` |
| 61 | **Tauri 是唯一完整真实实现路径** | 备注 | 直接调用 RuntimeFacade，但 DTO 格式与 HTTP API 路径不同 |

---

## 八、子代理 & 多代理系统差距 (Critical/P1)

| # | 差距 | 严重度 | 说明 |
|---|------|--------|------|
| 62 | **Kernel 层 SubagentSpec/Budget 定义但未使用** | P1 | Runtime 层自建了一套类型，budget 控制完全缺失 |
| 63 | **write_scope 无运行时强制** | P1 | Worker 子代理实际只能只读操作（硬编码 4 个只读工具） |
| 64 | **取消是标记级非执行级** | P1 | 无 abort handle/channel 通知/超时打断，失控子代理无法中断 |
| 65 | **子代理无独立 AgentSession/EventLog** | P1 | 所有事件写入父代理 session，事件污染 + 上下文污染 |
| 66 | **AgentTeams 仅为 fixture** | P2 | 有类型定义（AgentTeamRun/Blackboard/ConsensusDecision）但无运行时调度器 |
| 67 | **MultiAgentPolicy 决策未被调用** | P2 | `decide_multi_agent()` 只返回决策，无代码实际执行决策 |
| 68 | **TaskContract/Plan 未接入子代理** | P2 | 约束定义完整但运行时完全不使用 |
| 69 | **ContextPack 内容未实际传递给子代理** | P2 | 只存 ID，summary/evidence_refs 从未被读取 |
| 70 | **writes_allowed_by_default() 始终返回 false** | P3 | 即使 Worker 类型有写工具也是如此 |
| 71 | **ResearchWorker 仅支持 CSV profiling** | P3 | 无通用 Python 脚本/Jupyter notebook 执行，无沙箱隔离 |
| 72 | **Rust 侧无 Python worker 超时保护** | P3 | `output()` 调用可能无限阻塞 |

---

## 九、代码质量 / 技术债务

| # | 差距 | 说明 |
|---|------|------|
| 73 | **escape 函数多处重复定义** | payload.rs、event_log.rs、live_model_request.rs、compatible_provider.rs 各有独立实现 |
| 74 | **运行时状态通过字符串匹配恢复** | `extract_json_string` 查找 `"to_state":"` 等模式，typographic 变化导致静默失败 |
| 75 | **forced_transition 绕过状态机** | `begin_interactive_turn` 使用 `session.forced_transition` 事件绕过 `can_transition` |
| 76 | **并发权限请求单槽限制** | 只能同时有一个 pending_permission |
| 77 | **缺少 serde derive** | 大量结构体使用手写 JSON 而非 `#[derive(Serialize, Deserialize)]` |

---

## 架构亮点（值得保留和扩展）

1. **reasoning_sanitized + reasoning_raw_volatile 双轨设计** — 正确隔离敏感推理内容
2. **DualProtocolFallback** — Anthropic/OpenAI 协议自动切换
3. **三级缓存断点策略** (DeepSeek Cache Planner) — Zone A/B/C 设计合理
4. **DeepSeekStreamAssembly::tool_call_pairs()** — 多层回退逻辑覆盖边界情况
5. **Gate 系统** (NativeLiveCallGate) — 7 种状态的多层安全防护
6. **ToolContract::is_never_repair_field** — 关键字段保护防止静默数据损坏
7. **结构化工具错误** — 包含 error_code/recoverable/suggested_tool/next_action_hint
8. **文件写入的 BOM/换行符保留 + TOCTOU 二次检查** — 编辑安全性好
9. **Tauri 桌面端集成** — 唯一完整的真实实现路径，权限审批后自动继续

---

## 修复优先级建议

### 第一批 (架构基础) — 预计 2-3 周
1. 实现 EventLog 文件持久化（恢复 event_store 或重写）
2. 引入 serde 替换所有手写 JSON 序列化/反序列化
3. 实现 Rust 原生 HTTP 客户端（reqwest/hyper）
4. 统一事件类型命名（与前端契约对齐）

### 第二批 (安全加固) — 预计 1-2 周
5. Hook 决策冲突合并策略
6. PermissionPolicyStore 并发写保护 + Session scope 清理
7. System prompt 安全规则注入
8. 密钥扫描模式扩充

### 第三批 (功能补全) — 预计 3-4 周
9. 子代理真实 LLM 驱动 + Budget 控制 + Worktree 隔离
10. 对话历史管理（ConversationTurn 注入）
11. 上下文优先级排序
12. 工具目录补齐（缺失的 12+ 工具）

### 第四批 (集成统一) — 预计 2-3 周
13. Python mock → Rust LocalApiServer 替换（Electron/TUI 后端）
14. 端点补齐（events/summary/model-timeline/session-snapshot）
15. 快照/流事件格式统一
16. 统一 tool_dispatcher 和 tool_orchestration 分区逻辑
