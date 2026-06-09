# D2 · HTTP transport 现代化：Phase 8 引入 tokio + reqwest，但**保留 Python sidecar 作为 dual-run 对照**

> 状态：**决定**（2026-05-25）
> 决策者：architecture review
> 触发：架构整合 Phase 0
> 后续 phase：Phase 8（HTTP transport 现代化）

---

## 决策

**Phase 8 在 runtime 中引入 tokio + reqwest，新增 `ReqwestLiveHttpTransport`。但 `PythonSidecarLiveHttpTransport` 不立即删除——以 feature flag 形式保留至少 90 天作为双跑对照。**

修正初版 Phase 8 设想：原文档说"sidecar 标 deprecated 30 天后删除"，过激进；实测 sidecar 不只是 HTTP wrapper，还内含 2166 行的流式协议解析。

---

## 现状证据

### Sidecar 实际职责（被低估的复杂度）

`scripts/provider_http_sidecar.py` ≠ "shell out HTTP"。逐行检查发现：

- 支持 3 种 mode：`request`、`health_check`、`stream_visible_text`
- `stream_visible_text` 自己解析 SSE，区分 `thinking` / `visible` deltas（[`provider_http_sidecar.py:129-208`](../../scripts/provider_http_sidecar.py)）
- OpenAI / Anthropic / DeepSeek 三家 provider 的 `choices[].delta` 与 `message.content` 拼装在 Python 端（[`:317-358`](../../scripts/provider_http_sidecar.py)）
- API key 仅从环境变量读取（`authorization_env`），密钥从不离开 Python 进程；这是已实施的**安全边界**

Rust 端 `sidecar_http_transport.rs:220-281` 仅 `Command::new(python_bin).arg(script).stdin(json).spawn()`，**每次请求 spawn 一次 python3**。

### 当前同步约束

- 整个 `crates/runtime/Cargo.toml` **零** tokio / async-std / smol / reqwest 依赖
- `LiveHttpTransport::send` 是同步 trait（`fn send(&self, ...) -> Result<..., String>`）
- 主循环靠 `std::thread::spawn` 做并发（`local_api_server.rs:111` 接受连接 + 每连接独立线程）

### 性能与正确性现状

- 每次模型调用：fork python3（~150-300ms 启动） + interpreter warm + 流式 SSE 解析 → 显著首字节延迟
- Cancellation：Rust 端 `&AtomicBool` 只在 scripted transport 检查；**spawn 出去的 python3 不被 kill**，按 Ctrl+C 后子进程继续跑到完
- 跨平台：Windows 上 `python3` 命令名不一致；容器部署强制装 Python

---

## 候选方案

### 方案 A：纯 tokio + reqwest（激进）

立即用 reqwest async 重写 transport，python sidecar 30 天后删。

代价：

- 必须重写 SSE 解析（OpenAI / Anthropic / DeepSeek 三家 stream 格式不同）
- 必须重新建立 API key 隔离边界（Python 端的 `authorization_env` → Rust 端的 secrets 不写入 event log 不进 trace）
- async 改造需要 `Send` / `Sync` 全面调整（runtime 现有 `Rc`/`RefCell`/`!Send` future 全部审查）
- 第一版 bug 多，没有对照路径排查

### 方案 B：保留 sidecar，不做 tokio（保守）

不动现状，承担 Python 启动开销和 cancellation 缺失。

代价：

- doc39 / Phase 7 的 Compactor / Phase 5 的权限双跑事件流，每个 turn 多 ~200ms 启动开销
- Ctrl+C 体验在用户层永远不会变好
- 模型 SDK 升级（如 deepseek 新 reasoning 协议）必须改 Python，Rust 端跟着改 schema
- 长期看，Rust runtime 永远要被 Python "拽着"

### 方案 C：tokio + reqwest 与 sidecar 并存，feature flag 切换（决定）

