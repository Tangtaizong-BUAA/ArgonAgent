# Desktop 端深度体验审计与修复/升级计划

> 起草：Opus 2026-05-27
> 范围：`desktop/` (Tauri + React + 8 components) + `desktop/src-tauri/src/main.rs` (13 commands) + 与 `crates/runtime/` 的接线层
> 数据源：3 个并行 subagent 审计报告（前端架构、事件覆盖率、审批+性能全链路）+ live session jsonl (`runtime_session_1779866770924558000`, 23095 events)
> 上游决策：[`D1-tauri-only.md`](decisions/D1-tauri-only.md)、[`architecture_consolidation_plan.md` §Phase 9](architecture_consolidation_plan.md)

---

## 0. 执行摘要

桌面端的核心问题不是"视觉粗糙"，是**"后端做了 N 件事，前端只翻译了 N/4 件"** + **"用户做不了应该能做的事"**。

| 维度 | 当前状态 |
|---|---|
| 后端 emit 的 event_type 总数 | **~140**（grep code）/ 62（活样本一个 session） |
| 前端 dispatch 的 event_type | **~30**（覆盖率 ~22%） |
| 14 个 agent recovery 事件 | **0/14 surface**（"为什么停了"的根源） |
| 70+ context.compaction 事件/session | **0% surface** |
| 13 个 Tauri commands | 全部 wired，但**缺关键 5 个**（cancel turn、set autonomy mode、deny with suggestion、allow project rule、retry transient） |
| AppShell.tsx | **2200 行 god component**（26 useState + 16 useRef） |
| Cancel/Stop 按钮 | **完全不存在**（InterruptService 后端存在但 0 Tauri command） |
| 死按钮（点了无反应） | 至少 **8 处**（Mic、查看更改、artifacts、suggested tasks、再显示 N 个、查看安装指南、自动化面板、插件面板） |
| 监听 backend 从不发的事件（dead branch） | **~10 个** |
| 键盘快捷键 | **0**（无 Cmd+Enter / Esc / Cmd+K） |
| 已知阻塞用户 bug | 100+ tool 卡死、plan 不渲染 markdown、permission 不显示命令内容 |

**结论**：桌面端处于 alpha 状态。要从"看起来能用"到"真能用"，需要 **3 个 sprint（共 ~3-4 周）** 的针对性 fix，不是普通 polish。

---

## 1. 严重度分类清单

### 🔴 Red（阻塞日常使用，必须 sprint-1 修）

| # | 问题 | 证据 |
|---|---|---|
| R1 | **无法 cancel 正在跑的 turn** | `BottomComposer.tsx:296-310` 无 stop 按钮；`runtime_interrupt` Tauri command 不存在；后端 `crates/runtime/src/runtime/interrupt_service.rs:7` 已实现但未被 wire |
| R2 | **Plan banner 不渲染 markdown** | `AppShell.tsx:2693` 用 `whitespace-pre-wrap`；live `evt_23059` 实际含表格 + emoji `1️⃣` |
| R3 | **Permission UI 不显示命令内容** | `RightInspector.tsx:319-321` 只显示 `tool_id + permission_id`；用户无法判断是 `rm -rf` 还是 `ls` |
| R4 | **`agent.loop_budget_reached` / `agent.loop_recovery` 完全不可见** | 14 个 agent recovery 事件 0 UI；`agent_kernel/turn_controller.rs:260,301` 等 |
| R5 | **`context.compaction.*` 系列 (70+/turn) 完全不可见** | 用户不知道 context 何时压缩、为什么不压缩；`native_turn_controller.rs:332,393` |
| R6 | **Continue 按钮永远发英文** | `AppShell.tsx:833` hard-code `"Continue the current session using prior context."` — 模型会"看到"它，UI 是 zh-CN |
| R7 | **autonomy_mode 启动后无法切换** | `parse_autonomy_mode` `main.rs:670` 只在 `start_session` 用；Topbar 无 toggle；Tauri 无 `set_autonomy_mode` command |
| R8 | **`model.call_blocked` 未知 gate 显示原文** | `AppShell.tsx:168` `messageFromRuntimeError` 未知 gate fall through 到 `payload.gate ?? payload.error_code`，toast 显示 `"model.call_blocked"` |
| R9 | **handleSubmit 失败丢用户输入** | `AppShell.tsx:1722` 先 `setInputValue("")` 后 `submitPrompt` — submit 抛错时输入永久丢失 |
| R10 | **stream chunk 触发整 transcript 重建** | `Transcript.tsx:209` `useMemo` deps 是 `messages.slice(-n)` 新数组引用，memo 永不命中；18175 stream_delta = 18175 次树重建 |

