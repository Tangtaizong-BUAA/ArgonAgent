# ArgonAgent

一个本地优先的 AI Agent 工作台。

简单说：我想做一个更懂 DeepSeek 和 Qwen 的本地 AI 工作伙伴。它不只是把模型 API
接进来聊聊天，而是从运行时、上下文、工具调用、权限、桌面交互到评测体系，一层一层把
DeepSeek/Qwen 当成一等公民来优化。

项目主页和后续更新可以看这里：

[argonai.cn](https://argonai.cn)

## 这东西想解决什么

现在很多 Agent 工具都能“接模型、跑命令、改文件”，但不同模型的思考方式、上下文习惯、
工具调用格式和流式输出细节其实差很多。

ArgonAgent 的方向是：

- 像 Claude Code 一样能在本地仓库里认真干活。
- 像 Codex GUI 一样有一个清楚的桌面命令中心。
- 像研究助理一样能处理数据、实验、报告和文献工作流。
- 更重要的是：对 DeepSeek 和 Qwen 做原生优化，而不是把它们塞进一个通用 adapter 里凑合用。

## 我们怎么优化 DeepSeek / Qwen

这版已经完成了一批底层能力，重点不是“多接几个 provider”，而是让 Agent loop 更适合
DeepSeek/Qwen 长任务工作。

- **原生运行时链路**

  核心链路是：

  ```text
  RuntimeFacade -> AgentKernel -> NativeProfile(DeepSeek/Qwen)
  ```

  DeepSeek/Qwen 的 prompt、上下文预算、工具策略、流式解析和 eval gate 都在 native profile
  里管理，不被普通 compatible provider 稀释。

- **上下文和长会话**

  运行时有上下文预算、压缩前置检查、reasoning replay、事件日志和恢复路径。目标是让长任务
  不因为上下文膨胀、工具循环或中途审批而变成一团乱麻。

- **工具调用更像“契约”，不是字符串猜谜**

  工具调用会经过 Tool Contract Mediation、权限门、补丁校验、密钥扫描、artifact 捕获和事件记录。
  重点是让模型做事可追踪、可恢复、可回放，而不是把一大段输出扔给脚本碰运气。

- **DeepSeek/Qwen 的输出差异单独处理**

  DeepSeek 的 reasoning、cache prefix、DSML/工具 fallback，Qwen 的工具 schema、stream processor
  和上下文 guardrail 都有单独路径。compatible provider 可以用，但不能冒充 native 优化模型。

- **桌面端不是装饰**

  `desktop/` 是真实产品入口：React + Tauri，接 Rust runtime，能看事件流、审批状态、工具执行、
  transcript 和运行状态。不是一个只会展示 mock 数据的壳。

## 当前有什么

```text
crates/
  kernel/          产品基础类型：model、task、plan、tool、transcript。
  runtime/         Agent 运行时、DeepSeek/Qwen native profile、事件、工具、恢复逻辑。
  cli/             本地 CLI 入口。
  cli-dev-tools/   开发和 smoke 测试工具。

desktop/           React + Tauri 桌面命令中心。
docs/              架构、实现状态、决策记录、审计文档。
eval/fixtures/     parser、stream、scaffold、permission、research 等确定性夹具。
scripts/           验证脚本、eval runner、provider sidecar。
workers/           Research Worker 脚手架。
apps/              小型辅助 adapter。
```

## 快速跑起来

先跑本地检查：

```bash
python3 scripts/check_all.py
```

只跑 Rust 测试：

```bash
cargo test --workspace -- --test-threads=1
```

跑一个确定性的 Agent loop smoke：

```bash
cargo run -q -p researchcode-cli -- native-agent-loop-smoke
```

启动桌面端：

```bash
npm --prefix desktop install
npm --prefix desktop run tauri:dev
```

浏览器调试模式：

```bash
npm --prefix desktop run dev
```

默认会在 Vite 给出的本地地址打开，通常是 `http://127.0.0.1:5173`。

## 环境变量

大部分本地检查不需要真实 provider key。live provider 路径默认是 gated，需要你自己显式打开。

```bash
export DEEPSEEK_API_KEY=...
export QWEN_API_KEY=...
export QWEN_BASE_URL=...
```

不要提交 `.env`、API key、provider 原始响应、本地 runtime artifact 或私人文件。

## 常用检查

```bash
python3 scripts/run_parser_eval.py
python3 scripts/run_stream_eval.py
python3 scripts/run_native_profile_promotion_gate.py
node desktop/test_runtime_event_replay.mjs
npm --prefix desktop run build
```

## 当前阶段

这不是生产版，也不是“我们已经复刻了 Claude Code”的大旗。

更准确地说：这是一个 DeepSeek/Qwen-first 本地 Agent 工作台的已完成实现快照。现在已经有
runtime/kernel、native profile、桌面端、事件系统、权限恢复、工具契约、评测夹具和验证脚本。

接下来还会继续打磨：

- 长会话稳定性
- live provider gated smoke
- 桌面端 stream normalization
- transcript 性能
- subagent 事件隔离与合并
- Research Coworker 工作流

如果你想看看这个项目背后的更多东西，或者后续我会把它做成什么样，欢迎去：

[argonai.cn](https://argonai.cn)

## License

MIT License。详见 [LICENSE](LICENSE)。
