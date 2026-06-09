# Phase 8 · transport 异步化落地契约

> 状态：**Opus 设计已签**（2026-05-26）
> 上游：[`docs/decisions/D2-tokio-transport.md`](../decisions/D2-tokio-transport.md)、[`docs/architecture_consolidation_plan.md` §Phase 8](../architecture_consolidation_plan.md)
> 下游：Codex 按本契约执行 8.1-8.6（**8.7-8.8 由 Opus 在 90 天 dual-run 窗口结束后单独决定**）

---

## 1. 现状

```rust
pub trait LiveHttpTransport {
    fn send(&self, request: &PreparedModelHttpRequest) -> Result<LiveHttpResponse, String>;
    fn send_with_stream_observer(
        &self,
        request: &PreparedModelHttpRequest,
        observer: &mut dyn FnMut(LiveHttpStreamEvent),
        interrupt: &AtomicBool,
    ) -> Result<LiveHttpResponse, String>;
}
```

3 个实现：`PythonSidecarLiveHttpTransport` / `RecordedLiveHttpTransport` / `ScriptedLiveHttpTransport`。

整 runtime crate 0 tokio / 0 reqwest 依赖。模型 HTTP 走 Python sidecar `Command::new("python3")`。

Cancellation: `&AtomicBool`，5+ 处硬编码 `AtomicBool::new(false)`（不可取消入口）。

## 2. 8 个子 phase 路线图（依照 D2）

| 子 phase | 内容 | 是否 Codex 范围 |
|---|---|---|
| 8.1 | 独立 PR 引入 tokio + reqwest 依赖，**0 行 .rs 改动** | ✅ |
| 8.2 | `LiveHttpTransport` 扩展 `send_async`，sidecar 用 `spawn_blocking` 包同步实现 | ✅ |
| 8.3 | 新增 `ReqwestLiveHttpTransport`，每 provider fixture 双跑对比 | ✅ |
| 8.4 | `RESEARCHCODE_TRANSPORT=reqwest` env opt-in，默认仍 sidecar | ✅ |
| 8.5 | 双跑 14 天 + `dual_run.diff_count` telemetry | ✅（实现 telemetry） |
| 8.6 | `CancellationToken` 替换 `AtomicBool::new(false)` 硬编码 | ✅ |
| 8.7 | 默认切 reqwest（Opus 看 14 天 metric 后决定） | ❌ Opus only |
| 8.8 | 删除 sidecar（90 天 dual-run 通过后） | ❌ Opus only |

Codex 范围：**8.1 → 8.6**。完成 8.6 后停下，把 dual_run metric 报告交 Opus。

## 3. 各子 phase 契约

### 3.1 · 8.1 引入依赖（单 PR，0 .rs 改动）

`crates/runtime/Cargo.toml` 追加：

```toml
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "signal"] }
tokio-util = "0.7"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "stream", "json"] }
futures = "0.3"
eventsource-stream = "0.2"
```

**约束**：
- 不写任何 `async fn` / `.await`
- 不动 trait 签名
- 必须独立 PR，PR 描述附 `cargo build` 输出 + 依赖树新增节点数
- 拒收"顺手"动 .rs 文件的 PR

验收：
- `cargo build -p researchcode-runtime` 通过
- binary 大小变化记入 PR（预期 +2-5MB）
- `cargo tree -p researchcode-runtime --depth 1` 出新依赖

### 3.2 · 8.2 扩展 trait（单 PR）

```rust
use futures::future::BoxFuture;

pub trait LiveHttpTransport: Send + Sync {
    fn send(&self, request: &PreparedModelHttpRequest) -> Result<LiveHttpResponse, String>;

    fn send_with_stream_observer(
        &self,
        request: &PreparedModelHttpRequest,
        observer: &mut dyn FnMut(LiveHttpStreamEvent),
        interrupt: &AtomicBool,
    ) -> Result<LiveHttpResponse, String>;

    /// 新增：async 入口。默认 impl 用 spawn_blocking 包同步 send。
    fn send_async<'a>(
        &'a self,
        request: &'a PreparedModelHttpRequest,
    ) -> BoxFuture<'a, Result<LiveHttpResponse, String>> {
        Box::pin(async move {
            let req = request.clone();
            tokio::task::spawn_blocking({
                let this: &Self = self;  // lifetime trick
                move || this.send(&req)
            })
            .await
            .map_err(|e| format!("transport_blocking_join_failed: {e}"))?
        })
    }

    /// 新增：async 流式入口（Phase 8.3 用，default impl 走 spawn_blocking）
    fn send_stream_async<'a>(
        &'a self,
        request: &'a PreparedModelHttpRequest,
        observer: Box<dyn FnMut(LiveHttpStreamEvent) + Send + 'a>,
        token: tokio_util::sync::CancellationToken,
    ) -> BoxFuture<'a, Result<LiveHttpResponse, String>>;
}
```

