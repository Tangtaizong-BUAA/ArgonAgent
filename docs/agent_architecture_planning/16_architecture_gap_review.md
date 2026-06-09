# 16 Architecture Gap Review

## Purpose

This document records the second-pass completeness review after the native model scope was clarified:
- first-party native code-agent modes: DeepSeek and Qwen3.6-27B;
- ClaudeCode is a first-class scaffold/model-adaptation reference;
- Claude/OpenAI/Codex/GLM/generic local models are reference/eval/future-adapter material, not silent native fallbacks.

## Review Verdict

Architecture Decision:
- The architecture is directionally complete, but implementation should not start until the Phase 0 hardening items below are done.
- The biggest risks are not conceptual gaps. They are boundary drift: GUI bypassing runtime, model adapters leaking provider details into runtime, missing model-call telemetry, weak research sandbox reproducibility, and unclear reference-source reuse policy.

## Fixed In This Review

| Gap | Resolution | Files |
|---|---|---|
| GUI could bypass runtime and write SQLite directly | Removed GUI -> Store path; stated Runtime API is the only write path | `08_target_product_architecture.md` |
| Core types lacked lifecycle/creator/consumer/persistence detail | Added per-type lifecycle matrix | `08_target_product_architecture.md` |
| `model_calls` schema could not support DeepSeek/Qwen native telemetry | Added mode, adapter, deployment, parser, thinking, prompt/tool hash, context artifact fields | `08_target_product_architecture.md` |
| Qwen3.6 profile lacked sampling/deployment structure | Added deployment and generation profiles to `ModelProfile` | `08_target_product_architecture.md`, `10_model_optimization_architecture.md` |
| Qwen3.6 optimization reduced to strict schema in roadmap | Expanded Phase 6 to parser, preserve-thinking, context, sampling, eval, native logging | `11_full_roadmap.md` |
| Parser fixture test command was ambiguous | Split DeepSeek and Qwen test commands | `12_first_30_codex_tasks.md` |
| Research Worker sandbox was too high-level | Added resource limits, output path limits, environment snapshot, lineage, PII masking, run manifest | `09_research_coworker_architecture.md` |
| Clean-room/license policy was only an open question | Added Phase 0 deliverable and first task requirement | `11_full_roadmap.md`, `12_first_30_codex_tasks.md` |
| Clean-room/reference-use policy file did not exist | Created implementation policy with PR checklist | `docs/engineering/reference_use_policy.md` |
| Cross-family fallback wording could be misread | Changed repair stack to same-family fallback profile | `10_model_optimization_architecture.md` |
| Qwen mode naming was inconsistent | Standardized on `Qwen36_27BOptimizedMode` | `15_native_deepseek_qwen_modes.md` |

## Must-Fix Before Phase 0 Implementation

1. Maintain `docs/engineering/reference_use_policy.md`.
   - Status: initial policy created.
   - Remaining work: fill exact licenses from each local reference repository before implementation PRs start.
   - Must define what is allowed: architectural ideas, public API shapes, test ideas.
   - Must define what is blocked without explicit review: copying source, copying prompts verbatim, copying proprietary implementation details.
   - Must require production code comments to cite our architecture docs, not reverse-engineered code.

2. Decide runtime API shape.
   - Open question: Tauri IPC only, local HTTP/WebSocket only, or both.
   - Recommendation: define an internal Rust service interface first; expose Tauri IPC and local WebSocket as adapters.

3. Decide SQLite placement.
   - Open question: app-level DB, per-project DB, or hybrid.
   - Recommendation: hybrid: app DB for projects/settings/global profiles; per-project `.researchcode/index.sqlite` for sessions/artifacts/memories where portability matters.

4. Define artifact store layout.
   - Required paths: model requests/responses, context bundles, tool outputs, diffs, notebooks, charts, reports, research data profiles.
   - Required metadata: content hash, MIME type, sensitivity, source module, retention policy.

