# Audit 14: NativeProfile + RoleSplit Integration

**Date:** 2026-05-19 | **Scope:** Profile factory wiring, model selection, temperature scheduling

## 1. Profile Instantiation — Partially Wired

`profile_for_family()` called in 3 production locations:
1. `kernel.rs:229` — AgentKernel::for_request() uses `request.endpoint.family`
2. `kernel.rs:220` — AgentKernel::default() hardcodes DeepSeek
3. `prompt_assembler.rs:45` — branches on family for system prompt

**But:** Only `.family()` and `.profile_name()` are used. No behavioral dispatch through profile.

## 2. Model Selection — Single Model Only

**No role-based model switching.** All requests use the same `endpoint.actual_model_name` from `NativeProviderEndpoint`.

- `RoleSplit::deepseek_default()` maps 5 roles to models but is **never called in production**
- `PlannedModelCall.role_model_name` always `None`
- Main loop hardcodes `ModelRole::Executor` everywhere
- Compaction uses same endpoint as main loop (no Flash)

## 3. Temperature — Always None/Default

- All 3 adapters set `temperature_milli: None`
- `TemperatureSchedule` struct **does not exist** in source code
- `RoleSplit.temperatures` map (Routing=0.0, Executing=0.2, NarrativeAnswer=0.7) defined but never read
- HTTP requests carry no temperature parameter → provider defaults (~1.0)

## 4. Compaction Model — Same as Main Loop

When compaction triggers:
1. `build_native_compacted_initial_request(&request.endpoint, ...)` — reuses same endpoint
2. `build_native_tool_evidence_continuation_request(&request.endpoint, ...)` — reuses same endpoint
3. No separate Flash model selection exists

## 5. Titling/Summarization — Not Role-Split

`AgentRoleKey::Titler` and `Summarizer` defined but never used. No production code creates requests with these roles. Visible finalizer clones the main endpoint.

## 6. Hardcoded DeepSeek Assumptions

| # | Location | Assumption |
|---|---|---|
| 1 | kernel.rs:220 | Default() hardcodes DeepSeek family |
| 2 | native_agent_loop.rs:51 | StreamProcessor from deepseek module |
| 3 | native_agent_loop.rs:50 | ReasoningReplayManager from deepseek module |
| 4 | native_agent_loop.rs:1277-1289 | Reasoning capture only for DeepSeek family |
| 5 | prompt_assembler.rs:112 | deepseek_cache_zones() only for DeepSeek |
| 6 | All adapters | temperature_milli always None |
| 7 | All adapters | role_model_name always None |

## Phase 5 Completion: ~40-45%

Core infrastructure (trait, factory, enum) exists and is called. But major features (RoleSplit, TemperatureSchedule, per-role model selection) are entirely disconnected from production.