**注意 lifetime trick**：`spawn_blocking` 要求 `'static`——`PreparedModelHttpRequest` 应已是 `Clone`，clone 后 move 进闭包。Self 引用通过 `Arc<dyn LiveHttpTransport>` 持有（不要直接 `&self`）。

### 3.3 · 8.3 ReqwestLiveHttpTransport

新建 `crates/runtime/src/reqwest_http_transport.rs`：

```rust
pub struct ReqwestLiveHttpTransport {
    client: reqwest::Client,
    runtime_handle: tokio::runtime::Handle,
}

impl ReqwestLiveHttpTransport {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            client: reqwest::Client::builder()
                .use_rustls_tls()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
            runtime_handle: handle,
        }
    }

    /// Provider-specific: DeepSeek / Anthropic / OpenAI SSE 解析
    async fn dispatch_streaming(
        &self,
        request: &PreparedModelHttpRequest,
        mut observer: Box<dyn FnMut(LiveHttpStreamEvent) + Send + '_>,
        token: CancellationToken,
    ) -> Result<LiveHttpResponse, String> {
        // 1. 构造 reqwest::RequestBuilder，从 env 读 authorization_env
        let api_key = std::env::var(&request.authorization_env)
            .map_err(|_| "missing_api_key".to_string())?;
        // 2. tokio::select! 配合 token.cancelled()
        // 3. 用 eventsource-stream 解析 SSE
        // 4. 按 provider URL 选 visible / thinking 拆分逻辑
        // 5. 关键：api_key 仅用于 header，永不写入 event_log / detail_json
        ...
    }
}

impl LiveHttpTransport for ReqwestLiveHttpTransport {
    fn send(&self, request: &PreparedModelHttpRequest) -> Result<LiveHttpResponse, String> {
        // sync 入口：block_on(async 实现)
        self.runtime_handle.block_on(self.send_async(request))
    }

    fn send_async<'a>(&'a self, request: &'a PreparedModelHttpRequest) -> BoxFuture<'a, ...> {
        // 真 async 实现
    }

    fn send_stream_async<'a>(&'a self, request: &'a PreparedModelHttpRequest, observer, token) -> BoxFuture<'a, ...> {
        Box::pin(self.dispatch_streaming(request, observer, token))
    }
}
```

**Provider 分派表**（先 DeepSeek，再 Anthropic / OpenAI）：

| Provider | URL pattern | 拆分逻辑 |
|---|---|---|
| DeepSeek | `api.deepseek.com/v1/chat/completions` | `choices[].delta.reasoning_content` → thinking；`choices[].delta.content` → visible |
| Anthropic | `api.anthropic.com/v1/messages` | `content_block_delta.thinking` → thinking；`text_delta` → visible |
| OpenAI | `api.openai.com/v1/responses` | `output[].delta.text` → visible（无 thinking） |

**Phase 8.3 必须先实现 DeepSeek**——Anthropic/OpenAI 留为后续 PR（也在 Codex 范围，但分 PR）。

### 3.4 · 8.4 transport 选择

`RuntimeFacade::new` 内（或 `interrupt_service.rs` 边上）：

```rust
let transport: Arc<dyn LiveHttpTransport> = match std::env::var("RESEARCHCODE_TRANSPORT").as_deref() {
    Ok("reqwest") => {
        let runtime = tokio::runtime::Runtime::new()?;
        Arc::new(ReqwestLiveHttpTransport::new(runtime.handle().clone()))
    }
    _ => Arc::new(PythonSidecarLiveHttpTransport::default()),
};
```

**默认仍是 sidecar**，仅 env opt-in 切 reqwest。

### 3.5 · 8.5 双跑 telemetry

`Arc<dyn LiveHttpTransport>` 包一层 `DualRunTransport`：

```rust
pub struct DualRunTransport {
    primary: Arc<dyn LiveHttpTransport>,
    shadow: Arc<dyn LiveHttpTransport>,
    diff_sink: Arc<Mutex<Vec<TransportDiff>>>,
}

impl DualRunTransport {
    pub fn new(primary: Arc<dyn LiveHttpTransport>, shadow: Arc<dyn LiveHttpTransport>) -> Self;

    /// 主 path 返回结果；shadow path 在后台跑，diff 落入 sink
    pub fn diff_report(&self) -> Vec<TransportDiff>;
}

impl LiveHttpTransport for DualRunTransport {
    fn send(&self, request: &PreparedModelHttpRequest) -> Result<LiveHttpResponse, String> {
        let primary_result = self.primary.send(request);
        // 后台 std::thread::spawn shadow.send(request.clone())
        // diff: status_code / body_length / token_usage
        primary_result
    }
}
```

事件流：每次请求发 `transport.dual_run.diff` 事件，字段 `{provider, equal, diff_summary}`。CI 统计：14 天 `equal=false` 累计 < 0.1% 才进入 8.7（默认切换）。

env opt-in：`RESEARCHCODE_DUAL_RUN=1` 时启用 DualRunTransport。

### 3.6 · 8.6 Cancellation 升级

新增 `InterruptService::cancel_token`（Phase 6 已预留）：