### 🟡 Yellow（影响体验但能凑合，sprint-2 修）

| # | 问题 | 证据 |
|---|---|---|
| Y1 | request_revision feedback 硬编码 | `AppShell.tsx:1982` `"manual revision requested"` 无输入框，等于盲拒 |
| Y2 | 多个 plan_approval 只显示第一条 | `AppShell.tsx:2450` `pendingPlanApprovals[0]` — 排队 UI 缺 |
| Y3 | Permission 缺 `deny_with_suggestion` + `allow_project_rule` UI | `RightInspector.tsx:334-351` 只 3 按钮；后端 `main.rs:684,686` 支持但未暴露 |
| Y4 | Permission 决策不记历史决策值 | `AppShell.tsx:1395-1399` 只记 `permission_id` 不记 allow/deny |
| Y5 | 14 dead branch 监听后端从不发的事件 | `AppShell.tsx:1302` `tool.dispatched`、`:1254` `tool.permission.evaluated`、`:1178-1219` `thinking.chain.*`、`:226-286` `subagent.*` 等 |
| Y6 | 双 emit 分裂 | `agent.tool.completed` vs `tool.call_completed` 前端只听后者；recovery 路径走前者 → 静默丢 |
| Y7 | push + poll 双轨 dedup race | `AppShell.tsx:2287-2298` push active 时仍 1.2s poll；poll 路径事件无 `event_id` → 用 fallback key → 与 push key 不同 → **重放** |
| Y8 | `ensureTauriSubscription` 反复重订阅 | `AppShell.tsx:2300-2305` deps 链含 `applyRuntimeEvents`，重建触发 unlisten/listen → **丢期间事件** |
| Y9 | `deepseek.tool_call.partial` (174/turn) 无 loading affordance | 用户看不到工具参数正在 stream 中 |
| Y10 | `runtime.plan_approval.model_continued` 静默 | 用户 approve 后看不到"模型已继续"反馈 |
| Y11 | Mic 按钮永久 disabled | `BottomComposer.tsx:296-303` |
| Y12 | 8 处死按钮 | EmptyState suggestedTasks (`:42-49`)、Transcript 查看更改 (`:647`)、ArtifactsTab (`:399-410`)、再显示 N 个 (`:412-416`)、Onboarding 安装指南 link (`:152,188`)、Sidebar 插件/自动化 (`:107-127`) |
| Y13 | API key 明文存 localStorage | `App.tsx:36/95`；文案 `OnboardingScreen.tsx:116-118` 说"仅保存在本地"，实际不是 keychain，webview XSS 可读 |
| Y14 | onboarding 不探活 API key | `OnboardingScreen.tsx:88-95` 错 key 也能进工作台，第一次发消息才 401 |
| Y15 | localStorage runStore 每 chunk 全量序列化 | `AppShell.tsx:2169-2175` 写 20 个 run 全量 JSON.stringify，流式中持续 IO |

### 🟢 Green（轻度优化，sprint-3 修）

| # | 问题 | 证据 |
|---|---|---|
| G1 | 220ms finalize timer 跨 session 风险 | `AppShell.tsx:1226` 切 session 时 timer 未强 clear |
| G2 | `model.context_budget` (70/turn) 无 context 压力 bar | `model_io.rs:150` 携带 prompt_tokens / budget |
| G3 | `convergence.disagreement` 无信号 | 用户看不到"agent 决定再 loop"的判断 |
| G4 | `deepseek.cache.zone_*.{hit,miss}` 无成本反馈 | 56 hit + 12 miss 在 sample 中 |
| G5 | `tool.input_repaired` / `tool.auto_recovery` 静默修改 | 工具参数被服务端修过，用户结果看着"不对" |
| G6 | Logout 无确认弹层 | `Sidebar.tsx:262-273` 一键清 |
| G7 | 关 app 不取消 in-flight turn | `main.rs:275` spawn_blocking 无 cancel token，turn 跑到完才止 |
| G8 | mock.ts 459 行死数据捆进 bundle | 除 suggestedTasks 外 0 引用 |
| G9 | 无 aria-label / keyboard a11y | overlay 不能 Esc 关 |
| G10 | 三处独立推导 currentRun | `AppShell.tsx:2113/2185/2319` — 逻辑漂移风险 |