5. Define native deployment capability probe.
   - DeepSeek: endpoint, role variant, reasoning/replay support, long-context support, native tool schema support.
   - Qwen3.6-27B: deployment stack, context length, `qwen3` reasoning parser, `qwen3_coder` tool parser, preserve-thinking support, max output budget.

## Must-Fix Before Phase 1 Runtime Loop

1. Add durable event versioning.
   - `AgentEvent.type` must be an enum plus versioned payload schemas.
   - GUI renderers must tolerate unknown future events.

2. Add idempotency keys.
   - Required for command approval, patch application, tool retry, model retry, and event append.

3. Add cancellation semantics.
   - Define cancellation for model streams, shell commands, Python jobs, patch application, and worktree cleanup.

4. Define tool output budgets.
   - Every tool needs preview limits, artifact spill behavior, and model-context summarization rules.

5. Define read-before-write invariant.
   - Patch manager must reject edits unless the edit target has a known old hash or explicit user override.

## Must-Fix Before Phase 5/6 Native Model Work

1. DeepSeek adapter fixtures:
   - native tool call;
   - DSML fallback;
   - malformed JSON repair;
   - reasoning replay/sanitizer;
   - prefix-cache stable tool schema.

2. Qwen3.6 adapter fixtures:
   - thinking output with final answer;
   - non-thinking output;
   - preserve-thinking continuation;
   - `qwen3_coder` tool-call parser output;
   - Qwen template/tag fallback parse;
   - generic OpenAI tool-call rejection when parser capability is false.

3. Sampling profile tests:
   - thinking/general;
   - thinking/precise coding;
   - non-thinking/instruct.

4. Same-family fallback tests:
   - DeepSeek Pro -> Flash only when role/risk allows;
   - Qwen thinking -> Qwen coding/non-thinking only when role/risk allows;
   - cross-family fallback blocked unless explicit permission exists.

5. Telemetry completeness test:
   - every model call must have `mode_id`, `profile_id`, `adapter_version`, `prompt_template_hash`, `tool_schema_hash`, `deployment_stack`, parser flags, context length, and thinking settings.

## Must-Fix Before Research Worker v1

1. Python sandbox runner must enforce:
   - cwd allowlist;
   - output path allowlist;
   - CPU/wall-time/memory/output limits;
   - network deny by default;
   - permission request for package install.

2. Research lineage must record:
   - input file hashes;
   - transformation script hash;
   - environment snapshot;
   - output artifact hashes;
   - generated report references.

3. Data privacy policy must include:
   - PII/sensitive-column detection;
   - sample masking before model context;
   - explicit approval for cloud model use on sensitive datasets;
   - audit log for every dataset read.

## Remaining Open Questions

Product:
- Should first dogfood default to DeepSeek mode or Qwen3.6-27B mode?
- Should Qwen3.6-27B be assumed local-first, cloud-first through DashScope, or deployment-flexible?

Runtime:
- Should plans require user approval by default, or only for high-risk tasks?
- Should file patch approval be mandatory in local trusted mode?

Model:
- Which DeepSeek V4 role variants are actually accessible in development?
- Which Qwen3.6-27B serving stack is first-class for v1: DashScope, vLLM, SGLang, KTransformers, or Transformers?

Research:
- Is Markdown-only report output acceptable for v1?
- Does literature parsing need OCR in the first research release?

Team/cloud:
- Should remote approval exist before team task assignment?
- What exact data may be synced: metadata only, artifacts, diffs, prompts, or eval summaries?

## Architecture Decision Summary

- Keep the product local-first and runtime-authoritative.
- Keep GUI as command center over Runtime API, not a storage client.
- Keep coding and research as peer workflows over the same session/event/tool/permission substrate.
- Keep DeepSeek and Qwen3.6-27B as native model families with dedicated adapters.
- Keep ClaudeCode as the strongest scaffold adaptation reference and translate its model-shaped runtime mechanics into our two native modes.
- Keep eval and telemetry mandatory, because model-specific optimization without measurement becomes folklore.