```rust
impl InterruptService {
    pub fn cancel_token(&self) -> tokio_util::sync::CancellationToken {
        self.token.clone()  // Phase 6 预留字段
    }
}
```

替换 5+ 个 `AtomicBool::new(false)` 硬编码：

| 文件:行 | 当前 | 替换为 |
|---|---|---|
| `native_agent_loop_entrypoints.rs:7` | `&AtomicBool::new(false)` | 从 caller 传入 `&interrupt_handle` |
| `:19, :188, :327` | 同上 | 同上 |
| `live_http_transport.rs:` `&AtomicBool::new(false)` (scripted) | 同上 | scripted 测试豁免，保留硬编码 |
| `local_api_server.rs:111` `Arc::new(AtomicBool::new(false))` | 改 `Arc::clone(&interrupt_service.handle())` | |
| `agent_kernel/turn_controller.rs` `AtomicBool::new(false)` | 同上 | |

**保留**：`native_agent_loop_tests.rs` 中的硬编码（test 不取消）。

reqwest path 内部用 `tokio::select!`：

```rust
let response = tokio::select! {
    res = self.client.execute(req) => res,
    _ = token.cancelled() => return Err("cancelled".into()),
};
```

sidecar path：Phase 8 不改 sidecar 内部（python 子进程仍不可 kill）——这是已知缺陷，记入文档，Phase 8.8 删 sidecar 后自动解决。

## 4. 双跑验证脚本（Codex 实现，Phase 8.5）

`scripts/dual_run_diff_report.sh`：

```bash
#!/usr/bin/env bash
# 读 14 天的 event jsonl，统计 transport.dual_run.diff 事件
# 输出：
#   provider | total | diff_count | equal_pct
#   deepseek | 1024  | 0          | 100.0%
#   anthropic| ...
```

Phase 8.6 完成时 PR 描述附 14 天 diff_report 输出。

## 5. 安全约束（不可妥协）

| 约束 | 验证方式 |
|---|---|
| API key 仅从 env 读，永不进 event_log | grep `event_log\|append_event` in reqwest_http_transport.rs → 必须无 `api_key` / `authorization` 字段 |
| API key 永不进 detail_json | `cargo test reqwest_api_key_not_in_detail_json` |
| 错误响应不泄露 key | `cargo test reqwest_error_redacts_authorization_header` |
| TLS rustls only | `cargo tree` 不应出现 `native-tls` / `openssl-sys` |

每个 PR 描述附"安全审计 checklist"，列出上述 4 项验证。

## 6. PR 拆解

| 顺序 | PR | 范围 |
|---|---|---|
| 8.1 | Cargo.toml 引入依赖（0 .rs） | 1 PR |
| 8.2 | trait 扩展 send_async + sidecar 默认实现 | 1 PR |
| 8.3.a | ReqwestLiveHttpTransport（DeepSeek only） | 1 PR |
| 8.3.b | Anthropic provider 支持 | 1 PR |
| 8.3.c | OpenAI provider 支持 | 1 PR |
| 8.4 | RESEARCHCODE_TRANSPORT env opt-in | 1 PR |
| 8.5 | DualRunTransport + telemetry + dual_run_diff_report.sh | 1 PR |
| 8.6 | CancellationToken 替换 AtomicBool::new(false) | 1 PR |

8 PR，3-4 周。8.6 完成后 Codex 停下，等 Opus 看 14 天 metric。

## 7. 验收

| 子 phase | 验收 |
|---|---|
| 8.1 | `cargo build` 通过，无 .rs 改动 |
| 8.2 | 现有测试全绿；新增 trait fn 有默认实现，旧代码 0 改动 |
| 8.3 | 每 provider 10+ fixture 与 sidecar 输出 byte-identical |
| 8.4 | `RESEARCHCODE_TRANSPORT=reqwest cargo test` 通过 |
| 8.5 | 14 天 dual-run，`equal=false` 累计 < 0.1% |
| 8.6 | Ctrl+C → reqwest path 流式 chunk 中断 < 200ms；sidecar path 已知缺陷文档化 |

## 8. 不在 Phase 8 范围

- 不删 sidecar（8.8 由 Opus 决定）
- 不切默认 transport（8.7 由 Opus 决定）
- 不动 Python `provider_http_sidecar.py`（保持原状作为对照）
- 不动 `local_api_server.rs` 的 HTTP server 部分（那是 GUI 通信，不是模型 transport）
- 不引入 async 到 runtime 其他模块（仅 transport 边界 async；其他保持 sync）

## 9. 必须停下来报告

1. `spawn_blocking` lifetime 问题无法用 `Arc<dyn>` 解决
2. reqwest 输出与 sidecar 输出在某个 provider 上 > 0.5% diff（且非测试 fixture 问题）
3. API key 出现在任何 event 字段（哪怕是脱敏过的）—— 立即停 PR，Opus 现场 review
4. dual-run telemetry 自身性能开销 > 5%
5. CancellationToken 改造在某个 sibling 引发 sync ↔ async 边界问题