---

## 2. 修复路线图（3 个 Sprint）

### Sprint-1 · "能用"（1 周）— 目标：解决 R1-R10

| Day | PR | 内容 |
|---|---|---|
| D1 | desktop/fix-cancel-button | 新增 Tauri `runtime_interrupt_session` command（wire `InterruptService.interrupt()`）；BottomComposer 加 Stop 按钮（streaming 时替换 Send）；ESC 触发 cancel |
| D1 | desktop/fix-handlesubmit-loss | `AppShell.tsx:1722` 改 submit 成功才清 input；失败保留 `lastFailedInput` |
| D2 | desktop/fix-plan-markdown | `AppShell.tsx:2693` PlanApprovalBanner 改用 ReactMarkdown + remarkGfm；高度上限改 viewport-aware |
| D2 | desktop/fix-permission-context | `runtime_facade::stream_pending_permissions` 增加 `args_preview` / `path_preview` 字段；`RightInspector.tsx:313-353` 渲染命令文本（用 `<code>` 包裹 + 截断 + hover 展开） |
| D3 | desktop/fix-recovery-visibility | AppShell 新增 14 个 agent recovery / context.compaction 事件 handler → 折叠式信息条（顶栏 spinner + 详情可展开） |
| D3 | desktop/fix-continue-prompt-zh | `AppShell.tsx:833` Continue 改 i18n（zh-CN: "基于上一轮上下文继续"；en-US: 现状）；模型实际收到的 prompt 不变（保持英文以保 cache hit） |
| D4 | desktop/fix-autonomy-mode-toggle | 新增 Tauri `set_autonomy_mode(session_id, mode)` command；Topbar 加 mode picker（5 模式） |
| D4 | desktop/fix-call-blocked-unknown-gate | `AppShell.tsx:168` 补 unknown gate 默认文案 `"模型调用被拒绝（gate: X，详情见控制台）"` + 把 gate string 写入 transcript system message |
| D5 | desktop/fix-transcript-memo | `Transcript.tsx:209` useMemo deps 改 `[messages.length, lastMessage.id, lastMessage.contentHash]`；ToolActivityBubble/MessageBubble 自定义 memo equality |
| D5 | desktop/fix-stream-coalesce | localRuntimeClient stream_delta 在 30-60ms window 合并；只在 frame boundary flush setState |

**Sprint-1 验收**：
- 100 个 tool call 的长 session 跑通，输入框不卡（实测 typing latency < 50ms）
- 用户能 cancel 任意 turn（< 200ms 真的停）
- Plan banner 显示 markdown 表格/emoji 正常
- Permission card 展示实际命令文本（`shell.command` 看得到 cmd args）
- 用户能在顶栏切 autonomy mode

### Sprint-2 · "顺手"（1-2 周）— 解决 Y1-Y15

| 主题 | PR 组 |
|---|---|
| Plan & Permission 完整化 | request_revision 加输入框（Y1）；plan_approval 队列 UI（Y2）；permission 加 deny_with_suggestion + allow_project_rule 按钮（Y3）；permission 决策记历史值（Y4） |
| Event 接线收敛 | 删 14 dead branch（Y5）；合并 dual emit（Y6）；停 push active 时的 poll（Y7）；fix 反复重订阅（Y8） |
| 工具进度可见 | `deepseek.tool_call.partial` → 工具卡 loading bar（Y9）；`plan_approval.model_continued` → toast/checkmark（Y10） |
| 死按钮清理 | EmptyState/Transcript/Artifacts/Sidebar 死按钮去掉或实现（Y12）；Mic 移除或标 alpha（Y11） |
| 安全 + Onboarding | API key 走 tauri-plugin-keyring（Y13）；onboarding 加 ping 验活 + provider 文档链接（Y14） |
| Storage 性能 | localStorage 写入 debounce + 按 run id 增量（Y15） |

### Sprint-3 · "细节"（1 周）— 解决 G1-G10 + 启动 architecture refactor

