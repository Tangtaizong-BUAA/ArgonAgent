# Audit 08: DeepSeek Cache + Role + Thinking vs doc39

**Date:** 2026-05-19 | **Files:** cache_prefix.rs, role_split.rs, thinking.rs, budget.rs

## 1. CachePrefixPolicy — Implemented & Connected

**Status:** ACTIVE. Two construction paths:
1. `prompt_assembler.rs:112` — calls `deepseek_cache_zones()` + `deepseek_system_prompt()`
2. `native_agent_loop_prompt.rs:148` — calls `CachePrefixPolicy::build_zones()` + `deepseek_system_prompt()`

**Zone A (immutable):** Tools sorted by `canonical_tool_id` lexical — cache-stable. PASS.
**Zone B (session):** Metadata sorted by key. PASS.
**Zone C (turn):** Contains tool catalog. **Gap:** ContextBundle items go into a separate `<context>` block, NOT into Zone C. Cache misses on context content.

**`deepseek_system_prompt()` duplication:** NOT duplicated. `prompt_assembler.rs` delegates to `cache_prefix` version.

## 2. RoleSplit — Orphaned

`role_split.rs` defines:
- 5 `AgentRoleKey` variants: Executor→deepseek-chat, Compactor→deepseek-chat-flash, Reviewer→deepseek-chat, Titler→deepseek-chat-flash, Summarizer→deepseek-chat-flash
- 6 `RoleStage` variants with temperatures: Routing=0.0, Executing=0.2, NarrativeAnswer=0.7, etc.

**Zero production imports.** `RoleSplit`, `AgentRoleKey`, `RoleStage` — never imported outside their own file. Only used in unit tests.

## 3. TemperatureSchedule — Not Implemented

- `PlannedModelCall.temperature_milli` always `None` (all 3 adapters)
- HTTP request body never contains `temperature` field
- Provider defaults used (~1.0 for DeepSeek)
- `cache_prefix.rs:54` reads `plan.temperature_milli.unwrap_or_default()` → always "0"

## 4. ThinkingChain — Orphaned (423 lines)

Full state machine: `Idle → Streaming → Completed`, `ThinkingChainEvent` enum, TUI rendering, sanitization. **Never instantiated in production.** Main loop uses `reasoning_replay.capture_raw_response()` directly, bypassing ThinkingChain entirely.

## 5. Budget — Duplicated

`deepseek/budget.rs` (45 lines) duplicates functions from `context_budget.rs` (405 lines):
- `deepseek_full_budget()` — identical in both files
- `deepseek_scale()` — identical in both files

`context_budget.rs` version is used in production. `deepseek/budget.rs` version is dead code.

## 6. B3/B4 Compliance

| Requirement | Status |
|---|---|
| B3: Pro/Flash gradient (Executor→Pro, Compactor→Flash) | FAIL — RoleSplit defined but not wired |
| B4: Temperature sensitivity (tool calling ≤0.3, narrative 0.7) | FAIL — temperature always None |

## Recommendations
1. Delete `deepseek/budget.rs` duplicates or make them re-exports
2. Wire `ThinkingChain` into main loop reasoning delta processing
3. Wire `RoleSplit` + `TemperatureSchedule` into model call planning
4. Fix Zone C to include context bundle items for cache stability
