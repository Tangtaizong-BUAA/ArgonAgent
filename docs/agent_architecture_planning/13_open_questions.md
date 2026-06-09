# 13 Open Questions

## Product

1. Is the first paying/user segment software engineers, research labs, or mixed AI-heavy teams?
2. Should the product name remain `ResearchCode Coworker`, or should we choose a shorter name before implementation?
3. Is offline/local-model mode a launch requirement or a later differentiator?
4. Should the first GUI optimize for single-user local work or multi-agent task boards?

## Model Access

1. Which DeepSeek V4 endpoint is available for dogfood: official API, local checkpoint, or third-party provider?
2. Which DeepSeek role variants are actually available: Pro, Flash, thinking/non-thinking, long-context?
3. Which Qwen3.6-27B deployment should be supported first: DashScope, vLLM, SGLang, KTransformers, or Transformers serving?
4. What budget/cost constraints should ModelRouter enforce by default?
5. Should cross-family fallback between DeepSeek and Qwen ever be allowed, and if so should it always require per-task approval?
6. Should first internal dogfood default to DeepSeek mode or Qwen3.6-27B mode?

## Safety and Permissions

1. Should local trusted users be allowed to enable auto-apply patches?
2. Should shell network access default to deny or ask?
3. Should plugins/hooks be allowed to execute arbitrary local code in v1?
4. What paths are always sensitive: home dir, SSH keys, cloud credentials, dataset folders?

## Research Workflows

1. Which data formats matter first: CSV/Excel/Parquet/JSON, or domain-specific formats?
2. Should notebook generation be Jupyter-compatible in v1?
3. Do reports need PDF/LaTeX export immediately, or is Markdown enough first?
4. Should literature parsing support OCR in v1?

## Architecture

1. Should runtime API be local HTTP/WebSocket, Tauri IPC, or both from the beginning?
2. Should SQLite live per app, per project, or hybrid?
3. Should worktrees be stored inside project `.git` worktree paths or app data?
4. Should eval harness be a standalone crate/CLI or runtime module?
5. Should artifact storage be content-addressed globally, per project, or hybrid?

## Licensing / Reuse

1. Are these reference repositories only for analysis, or can code be reused where license permits?
2. Do we need a clean-room implementation policy for ClaudeCode-like source?
3. Should plugin manifest format be compatible with Claude/OpenCode conventions or proprietary?

## Team/Cloud Future

1. What data may be synced to cloud: metadata only, artifacts, logs, diffs, model prompts?
2. Is enterprise managed policy a near-term requirement?
3. Should remote approvals work before team task assignment?
4. Should GitHub/CI integration ship before or after research workflows?
