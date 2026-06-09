# Audit 09: Qwen Profile + Factory vs doc39

**Date:** 2026-05-19 | **Files:** native_profile/mod.rs, native_profile/qwen/, native_profile/deepseek/mod.rs

## 1. NativeProfile Trait — Minimal Skeleton

```rust
pub trait NativeProfile {
    fn family(&self) -> NativeModelFamily;
    fn profile_name(&self) -> &'static str;
    fn supports_reasoning_replay(&self) -> bool { false }
}
```

**Gap:** Only 3 methods. doc39 §2 requires 6 components in NativeProfile layer:
PromptScaffold, CachePrefixPolicy, ToolSchemaPolicy, RoleSplit+TemperatureSchedule, ReasoningReplayManager, StreamProcessor. None of these are in the trait.

## 2. QwenProfile — Empty Shell (19 lines)

Only implements `family()` and `profile_name()`. Missing vs DeepSeek:

| Submodule | DeepSeek | Qwen |
|---|---|---|
| Stream processing | 2 files (stream.rs, stream_processor.rs) | **NONE** |
| Reasoning replay | reasoning.rs | **NONE** |
| Cache prefix | cache_prefix.rs | **NONE** |
| Role split | role_split.rs | **NONE** |
| Thinking chain | thinking.rs | **NONE** |
| Budget | budget.rs (duplicated) | budget.rs |

Qwen has only budget. Missing 5 out of 6 submodules.

## 3. profile_for_family() — Production Call Sites Exist

Now called in 3 production locations (was 0 in old gap analysis):
1. `kernel.rs:229` — `AgentKernel::for_request()` uses `request.endpoint.family`
2. `kernel.rs:220` — `AgentKernel::default()` hardcodes DeepSeek
3. `prompt_assembler.rs:45` — `assemble_native_prompt()` branches on family

**But** the profile is only used for family/name validation. Actual model-specific behavior bypasses the profile entirely.

## 4. Main Loop Provider Coupling

`native_agent_loop.rs` imports directly from DeepSeek modules:
```rust
use crate::native_profile::deepseek::reasoning::ReasoningReplayManager;
use crate::native_profile::deepseek::stream_processor::StreamProcessor;
```

These are hardcoded DeepSeek paths. Qwen requests use the same StreamProcessor. No Qwen-specific stream processor exists.

## 5. Key Gaps

1. NativeProfile trait doesn't encapsulate model-specific behavior — all real logic bypasses it
2. QwenProfile is 19-line empty shell — Phase 5 "QwenProfile落地" not started
3. Main loop hardcodes DeepSeek imports regardless of provider family
4. Qwen has no chat-template detection, no stream processing, no reasoning replay
5. Factory creates profiles but callers only use `.family()` and `.profile_name()`

## Phase 5 Completion: ~40-45%
