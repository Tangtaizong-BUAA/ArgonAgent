# 03 DeepSeek-TUI and DeepSeek Docs Analysis

Primary sources:
- `DeepSeek-TUI-main/DeepSeek-TUI-main`
- `DeepSeek_V4.pdf` extracted locally to text for analysis.

## 1. DeepSeek-TUI Overall Architecture

Observed:
- Rust workspace with crates for TUI, CLI, core, config, protocol, MCP, tools, state, exec policy, hooks, and app server.
- `crates/tui/src/core/engine.rs` is the background engine. It receives operations from UI, streams events back, handles cancellation, approvals, user input, tools, compaction, cycle management, subagents, MCP, sandbox/network/LSP, and runtime services.
- `crates/tui/src/models.rs` defines message/content/tool/usage structures.
- `crates/tui/src/client/chat.rs` adapts internal messages to OpenAI-compatible chat completions.
- `crates/tui/src/prompts.rs` composes layered system prompts.

Inferred:
- DeepSeek-TUI is a specialized TUI agent runtime that has accumulated many compensating mechanisms for DeepSeek/OpenAI-compatible model quirks.

## 2. API and Model Configuration

Observed in `config.example.toml`:
- Default provider is DeepSeek with `base_url = "https://api.deepseek.com"`.
- DeepSeek beta URL is mentioned for strict tool mode.
- V4 model IDs include `deepseek-v4-pro` and `deepseek-v4-flash`, plus NVIDIA/Fireworks/SGLang variants.
- `default_text_model = "deepseek-v4-pro"`.
- Reasoning effort supports off/low/medium/high/max; low/medium map to high in compatibility paths, off disables thinking.
- Providers include DeepSeek, NVIDIA NIM, Fireworks, and SGLang.

Recommendation:
- Our DeepSeek profile should model provider variant separately from semantic profile:
  - `deepseek-v4-pro@deepseek`
  - `deepseek-v4-flash@deepseek`
  - `deepseek-v4-pro@sglang-local`
  - `deepseek-v4-flash@fireworks`

## 3. Reasoning / Thinking

Observed in `DeepSeek_V4.pdf`:
- V4 has Non-think, Think High, and Think Max modes.
- Think Max prepends a strong reasoning-effort instruction to the system prompt.
- Tool-calling scenarios preserve reasoning content across rounds and user-message boundaries.
- The paper explicitly warns that frameworks simulating tool interactions via user messages may not trigger tool-calling context paths and may not benefit from enhanced reasoning persistence.

Observed in `client/chat.rs`:
- `MessageRequest` carries `thinking` and `reasoning_effort`.
- `build_chat_messages_with_reasoning()` includes or synthesizes `reasoning_content`.
- `sanitize_thinking_mode_messages()` forces non-empty `reasoning_content` on assistant messages when the model/effort requires it and logs approximate replay tokens.
- `parse_sse_chunk()` parses `reasoning_content` / `reasoning` deltas into internal `Thinking` blocks.

Architecture Decision:
- For DeepSeek thinking mode, never simulate tools as plain user messages. Use real tool-call messages and preserve reasoning content according to provider requirements.

## 4. Tool Schema and Parser

Observed in `DeepSeek_V4.pdf`:
- V4 introduces DSML/XML tool-call schema using `<|DSML|tool_calls>`, `<|DSML|invoke>`, and `<|DSML|parameter>` with explicit string/non-string encoding.
- The paper says XML mitigates escaping failures and reduces tool-call errors.

Observed in DeepSeek-TUI:
- `core/tool_parser.rs` is marked legacy; structured tool-call items are preferred, but it parses `[TOOL_CALL]...[/TOOL_CALL]`, XML-style `<tool_call>`, and `<invoke>` formats.
- `models.rs` supports tool fields such as `allowed_callers`, `defer_loading`, `input_examples`, `strict`, and `cache_control`.
- `client/chat.rs` converts internal tools to Chat Completions tool schema and maps tool choices.

Recommendation:
- Implement a `DeepSeekOutputParser` with three layers:
  1. native provider tool calls;
  2. DSML/XML extraction fallback if provider exposes text-only output;
  3. invalid-tool repair tool result explaining the schema violation.

## 5. Tool-Use Error Recovery

Observed in `tools/arg_repair.rs`:
- Deterministic repair ladder for malformed JSON arguments:
  - strict parse;
  - strip literal control chars inside strings;
  - strip trailing commas;
  - balance braces/brackets;
  - strip excess closers;
  - fallback to empty object;
  - reject oversized >1 MiB.

Observed in `client/chat.rs`:
- Drops tool results without matching tool calls.
- Strips orphaned tool calls after compaction if tool results were summarized away.
- Logs 400-after-sanitizer violations.

Recommendation:
- Use repair only as a compatibility layer and log every repair to eval. Silent repair can mask model failures.

## 6. Context and Prefix Cache Optimization

Observed in `models.rs`:
- `context_window_for_model` maps DeepSeek V4 to 1,000,000 tokens and legacy DeepSeek to 128K.
- `compaction_threshold_for_model` uses 80% of known context window.
- Usage includes `prompt_cache_hit_tokens`, `prompt_cache_miss_tokens`, `reasoning_tokens`, and `reasoning_replay_tokens`.

Observed in `prompts.rs`:
- Prompt layers are arranged most-static to most-volatile to maximize DeepSeek KV prefix-cache hits.
- Stable layers: mode prompt, project context, instructions, user memory, goal, skills, context-management text, compaction template.
- Volatile handoff block is appended after the stable boundary.
- The prompt explicitly warns that reordering/rewriting earlier context hurts cache hits.