Phase 8 引入 tokio + reqwest，新增 `ReqwestLiveHttpTransport`；sidecar 保留 90 天作为 feature flag 可切换的 fallback。

收益：

- 双跑对照可以**逐 provider 验证**（先 DeepSeek，后 Anthropic，最后 OpenAI）
- bug 发现快：reqwest 返回的 stream 与 sidecar 返回的 stream，逐 chunk diff 对比
- 回滚成本低：`--transport=sidecar` 一行环境变量切回去
- 90 天内若发现 reqwest 路径有未预料问题（如 Anthropic SSE 边界）可以延期，不被迫硬切

代价：

- 90 天内维护两个 transport 实现
- 一次性引入 tokio 是 Rust 项目最大依赖事件，需要 dedicated PR + 充分测试

---

## 选择：方案 C

理由：

1. **不可逆度**：引入 tokio 是 Rust 项目的"地壳运动"。任何函数加 `async` 都会传染上游。这种规模的改造必须有双跑回滚路径。
2. **安全边界已被实施**：Python sidecar 的 `authorization_env` 机制是真实的、已被 `sidecar_rejects_secret_like_authorization_env_before_spawn` 测试验证的安全设计。Rust 端重做要等价证明。双跑期间用 sidecar 兜底。
3. **doc39 北极星不要求一次到位**：Phase 8 的目标是"现代化"，不是"清洁"。先解锁 cancellation + 性能，干净留给后续。
4. **协议演进风险**：DeepSeek 的 reasoning_content / Anthropic 的 thinking blocks 还在演化。Python sidecar 改起来快，作为新协议落地的"前哨站"是合理过渡形态。

---

## 落地路径（Phase 8 细化）

### 8.1 引入 tokio 依赖（独立 PR，不带任何逻辑变更）

```toml
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "signal"] }
tokio-util = "0.7"  # 仅为 CancellationToken
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "stream", "json"] }
```

**这一个 PR 只动 Cargo.toml + 通过 cargo build。** 任何 .rs 改动单独 PR。
不偷渡：**禁止**在同一 PR 写任何 `.await`。

### 8.2 扩展 `LiveHttpTransport` trait（独立 PR）

```rust
pub trait LiveHttpTransport: Send + Sync {
    fn send(&self, ...) -> Result<LiveHttpResponse, String>;  // 现状保留
    fn send_async(&self, ...) -> BoxFuture<'_, Result<LiveHttpResponse, String>>;  // 新增
}
```

`PythonSidecarLiveHttpTransport::send_async` 用 `tokio::task::spawn_blocking` 包同步实现——保证不需要先重写 sidecar。

### 8.3 新增 `ReqwestLiveHttpTransport`

- 只支持 `request` 和 `stream_visible_text` mode
- 实现 SSE 解析（用 `eventsource-stream` 或手写）
- API key 处理：仅 `std::env::var(&authorization_env)`，不进任何 log / event / trace
- 单元测试：与 sidecar 输出对每个 provider 至少 10 个 fixture 逐 byte 对比

### 8.4 RuntimeFacade 加 transport 选择

```rust
let transport: Arc<dyn LiveHttpTransport> = match env::var("RESEARCHCODE_TRANSPORT").as_deref() {
    Ok("reqwest") => Arc::new(ReqwestLiveHttpTransport::new(...)),
    _ => Arc::new(PythonSidecarLiveHttpTransport::new(...)),  // 默认 sidecar
};
```

**默认仍是 sidecar，新路径靠 env opt-in。**

### 8.5 双跑验证（14 天）

每个请求并行发两路，比对：

- HTTP status code
- 响应 body byte 长度（±1% 允许 chunk 边界差）
- 流式 chunk 序列（visible 文本拼接后必须 byte-identical）
- thinking deltas 数量与顺序
- token usage 字段一致

diff 计数计入 telemetry：`transport.dual_run.diff_count`。

### 8.6 Cancellation 升级

