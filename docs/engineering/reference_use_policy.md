# Reference Use Policy

## Purpose

ResearchCode Coworker uses several existing agent systems as architecture references. This policy keeps implementation work clean, auditable, and legally safer.

## Reference Materials

| Reference | Local path / source | Allowed use | Restricted use |
|---|---|---|---|
| ClaudeCode-like source | `Open-ClaudeCode-main/Open-ClaudeCode-main` | Architecture analysis, behavioral patterns, model-scaffold adaptation ideas, test inspiration | Copying source, prompts, hidden implementation details, or exact tool schemas without license/legal review |
| Anthropic public ClaudeCode docs/examples | `claude-code-main/claude-code-main` | Public UX concepts, settings/workflow analysis, public API behavior | Copying docs/prompts verbatim into product |
| OpenCode | `opencode-dev (1)/opencode-dev` | Runtime/control-plane structure, storage concepts, permission/worktree ideas subject to license | Direct source reuse unless license-compatible and explicitly reviewed |
| DeepSeek-TUI | `DeepSeek-TUI-main/DeepSeek-TUI-main` | DeepSeek-specific optimization ideas, parser/eval concepts subject to license | Direct code reuse without license review |
| ClawCode Rust rewrite | `claw-code-main (1)/claw-code-main` | Rust crate/module boundary inspiration, mock/parity harness ideas subject to license | Copying modules without license review |
| DeepSeek V4 paper | `DeepSeek_V4.pdf` | Model-profile design, eval assumptions, reasoning/tool/context strategy | Treating paper claims as production truth without eval |
| Qwen3.6-27B model card/docs | public Qwen/Hugging Face docs | Native Qwen deployment/profile/parser/context strategy | Assuming serving features exist without deployment capability probe |

## Clean-Room Rules

Architecture Decision:
- Production implementation should cite our own planning docs, not copied reference code.
- When a developer implements a feature inspired by reference systems, the task should point to `docs/agent_architecture_planning/*.md` rather than asking them to copy a reference file.
- Exact source reuse requires separate license review and an explicit note in the task/PR.
- Prompt text, tool schemas, parser logic, and permission rules from non-owned references must be re-designed for ResearchCode Coworker unless license review explicitly allows reuse.

## Allowed Without Additional Approval

- Describing observed behavior in architecture docs.
- Reimplementing general ideas: state machine, event log, tool registry, permission approval, patch manager, worktree manager, context compaction, model profile routing.
- Creating new interfaces/types that satisfy our own requirements.
- Writing original tests based on behavior categories rather than copied fixtures.

## Requires Explicit Review

- Copying any source file, function body, prompt, schema, regex parser, fixture, or UI text from a reference project.
- Using reference project names or trademarks in product UI/marketing.
- Vendoring reference code.
- Adding dependencies whose license conflicts with future commercial distribution.

## PR Checklist Requirement

Every implementation PR should answer:

1. Which architecture doc section guided this change?
2. Did this change copy source, prompt text, schema, or fixture content from a reference repository?
3. If yes, where is the license review note?
4. Does this change introduce a new third-party dependency? If yes, what license?
5. Does this change affect model prompts, tool schemas, permissions, or sandboxing?

## Implementation Guidance

- Use `docs/agent_architecture_planning/16_architecture_gap_review.md` as the current hardening checklist.
- Use reference repos for reading and comparison, then close them before writing implementation code when practical.
- Prefer small original modules with tests over porting large reference modules.
- Keep model-specific adapters original: DeepSeek and Qwen3.6-27B behavior should be validated with our own fixtures and evals.