Observed in `compaction.rs`:
- Auto-compaction floor is 500K tokens.
- Default threshold for V4 is 800K, not old 50K heuristics.
- Comments explain that compaction rewrites the stable prefix and can destroy cache economics.
- Summary limits are larger for large-context models.

Architecture Decision:
- DeepSeek context policy should be cache-aware:
  - stable system/tool catalog order;
  - stable tool descriptions;
  - append-only recent events;
  - compact only when budget pressure justifies cache loss;
  - log cache hit/miss per call.

## 7. Tool Catalog Stability

Observed in `tools/registry.rs`:
- `to_api_tools()` sorts tools by name for prefix-cache stability.
- Serialized tool catalog is memoized after first build and invalidated only on register/remove/clear.
- Schema is sanitized before sending to API.

Recommendation:
- Tool registry should produce deterministic byte-stable schemas for all providers, but this is especially important for DeepSeek.

## 8. Large Output and Eval

Observed:
- `tools/registry.rs` routes large outputs through a workshop/large-output router and can store raw results.
- `eval.rs` defines an offline harness for list/read/search/edit/apply_patch/exec_shell scenarios, with metrics for success, tool errors, duration, per-tool stats, and record/replay fixtures.

Recommendation:
- Reuse the idea: local offline eval first, then model-in-the-loop eval. DeepSeek-specific improvements must show lower tool-call error, lower replay waste, higher pass rate, or lower cost.

## 9. Real DeepSeek-Specific Optimizations

True DeepSeek-specific optimizations:
- V4 1M context window and thresholds in `models.rs` / `compaction.rs`.
- Thinking-mode `reasoning_content` replay and sanitizer in `client/chat.rs`.
- Reasoning replay token telemetry.
- Prefix-cache aware prompt ordering in `prompts.rs`.
- Stable sorted/memoized tool catalog in `tools/registry.rs`.
- High auto-compaction floor to avoid unnecessary prefix-cache destruction.
- DSML/XML fallback parser retained for DeepSeek-style text tool calls.
- JSON argument repair for DeepSeek/OpenAI-compatible streaming argument failure modes.
- Cache hit/miss usage accounting.
- Strict tool mode tied to DeepSeek beta endpoint in config.

Generic agent optimizations:
- Tool registry.
- Shell/file/search tools.
- Approval policy.
- Sandbox/network policy.
- MCP.
- Session resume/handoff.
- Skills.
- LSP diagnostics.
- Subagents.
- Eval harness.

Mostly "just API switch":
- Provider aliases and base URL fields by themselves.
- Naming a model `deepseek-v4-pro` without changing context/tool/reasoning policy.

Possibly ineffective or needs eval:
- Forcing tool use every turn with strict tool mode can degrade final-answer turns.
- Reasoning replay can be expensive; it must be constrained by role/task.
- Text/XML fallback parsing can hide failures if native tool-calling is available.
- Very large context can reduce retrieval precision beyond 128K, per V4 paper's MRCR note.

## 10. DeepSeek Model Fit

Observed from `DeepSeek_V4.pdf`:
- V4-Pro-Max is strong on reasoning, coding competitions, formal math, and long-context tasks.
- V4-Pro is strong but open models still lag closed models on some code-agent tasks.
- V4-Flash-Max is cost-efficient but underperforms Pro on coding agent tasks, especially Terminal Bench.
- Agent evals use minimal bash/file-edit tools, up to 500 steps, 512K max context.
- Search agent tasks use websearch/Python tools; agentic search beats standard RAG in complex tasks.

Recommendation:
- Planner: V4-Pro High/Max for long-context planning and research synthesis.
- Executor: V4-Flash for read/search/summarize; V4-Pro for code edits when cost allows; do not use Flash as sole executor for high-risk code patches.
- Reviewer: Use V4-Pro-Max as the DeepSeek-native reviewer, and require build/test verification. ClaudeCode/Codex are comparison references for reviewer design, not native fallback targets in the first product line.
- Research/data analyst: V4-Pro for analysis design/reporting; Python/DuckDB for actual computation; Flash for schema summarization.

## 11. DeepSeek V4 ModelProfile Draft

Recommendation:
- Strengths: long context, reasoning, coding competition, formal math, planning from retrieved context, agentic search, cost-efficient Flash.
- Weaknesses: tool-call protocol sensitivity, reasoning replay cost, potential over-thinking, reduced retrieval precision beyond 128K, Flash weaker on coding agent tasks.
- Roles:
  - Pro-Max: planner, diagnosis, research synthesis, hard reviewer.
  - Pro-High: coding executor and data-analysis script generator.
  - Flash-High: explorer, summarizer, low-risk executor.
  - Non-think: fast command explanations, small edits, simple summaries.
- Forbidden:
  - Flash as final reviewer for destructive/high-risk patches.
  - Think Max for every small edit.
  - Text-simulated tools in thinking mode.

## 12. Eval Plan for DeepSeek Optimization

Metrics:
- Tool-call parse success.
- Argument repair rate.
- Orphaned tool-call stripping rate.
- Cache-hit percentage and cache-hit cost savings.
- Reasoning replay tokens per successful task.
- Patch apply success.
- Build/test pass.
- Human approval count.
- Terminal Bench-style task pass.
- Data-analysis script correctness.

Architecture Decision:
- Every DeepSeek-specific switch must be controlled by a profile flag and included in A/B evals.