| 主题 | 内容 |
|---|---|
| State machine refactor | AppShell.tsx 拆分（context + hooks + reducer）；首先抽 `useRuntimeSession` / `useRuntimeEvents` / `usePermissionFlow` 三个 hook |
| 监控/透明性 chip 系列 | context 压力 bar（G2）；convergence 决策 chip（G3）；cache hit rate chip（G4）；input_repaired 静默警告（G5） |
| Safety nets | Logout 确认（G6）；关 app 时优雅 cancel（G7）；timer cross-session protect（G1） |
| Bundle 减重 | 删 mock.ts 死数据（G8） |
| A11y baseline | overlay 加 ESC trap、focus management、aria-label（G9） |
| 单一真相收敛 | currentRun 三处推导合一（G10） |

---

## 3. 详细发现（按主题）

### 3.1 Stop / Cancel 缺失（最痛）

**症状**：用户发出问题后，无法中途取消——只能等流式完成或杀 app。

**后端现状**：`crates/runtime/src/runtime/interrupt_service.rs:7` `InterruptService::interrupt()` 已实现，Phase 6.1.f 落地。但**没有任何 Tauri command 调用它**——`desktop/src-tauri/src/main.rs` 13 个 command 中无 `interrupt` / `cancel`。

**修复**：
- 新增 `runtime_interrupt_session(session_id) -> Result<()>` Tauri command（10 行）
- BottomComposer 在 `isStreaming=true` 时把 Send 按钮替换为 Stop 按钮（图标 `Square` from lucide-react）
- 全局 ESC 键监听：若有 running session 且无 modal 打开 → 触发 cancel
- toast 反馈 "已中断" + 历史 transcript 标 `system: 用户取消`

**估时**：4 小时

### 3.2 Plan Approval UX

**已做对**：
- Banner 居中弹层 + 顶部状态栏同步 (`AppShell.tsx:2579-2587` + `PlanApprovalBanner`)
- inFlight loading + error display
- 不被 transcript 滚动覆盖

**做错的**：
1. **plan_preview 是 plain text**（`AppShell.tsx:2693` `whitespace-pre-wrap`）→ live `evt_23059` 含 `## 标题 / | 表格 | / 1️⃣ emoji`，渲染成竖线纯文本
2. **request_revision feedback 硬编码** "manual revision requested"（`AppShell.tsx:1982`）→ 用户无法告诉模型"为什么不批，改哪里"
3. **多个 pending 只显示第一条**（`AppShell.tsx:2450`）→ 排队感知缺失
4. **切回旧 run 丢 plan_preview ref 缓存**（`AppShell.tsx:1783` `planPreviewByApprovalIdRef.current.clear()`）→ 只剩 goal，markdown 丢

**修复方向**：
- 用 `<ReactMarkdown remarkPlugins={[remarkGfm]}>` 包裹 plan_preview
- Banner 加输入框（`<textarea>` 200 字以内）作为 request_revision 的 reason，传给后端
- 多 pending 时用 tab / stack UI（首条 active，余下次第呈现）
- 切 run 时不要 clear ref；按 `(session_id, approval_id)` 分桶持久

### 3.3 Permission UX 全链路缺口

**最危险**：用户看到 "需要权限：shell.command (native_loop_v2_perm_xxx)"，**完全不知道是 rm -rf 还是 ls**。

证据：
- `RightInspector.tsx:319-321` 只展示 `tool_id` + `request_type` + `permission_id`
- 后端 `runtime_facade::stream_pending_permissions` 返回的 `PendingPermission` struct 不含 args/path 字段
- 等于"闭眼批权限"

**修复**：
1. 后端 `PendingPermission` 加字段：`args_preview: String`（前 200 字符）+ `path_preview: Option<String>`（如果是文件操作）+ `risk_level: String`（low/medium/high/critical）
2. 前端 RightInspector permission card：
   - `<code className="text-xs">{args_preview}</code>` 用等宽字体展示
   - 长 args 折叠 + hover 展开
   - 风险等级用 colored badge
3. 加 4 个决策按钮（当前只 3 个）：
   - Allow Once（现有）
   - Allow Session（现有）
   - **Allow Project Rule**（新，写 TSV 规则文件）
   - **Deny with Suggestion**（新，带 textarea）
   - Deny（现有）
4. 加键盘 hotkey：Y=allow once / A=allow session / N=deny

### 3.4 Recovery / Loop 状态完全不可见

**最严重 systemic 缺口**。14 个 agent recovery 事件 **0/14 surface**：

