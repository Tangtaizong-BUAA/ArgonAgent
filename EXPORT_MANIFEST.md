# Export Manifest

Prepared: 2026-06-07

This folder is a clean GitHub-ready snapshot of the completed DeepCode /
ResearchCode Coworker implementation slice.

## Included

- Root project configuration: `Cargo.toml`, `Cargo.lock`, `AGENTS.md`.
- GitHub-facing docs and license: `README.md`, `LICENSE`.
- Rust workspace: `crates/kernel`, `crates/runtime`, `crates/cli`,
  `crates/cli-dev-tools`.
- Product desktop surface: `desktop/`, excluding local dependency, build, and
  smoke-run output folders.
- Architecture and implementation docs: `docs/`.
- Deterministic fixtures: `eval/fixtures/`.
- Validation and helper scripts: `scripts/`.
- Research Worker scaffold: `workers/research_worker/`.
- Auxiliary adapter: `apps/open_claudecode_tui_adapter/`.

## Excluded

- Secrets and provider configuration: `.env`, `.researchcode/`.
- Local agent/session/cache directories: `.argon_agent/`, `.claude/`,
  `.codex-pet-runs/`.
- Build or dependency outputs: `target/`, `node_modules/`, `dist/`, `build/`.
- Runtime and GUI smoke artifacts: `runs/`, `artifacts/`,
  `desktop/.gui-smoke-runs/`, `desktop/.tmp_frames/`.
- Downloaded reference repositories and archives.
- Local PDFs, Word files, images, scratch HTML, and other personal documents.

## Recommended GitHub Flow

From this folder:

```bash
git init
git add .
git commit -m "Initial DeepCode implementation snapshot"
```

Then add your GitHub remote and push.
