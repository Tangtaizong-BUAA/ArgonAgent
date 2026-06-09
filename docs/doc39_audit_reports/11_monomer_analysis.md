# Audit 11: Monomer Analysis (native_agent_loop.rs)

**Date:** 2026-05-19 | **Target:** <400 lines | **Current:** 2,975 lines

## Executive Summary

| Metric | Value |
|---|---|
| Total file | 2,975 lines |
| Main function `run_native_agent_loop_v2_deepseek_inner` | ~2,681 lines |
| Sub-module files | 11 files, ~11,145 lines |
| Deletable (V1 legacy) | ~550 lines |
| Extractable to AgentKernel | ~1,600 lines |
| Extractable to native_profile | ~460 lines |
| **Estimated after full extraction** | **~130-200 lines** |

Reduction from 9,200 to 2,975 already achieved via 11 sub-modules. Core function still needs ~2,300 lines extracted.

## Line-by-Section Breakdown

| Lines | Section | Target | Rows |
|---|---|---|---|
| 1-131 | Imports + sub-module declarations | Keep (update on extract) | 131 |
| 132-289 | Struct definitions | agent_kernel / delete | 158 |
| 291-407 | Main function setup | agent_kernel + native_profile | 117 |
| 408-483 | Main iteration loop framework | agent_kernel | 76 |
| 484-946 | Continuation branch (+HTTP recovery) | agent_kernel | 463 |
| 947-1276 | Initial request branch (+compaction) | agent_kernel + native_profile | 330 |
| 1277-1317 | Post-response processing | agent_kernel + native_profile | 41 |
| 1318-1404 | Streamed tool batch processing | agent_kernel | 87 |
| 1405-1534 | No-tool-call handling | agent_kernel | 130 |
| 1535-1632 | Tool batch guard | agent_kernel | 98 |
| 1633-2779 | **Tool execution loop (LARGEST)** | agent_kernel + native_profile | 1,147 |
| 2780-2971 | Post-batch operations | agent_kernel | 192 |

## V1 Legacy (Deletable)

- `NativeAgentLoopStep`, `NativeAgentLoopRequest`, `NativeAgentLoopResumeRequest`
- `NativeAgentLoopExternalDecisionPackage*`
- `run_native_agent_loop`, `resume_native_agent_loop_after_external_decision`
- **Total: ~550-600 lines deletable**

## DeepSeek-Specific Inline Code

| Pattern | Occurrences | Should be in |
|---|---|---|
| reasoning_replay capture/inject/compact | 6 sites | native_profile::deepseek::reasoning |
| cache breakpoints | 2 sites | native_profile::deepseek::cache_prefix |
| DSML detection | 1 site | native_profile::deepseek::stream |
| shell.command→directory recovery | 3 identical blocks (~330 lines) | native_profile::deepseek |

## Extraction Priority

1. **P0:** Tool execution loop (lines 1633-2779) → AgentKernel (~1,147 lines)
2. **P0:** HTTP fault recovery (lines 711-922, 1168-1231) → AgentKernel (~276 lines)
3. **P1:** DeepSeek reasoning/cache/DSML → native_profile (~70 lines)
4. **P1:** Delete V1 legacy (~550 lines)
5. **P1:** Shell.command→directory recovery dedup (~330→100 lines)
6. **P2:** Full loop framework → AgentKernel::run_turn
7. **P2:** Unify completion patterns (15 call sites → 1 finalize_turn)

## Post-Extraction Target

```rust
fn run_native_agent_loop_v2_deepseek_inner(...) {
    let mut kernel = AgentKernel::for_request(&request);
    kernel.run_turn(transport, session, turn_state, event_log, interrupt)?;
    Ok(loop_result(status, session, counts))
}
// ~25-50 lines core + ~100-150 lines types/imports = ~130-200 total
```