```
agent.loop_recovery (34/turn)
agent.loop_budget_reached     ← "agent 已经放弃" 的硬信号
agent.loop_budget.normalized
agent.loop_incomplete
agent.loop_plateau_finalized
agent.convergence_escalation
agent.recovery.{started,completed,blocked,escalated}
agent.fast_auto_write.{completed,recovery}
agent.continuation_summary
agent.visible_finalizer.failed
```

每一个都是用户反馈"为什么停了不知道"/"为什么在循环不知道"的根源。

**修复**：
- 顶栏新增 `RecoveryStatusChip` 组件：subscribe 上述 14 个事件
- 形态：spinner + 文字 "正在恢复 (loop_recovery #34)" / 警告图标 + "已达迭代上限 (15/15)"
- 点击 chip 展开右侧 inspector "Recovery 历史" tab，列时间轴
- 后端可考虑加 `agent.recovery.batched` aggregator（避免 chip 每 chunk 闪烁）

### 3.5 Compaction 透明性

**症状**：用户问"为什么 context 不被压缩"——后端**每 turn 都发** `context.compaction.skipped`（70 次在 sample 中），原因 `below_threshold`，但前端 **0 处理**。

**修复**：
- 顶栏加 `ContextPressureBar`：消费 `model.context_budget`（含 prompt_tokens / budget_remaining）
- 触发 `context.compaction.started` → bar 变橙
- `context.compaction.completed` → bar 重置 + 小 toast "已压缩 X% tokens"
- `context.compaction.blocked` → bar 变红 + 持续 banner "上下文超限，请新建会话"
- `context.compaction.skipped (below_threshold)` → 不打扰，只在 inspector 详情可见

### 3.6 后端事件 vs 前端 dispatch 覆盖率

**完整覆盖率矩阵** 见 Subagent B 报告。摘要：

| 前缀 | 后端数 | 前端处理 | 缺失严重度 |
|---|---|---|---|
| `plan.*` | 5 | 3 (mode_entered / approval_requested / approval_decided) | 高（mode_exited 静默） |
| `permission.*` | 4 | 2 (requested / decided) | 中（decision.recorded 重复但可忽略） |
| `context.compaction.*` | 5 | 0 | **最高** |
| `agent.turn.*` | 6 | 1 (telemetry.turn_summary) | 高 |
| `agent.loop_*` / `agent.recovery.*` | 14 | 0 | **最高** |
| `tool.*` | 25+ | 5 (核心 lifecycle OK) | 中（recovery/repair 系列缺） |
| `turn.*` | 4 | 0 | 中 |
| `model.*` | 12 | 5 (基础) | 中（context_budget / continuation_strategy 缺） |
| `session.*` | 5 | 1 (state_changed) | 中（forced_transition 是异常信号） |
| `deepseek.*` | 15+ | 5 (部分 cache/dsml/protocol) | 中（dsml.leak/tool_call.partial 缺） |
| `runtime.*` | 12+ | 4 (error / permission_resume) | 中（plan_approval.model_continued / write_intent_fallback 静默） |

**Dead branches**（前端监听但后端从不发）需要清理：
- `AppShell.tsx:1302` `tool.dispatched`
- `:1254` `tool.permission.evaluated`
- `:1339` `tool.completed`
- `:209` `model.call_recovery_planned`
- `:252-256` `deepseek.cache_plan.*` / `cache_stats.recorded`
- `:226-286` `reasoning.*` / `subagent.*`
- `:249` `hook.DsmlFallbackTriggered`
- `:1178-1219` `thinking.chain.{started,delta,completed}`

这些是早期 mock/计划的残留，已不可达——删除或确认后端补上。

### 3.7 AppShell god component

**症状**：`AppShell.tsx` 2200 行，26 useState + 16 useRef，所有事件 reducer 逻辑、所有 callback、所有 derive 都堆在此。

**反模式**：
- 三处独立推导 currentRun（`:2113-2126` 持久化 useEffect / `:2185-2198` 写盘 useEffect / `:2319-2332` useMemo）
- inputValue 从 AppShell 一路 prop 传到 BottomComposer，每按键触发整树 reconcile
- runStore (localStorage) + sessionRecord (磁盘) + runtime snapshot 三份真相
- `applyRuntimeEvents` (262 行) 用 useCallback 包，deps 链触发反复 subscribe

**Refactor 提议（Sprint-3 启动）**：