- 新增 `Arc<CancellationToken>` 作为 RuntimeFacade 字段
- ReqwestLiveHttpTransport 用 `tokio::select!{ res = req.send() => ..., _ = token.cancelled() => ... }`
- Sidecar 路径：Phase 8 不改（已知缺陷）；Ctrl+C 行为差异计入文档
- `native_agent_loop_entrypoints.rs:7,19,188,327` 的 `AtomicBool::new(false)` 硬编码替换为 token

### 8.7 默认切换（≥ 14 天双跑 + diff_count = 0 后）

- 默认 transport 改为 reqwest
- sidecar 标 `#[deprecated]`
- 保留 90 天可 opt-in，期间观察 deprecated warning 数 vs 实际切换数

### 8.8 sidecar 删除（90 天后独立 PR）

满足全部条件才执行：

- 默认 transport = reqwest 已 90 天
- `transport.dual_run.diff_count` 累计 < 5（连续 90 天）
- 三家 provider（DeepSeek / Anthropic / OpenAI）至少各 1000 次真实请求覆盖
- 安全审计通过（API key 不出现在任何 Rust 端 log / event）

否则推迟 30 天复评。

---

## 验收

| 时点 | 指标 | 目标 |
|---|---|---|
| Phase 8.1 完成 | `cargo build` 通过、无 .rs 改动 | binary 大小变化记录 |
| Phase 8.3 完成 | reqwest fixture 对比 sidecar diff | 0 |
| Phase 8.5 完成 | 14 天 dual-run | diff < 0.1% |
| Phase 8.6 完成 | Ctrl+C 实测延迟 | < 200ms（reqwest 路径） |
| Phase 8.7 完成 | 模型请求 P50 延迟变化 | 下降 ≥ 100ms（sidecar 启动开销消除） |
| Phase 8.8 完成 | `find scripts -name "provider_http_sidecar.py"` | 空 |

---

## 撤销代价

| 阶段 | 撤回代价 |
|---|---|
| 8.1 引入 tokio 后 | 极高——tokio 一旦进入，依赖树膨胀；建议视为不可撤回 |
| 8.4 双跑期 | 低——`unset RESEARCHCODE_TRANSPORT` 即回 sidecar |
| 8.7 默认切换后 | 中——默认值改回 sidecar，1 行 |
| 8.8 sidecar 删除后 | 高——需 git revert 找回 Python 脚本 + 重新接入 |

**关键约束**：8.1 是不可逆点。在 8.1 之前必须确认本决策不被反悔。

---

## 反对意见与回应

**反 1**：Rust 端重写 SSE 解析风险高，sidecar 已经稳定多月，为什么要动？
**回应**：sidecar 的 cancellation 缺失是用户体验硬伤；Phase 8 不是"cleanup"，是"解锁 cancellation + 性能"两个真实痛点。双跑期保证安全。

**反 2**：tokio 引入会让所有 fn 变成 async fn，runtime 改造无穷无尽。
**回应**：Phase 8.2 的策略是 `BoxFuture` + `spawn_blocking` 共存——同步路径不被强迫 async 化。只有 transport 层和 cancellation 边界需要 async。

**反 3**：reqwest + rustls 在国内网络环境下未必比 Python urllib 稳。
**回应**：双跑 14 天的目的就是发现这类问题；如果证伪，回滚到 sidecar 0 代价。

**反 4**：90 天 sidecar 保留期太长，会让两套实现长期漂移。
**回应**：90 天是上限，不是下限。8.7 切换后如果 30 天 diff = 0 且无回滚事件，可以单独 PR 提前删除。

---

## 备注

- 本决策不影响 `desktop/electron/main.ts` 中的 Python `local_api_server.py`——那是 D1 的 UI server，与 model provider sidecar 是两个独立 Python 入口，按各自决策处置。
- Anthropic provider 的 `cache_control` / `extended thinking` 协议处理，Phase 8 reqwest 实现必须等价覆盖。
