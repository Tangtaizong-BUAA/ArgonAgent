# D1 · Desktop 宿主二选一：保留 Tauri，退役 Electron

> 状态：**决定**（2026-05-25）
> 决策者：architecture review
> 触发：架构整合 Phase 0
> 后续 phase：Phase 9（Desktop 单一路径）

---

## 决策

**保留 Tauri 作为唯一桌面宿主。Electron + Python `local_api_server.py` 路径在 Phase 9 删除。**

---

## 现状证据

### Tauri 路径（保留）

- `desktop/src-tauri/src/main.rs` — **1552 行 Rust**，**13 个 `#[tauri::command]`**
- `desktop/src-tauri/Cargo.toml` 已直接依赖 `researchcode-runtime = { path = "../../crates/runtime" }`，**同进程**调用 `RuntimeFacade`
- 依赖收敛：`tauri = "2"` + `rfd = "0.14"`（文件对话框）+ `serde/serde_json`，无网络栈
- 已在 `desktop/package.json` 提供 `tauri:dev` / `tauri:build` 完整脚本
- 前端 `desktop/src/runtime/localRuntimeClient.ts` 首选 `window.__TAURI__.core.invoke`

### Electron 路径（退役）

- `desktop/electron/main.ts` — 217 行；`preload.ts` — 11 行
- 实际行为：[`main.ts:111-112`](../../desktop/electron/main.ts) `runtimeServer = spawn("python3", scripts/local_api_server.py)`
- 与 Rust runtime **不同进程**，靠 HTTP 8765 + bearer token 通信
- Python `scripts/local_api_server.py` 2166 行，是 Rust `crates/runtime/src/local_api_server.rs`（1578 行）的旧 mock，**抢同一端口 8765**
- 依赖：`electron@^36`、`concurrently`、`wait-on`、`dist-electron` 构建产物——三套额外 build 链

### 共享部分

两个宿主共享同一个 React `src/`（`window.__TAURI__` vs `window.electronAPI` runtime sniff）；删除 Electron 不动前端代码。

---

## 候选对比

|  | Tauri | Electron |
|---|---|---|
| 与 Rust runtime 通信 | 同进程 invoke，类型直通 | HTTP/8765，需要 token 鉴权 |
| 取消信号 | 直达 Rust `&AtomicBool` / 未来 `CancellationToken` | 须穿越 HTTP，子进程 kill 无规范 |
| 包体积 | ~10MB 量级 native | ~150MB（Chromium 内嵌） |
| 模型流式延迟 | 进程内函数调用 | 多一跳 localhost HTTP + Python sidecar 二次 spawn |
| 跨平台 | macOS/Windows/Linux 直接产 | 同样支持，但安装 Node + Electron 依赖 |
| 用户路径要求 | Rust toolchain | Node + Python3 + Electron |
| 当前用户场景 | `npm run tauri:dev` 已可用 | `npm run electron:dev` 需 wait-on + concurrently 编排 |
| 安全面 | runtime crate 是唯一信任边界 | HTTP API + bearer token + Python 子进程三层信任 |

Electron 唯一胜出维度是"前端开发熟悉度"，但前端代码 100% 共享，无意义。

---

## 选择理由（按权重）

1. **删除冗余通信通道**（最关键）：现状 5 个 UI↔runtime 通道（Tauri / Electron IPC / Rust 8765 / Python 8765 / open-tui adapter）有 4 个源于"Electron 还在"。砍 Electron 直接消掉 3 个（IPC + Python server + 8765 端口冲突）。
2. **同进程取消**：Phase 8 的 `CancellationToken` 改造在 Tauri 路径里 1 行 `invoke("cancel", ...)` 直达；Electron 路径还要设计 kill 协议、reconnect、token 失效处理。
3. **不偷渡 Python 运行时依赖**：用户跑 Tauri 不需要 Python3；Electron 路径强制 Python3 存在（且版本敏感）。
4. **Tauri 已是事实主路径**：1552 行 src-tauri main.rs vs Electron 228 行——实际投入早已不对称。

---

## 反对意见与回应

**反 1**：Electron 生态更成熟，未来想加 web 集成（DevTools、扩展）更容易。
**回应**：Tauri 2 自带 WebView2/WKWebView，调试体验已接近 Electron；本项目目标是终端工程师的"argon agent"，不需要 Electron 级别的浏览器集成。

**反 2**：Tauri 在 Linux 上 WebView 实现碎片化（webkit2gtk 版本差异）。
**回应**：当前用户为 macOS（Darwin 24.6.0），Linux 是次要平台；Linux 风险在 Phase 9 复跑 smoke 时验证。

**反 3**：删 Electron 会让"已跑通"的 Python `local_api_server.py` 一起死，可能影响测试套件。
**回应**：见 Phase 1.1 / Phase 9.2 验收——所有依赖 8765 的测试要么改连 Rust `local_api_server.rs`（端口相同，路由集），要么标 obsolete。`scripts/local_api_server.py` 退场必须有独立 PR + 测试套件迁移。

---

## 落地路径

| Phase | 动作 |
|---|---|
| Phase 1.1 | 删除 `apps/desktop/` 死 stub（与 Electron 无关，但同 phase 清理） |
| Phase 9.1 | 删除 `desktop/electron/` 目录 + `dist-electron/` 构建产物 + `package.json` 中 `electron:dev` / `electron:build` 脚本 + `electron`/`concurrently`/`wait-on` devDeps |
| Phase 9.2 | 删除 `scripts/local_api_server.py` + 关联测试（`test_local_api_http.py`、`test_local_api_server.py`） |
| Phase 9.3 | `localRuntimeClient.ts` 删除 `window.electronAPI` 嗅探分支 + Electron IPC 通道 |
| Phase 9.5 | UI ↔ runtime 通道清单从 5 收敛到 2（Tauri command 主路径 + browser HTTP fallback 仅 dev 用） |

---

## 验收

- `find desktop/electron -type f` 返回空
- `find scripts -name "local_api_server*"` 返回空
- `rg "electronAPI|electron:" desktop/` 0 命中
- `rg "spawn.*python" desktop/` 0 命中
- `desktop/gui_three_round_smoke.mjs --backend=rust --real-dialogue --live-provider` 三轮通过
- 端口 8765 仅由 `crates/runtime/src/local_api_server.rs` 绑定

---

## 撤销代价

| 想撤回时 | 代价 |
|---|---|
| Phase 9 删除前 | 零代价（仅 Tauri 用户增加） |
| Phase 9 删除后 30 天内 | 中等：从 git revert 找回；测试套件需要重接 |
| Phase 9 删除后 90 天后 | 高：依赖树（electron@36、wait-on 等）需要重新选版本，Python sidecar 行为可能已与 Rust local_api_server 漂移 |

**结论**：决定后保留 90 天的"反悔窗口"——若 Tauri 在 Linux/Windows 上暴露严重缺陷且无法 12 周内修复，可在 90 天窗口期内 revert；超过则视同永久。

---

## 备注

- `apps/open_claudecode_tui_adapter/cli.mjs` 不在本决策范围（连接 8765 的另一个客户端），其去留在 Phase 9.4 单独决策。
- 本决策不影响 `sidecar_http_transport.rs`（那是 runtime → model provider 的另一个 sidecar，由 D2 决策）。