```
desktop/src/
  hooks/
    useRuntimeSession.ts    — 持有 sessionId / bootstrap / configure_provider
    useRuntimeEvents.ts     — 单一订阅源（push or poll，不双轨）
    usePermissionFlow.ts    — pending permissions + plan approvals state machine
    useTranscript.ts        — messages + visibleLimit + stream coalescing
  contexts/
    RuntimeContext.tsx       — provides session + events handle
    UIContext.tsx            — modal/banner/keyboard shortcut bus
  reducers/
    runtimeEventReducer.ts   — 把 262 行 if/else 改 reducer
  AppShell.tsx (目标 ≤ 400 行)
```

不需要一次性重构。Sprint-3 只抽 3 个 hook（useRuntimeEvents / usePermissionFlow / useTranscript），后续 sprint 渐进。

### 3.8 性能瓶颈（已修 B3，但有遗留）

已修：stream_events IPC payload 不再全量传 jsonl；stream chunk 合并；Transcript window 真虚拟化。

遗留（Sprint-2/3）：
- `AppShell.tsx:2169-2175` localStorage 写 20 个 run 全量序列化，每 chunk 触发
- `main.rs:1108` push 每条 event 都 JSON parse + emit，无 batching
- `Transcript.tsx:622-688` ReactMarkdown 流式中每 delta 全量 re-parse markdown（应只在 stream_completed 后切换）
- `Transcript.tsx:392,517` `memo(ToolActivityBubble/MessageBubble)` 的 `group` prop 每次重建新对象 → memo 失效

### 3.9 Onboarding + 安全

**最危险的安全错位**：`OnboardingScreen.tsx:116-118` 文案 "密钥仅保存在本地"，实际存在 `localStorage` (`App.tsx:95`)——**任何 webview XSS 可读**。文案隐含的安全承诺与实现不符。

**修复**：
- `tauri-plugin-keyring` 接 OS keychain
- 或退而求其次，存在 `~/.argon_agent/credentials.toml`（仅 user 可读 0600）
- 文案改为 "密钥保存在系统钥匙串 / 本地受保护文件"

**Onboarding 不探活**：`OnboardingScreen.tsx:88-95` 输入错的 key 也能进工作台，第一次发消息才 401。修复：提交时调 `runtime_health_check_provider(provider, env_var)` 命令，后端真发一次轻量 ping。

### 3.10 死按钮清理（8 处）

| 文件:行 | 按钮 | 处置 |
|---|---|---|
| `EmptyState.tsx:42-49` | suggestedTasks | 实现：点击填入 BottomComposer + 自动 focus |
| `Transcript.tsx:647` | 查看更改 | 实现：跳右 inspector "更改" tab；或暂时移除 |
| `RightInspector.tsx:399-410` | ArtifactsTab artifact 卡 | 实现：点击调 Tauri `open_artifact_in_finder` |
| `RightInspector.tsx:412-416` | 再显示 N 个 | 实现 pagination |
| `OnboardingScreen.tsx:152,188` | 查看安装指南 / 阅读快速开始 | 替换 `href="#"` 为真实 docs URL，或移除 |
| `Sidebar.tsx:107-127` | 插件 / 自动化 面板 | 暂时移除入口或标 "即将推出"，避免误导 |
| `BottomComposer.tsx:296-303` | Mic 按钮 | 暂时移除（语音功能未实现）或标 alpha |
| `Sidebar.tsx:262-273` | Logout（无确认） | 加 confirm dialog |

---

## 4. 与上游 Phase 9 的协调

D1 决策已明确 Phase 9 删除 Electron + Python local_api_server。本审计的修复**不依赖** Phase 9，但有两处协调：

| 桌面修复 | Phase 9 影响 |
|---|---|
| Stop/Cancel 按钮 wire Tauri command | Phase 9 删 Electron 后 `localRuntimeClient.ts` 简化 → Cancel 接线更干净，建议**先做 Phase 9 再 Sprint-1** |
| State machine refactor | 与 Phase 9 都改 `localRuntimeClient.ts`；Sprint-3 启动前要等 Phase 9 合并 |

**推荐节奏**：

```
Phase 9 (D-Phase 0)        ← 1-2 天先做，清掉 Electron + Python server
   ↓
Desktop Sprint-1 (能用)    ← 1 周
   ↓
Desktop Sprint-2 (顺手)    ← 1-2 周
   ↓
Desktop Sprint-3 (细节 + refactor)  ← 1 周
```

总 3-4 周。期间内核侧 Phase 8.2+ 可并行（不冲突）。

---

## 5. 验收标准（"能用"的最小定义）

8 项中需 8/8 通过才算 Sprint-1 完成：

| 维度 | 验收 |
|---|---|
| 启动 | `npm run tauri:dev` 一次成功，无 console error |
| 首次对话 | 输入问题 → 流式回答 → 正常结束（不卡死、不丢字、不重复发送） |
| 工具调用 | tool call 显示完整、permission 提示能响应、结果不串行错位、**args/path 可见** |
| 取消 | **流式中按 Cancel 真的停**（实测 < 200ms） |
| 多轮 | 第 2 轮 / 第 3 轮不破坏上一轮上下文 |
| 会话切换 | sidebar 切 session 不丢消息、不展示别的 session 内容 |
| 错误 | provider 报错 / permission denied 时 UI 不白屏、**有明确提示**（不是 raw event_type 字符串） |
| 不崩 | **100 工具调用 / 30 分钟连续使用不需要重启 app** |

---

## 6. 不在本审计范围

- 视觉系统（colors / typography）— 等 Sprint-1/2/3 后单独 design epic
- 命令面板 / cmd+k / 全局搜索 — 见原计划 D-Phase 5
- 多窗口 / tray icon — 后续 epic
- session 历史 time-travel — 见原计划 D-Phase 3
- 双模型对比 — 见原计划 D-Phase 4
- 中文/英文 i18n 切换 — 现阶段全 zh-CN

---

## 7. 关键文件清单（修复时常用）

| 文件 | 行数 | 主要职责 |
|---|---|---|
| `desktop/src/App.tsx` | 117 | 顶层 shell + onboarding gate |
| `desktop/src/components/AppShell.tsx` | **2200** | god component，所有事件 dispatch + state |
| `desktop/src/components/Transcript.tsx` | ~700 | 消息渲染 + stream + tool bubble |
| `desktop/src/components/BottomComposer.tsx` | ~400 | 输入框 + 模型菜单 + slash command |
| `desktop/src/components/RightInspector.tsx` | ~450 | permission cards + progress + artifacts |
| `desktop/src/components/Sidebar.tsx` | ~300 | session list + settings menu |
| `desktop/src/components/Topbar.tsx` | ~150 | 状态 + continue / retry / export |
| `desktop/src/components/OnboardingScreen.tsx` | ~250 | 首次配置 |
| `desktop/src/runtime/localRuntimeClient.ts` | 557 | Tauri/Electron/HTTP 三路 transport client |
| `desktop/src-tauri/src/main.rs` | 1552 | 13 Tauri commands + push pipe |
| `crates/runtime/src/runtime/interrupt_service.rs` | ~150 | 后端 cancel signal（待 wire） |

---

## 8. Sprint-1 第一周可立即开工的 PR

按 ETA 排序，每个 PR 独立可合：

1. **PR-D1.1 Stop/Cancel 按钮 + Tauri command**（4h，最高优先）
2. **PR-D1.2 handleSubmit 失败不丢输入**（30min）
3. **PR-D1.3 Plan banner 渲染 markdown**（1h）
4. **PR-D1.4 Permission card 展示 args/path**（4h，需后端补 PendingPermission 字段）
5. **PR-D1.5 Recovery / loop_budget chip**（6h）
6. **PR-D1.6 autonomy_mode 顶栏切换 + Tauri command**（3h）
7. **PR-D1.7 Continue prompt i18n**（30min）
8. **PR-D1.8 model.call_blocked unknown gate 文案**（30min）
9. **PR-D1.9 Transcript memo 修复（性能）**（2h）
10. **PR-D1.10 stream_delta coalesce**（2h）

**总计** ≈ 24h 实际工作 = 1 周（含测试 + bug 抓回归）。

---

## 9. 立即可执行

不需要更多决策。如果你 ready 启动，告诉我：

- 执行模式：**A** 你手动跑 PR-D1.1 到 1.10？**B** 给 Codex 整套指令让它跑？**C** 你跑前 3 个最痛的 (cancel / markdown / permission args)，剩下委托？
- 是否先做 Phase 9 清理（1-2 天）再启 Sprint-1？

不要再要"计划"——这份文档就是计划。等你选择执行方式即可。
