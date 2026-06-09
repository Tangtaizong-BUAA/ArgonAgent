# Red Team: Architecture & Design Drift Findings

## Scope

This review cross-validates the Phase 1 audit reports (Agents 1, 3, 6, 8, 9, 15), the doc39 Conflict Matrix, the Issue Matrix, and the source implementation against the doc39 architecture specification and the OpenClaudeCode reference design. Focus areas: misclassified compliance, missed violations, false conflicts, and reference design drift.

---

## Finding 1: AgentKernel Is a Hollow Facade -- Classified COMPLIANT but Architecturally VOID

**Severity: P0 (architectural deception)**
**File:** `crates/runtime/src/agent_kernel/kernel.rs:203-237`
**Conflict Matrix Status: §1.1 marked COMPLIANT**

### What We Missed

The conflict matrix declares §1.1 (AgentKernel as authoritative service facade) as **compliant** with evidence "AgentKernel struct in agent_kernel/kernel.rs with clean service graph." This is architecturally false.

`AgentKernel` at kernel.rs:128-142 defines a struct with 10 service fields:
- `turn_controller` (NativeLoopTurnController)
- `compactor` (Compactor)
- `permission_gate` (PermissionGate)
- `context_manager` (ContextManager)
- `evidence_ledger` (Arc<Mutex<EvidenceLedger>>)
- `event_log_handle` (Arc<RwLock<EventLog>>)
- `convergence` (ConvergenceEnforcer)
- `tcml` (TcmlService)
- `completion` (CompletionAuthority)
- `tool_orchestration` (ToolOrchestrationService)

Yet `run_turn()` at line 203 does **nothing with any of these services except validate the profile**. The entire method body (lines 203-237) is:
1. Create an `AtomicBool` interrupt flag
2. Call `run_native_agent_loop_v2_deepseek_with_interrupt()` — the old monolithic loop function

The `AgentKernel` service fields exist but are only consumed by `AgentKernel::for_request()` at line 177 to materialize *a second copy* that the monolithic loop constructs independently at line 417 via `AgentKernel::for_request(&request)`. The kernel's own services are dead storage.

**Crucially:** When the main loop at line 417 creates `kernel_services = AgentKernel::for_request(&request)`, it constructs a NEW kernel with its OWN service graph. The original AgentKernel that called `run_turn()` is never used. The loop then interacts with `kernel_services.turn_controller`, `kernel_services.evidence_ledger`, `kernel_services.context_manager`, etc. The AgentKernel struct is just a bag-of-services constructor, not an orchestrator.

### Why It Matters

This is the central architectural deception of the codebase. The conflict matrix shows 32% compliant, 21% partial, 33% conflict -- but the "kernel" that forms the foundation of §1 compliance does not orchestrate anything. The entire service graph is dead code when viewed from the kernel boundary. This invalidates the compliance status of §1.1, §1.2, §1.4, §1.5, §1.6, and §1.8 -- all of which depend on the kernel being the actual execution authority rather than a pass-through.

### doc39 Gap Analysis Confirms

The gap analysis at `doc39_implementation_gap_analysis.md:36-40` already noted this: "`run_native_turn()` 不是编排器，只是一行转发到 `run_native_agent_loop_v2_deepseek()`". The Phase 1 audit reports failed to escalate this from gap analysis to conflict matrix severity.

---

## Finding 2: `visible_text_looks_like_transition_statement` IS the Rejected `visible_finalizer_answer` Pattern

**Severity: P1 (pattern reincarnation under different name)**
**File:** `crates/runtime/src/native_agent_loop.rs:1529-1558`, `crates/runtime/src/native_agent_loop_util.rs:987-1076`
**Conflict Matrix Status: §15 visible_finalizer_answer marked REMOVED (compliant)**

### What We Missed

Agent 15 (doc39 Drift) and the conflict matrix both declare `visible_finalizer_answer` as **removed/compliant** because the exact string `visible_finalizer_answer` does not appear in production code. This is a keyword-search compliance check that missed the pattern reincarnation.

The `visible_text_looks_like_transition_statement()` function at native_agent_loop_util.rs:987-1076 checks model output against `final_answer_signals` -- an array of 15 string patterns including:
- "I'll now provide"
- "Let me summarize"
- "Here is my answer"
- "Here's what I found"
- "Based on my analysis"
- and 10 others

These are used at native_agent_loop.rs:1545 to decide whether the loop should **continue** (transition statement detected) or **complete** (visible answer detected). If the model produces text matching any of these patterns after prior tool work, the loop is forced to continue instead of completing.

The mechanism is identical to what doc39 §15 rejects:
1. **doc39 rejected pattern:** Runtime uses string matching to decide the model is "done"  
2. **Current pattern:** Runtime uses string matching to decide the model is "NOT done" (negated logic, same mechanism)
3. **Both:** Runtime substitutes its own judgment for model signal

The distinction between "positive filter" and "negative filter" is cosmetic. The architecture violation is the same: the runtime makes loop-control decisions based on fragile string heuristics rather than model-emitted signals (tool calls, stop reasons, explicit completion markers).

### Anti-Pattern: String Heuristic as Loop Control Signal

The current code at native_agent_loop.rs:1529-1558:
```rust
if tool_calls.is_empty() {
    // ...
    let prior_tool_work = tool_call_count > 0 || !last_tool_batch.is_empty();
    if prior_tool_work && visible_text_looks_like_transition_statement(&visible_text) {
        // Treat as transitional narration, continue loop
        continue;
    }
    // Otherwise treat as visible answer, complete
    return Ok(loop_result(NativeAgentLoopStatus::Completed, ...));
}
```

This is a loop-control decision based on string matching. doc39 explicitly rejects this architecture (§15: no string-based completion heuristics). The `final_answer_signals` array is semantically identical to the rejected patterns except for the negation operator.

---

## Finding 3: Evidence Clearing Loop Defeats Cross-Iteration Continuity

**Severity: P1 (evidence fragmentation)**
**File:** `crates/runtime/src/native_agent_loop.rs:1705-1706`
**Conflict Matrix Status: NOT FLAGGED at this severity level**

### What We Missed

The main loop clears both `last_tool_batch` and `evidence_ledger` at lines 1705-1706 before re-executing tools:

```rust
last_tool_batch.clear();     // line 1705
evidence_ledger.clear();     // line 1706
```

The comment at lines 1702-1704 frames this as intentional:
```
// Keep provider continuations scoped to the immediately preceding
// assistant tool-use batch. Accumulating older tool results makes each
// continuation replay stale evidence and encourages browse/read loops.
```

This is architecturally self-defeating:

1. **EvidenceLedger.clear() wipes current iteration but preserves sealed iterations.** When line 1706 calls `clear()`, it zeroes `current_items` and all current counters (new_evidence_count, error_count, etc.). The `iterations` vector of sealed `IterationEvidence` snapshots is preserved. At line 823, `begin_iteration()` seals the post-clear batch into `iterations`. 

2. **The streaming evidence built between lines 828-1400 is LOST.** The streaming tool handler at line 828 populates evidence_ledger during HTTP streaming. Then at line 1705-1706, this entire batch is cleared. The tools are re-executed in the post-clear loop (lines 1708-2778), but the streaming evidence's classification (NewEvidence vs Recovery vs Error) is destroyed before convergence can act on it.

3. **ContinuationView only sees the immediately preceding batch.** At line 628, `continuation_view_for_batch(&evidence_ledger, &last_tool_batch)` constructs the model's view of what happened. After clear(), only the post-clear batch is visible. The model cannot reference evidence from iteration N-2 or earlier because `last_tool_batch` only holds the most recent iteration.

4. **This explains the observed 70-iteration plateau delays (P0-09).** If convergence tracking only has one iteration of evidence at a time, plateau detection requires building up streaks across sealed `iterations` entries. The convergence check DOES work across sealed iterations (see Finding 1 correction below), but the evidence *quality* degrades because only the most recent batch enters the sealed record.

### Correction to Agent 9's Analysis

Agent 9 (Long Task Progression) at Hidden Risk #5 describes this as "early-iteration evidence is only in the `history_digest`." This is correct but under-weighted. The architecture is DESIGNED to lose evidence — not as a bug but as an intentional "scoping" decision. The design comment at line 1702 explicitly states this prevents "browse/read loops," but the side effect is that the model cannot build on prior evidence and must re-derive conclusions.

---

## Finding 4: EscalateToCodeEdit Is More Destructive Than Audits Claim

**Severity: P1 (escalated from P1 — the mechanism is worse than reported)**
**File:** `crates/runtime/src/native_agent_loop.rs:2814-2837`, `crates/runtime/src/agent_kernel/turn_controller.rs:1084-1148`
**Conflict Matrix Status: §2.1 marked CONFLICT (correctly flagged)**

### Challenge to Existing Finding

The conflict matrix and Agent 1 both flag this as §2.1 violation (manifest change mid-loop). This is correct but **undersells the structural damage**.

When escalation fires at line 2814:
1. `effective_tool_exposure` mutates from ReadOnly/FastAutoWrite to CodeEdit (line 2815)
2. A NEW manifest is built with `TurnRoute::CodeEdit` (line 2816-2822)
3. `manifest_allowed_tools` is replaced wholesale (line 2823-2828)
4. `tools_json` (the tool schema sent to the model) is replaced (line 2829)

**What the audits miss:**

1. **The model is NOT informed of escalation.** There is no system message, no user guidance, no context injection telling the model "you now have write tools available." The model continues in the same conversation context, with new tools silently appearing in the next request's tool manifest. The model has no signal that the escalation occurred.

2. **The TurnRoute changes silently.** After escalation, `TurnRoute::CodeEdit` is used for manifest building, but `turn_route` (the original route at line 446) is NOT updated. The rest of the loop still uses the ORIGINAL route for prompt building and continuation hints. This creates a split state:
   - Manifest: built for CodeEdit
   - Prompt/continuation: built for original route (e.g., ReadOnlyExplore)
   - The model sees write tools in the manifest but is guided by read-only continuation hints

3. **There is no de-escalation path.** Once escalated to CodeEdit, the exposure stays at CodeEdit for all remaining iterations. If the model was actually making progress but just needed one more read pass, it now has write tools permanently added to a context that was built for read-only exploration.

4. **Convergence checks become confused.** After escalation, `can_escalate_to_code_edit` at line 2806-2807:
   ```rust
   can_escalate_to_code_edit: effective_tool_exposure != NativeAgentToolExposure::CodeEdit
   ```
   This becomes `false` permanently after first escalation. The next DuplicateDominance/InformationStagnation verdict (turn_controller.rs:1084) routes to SoftWarning instead of Escalate, because `can_escalate_to_code_edit` is false and `self.escalation_attempts >= self.max_escalation_attempts` (2). This means after 2 escalations (which both happen within one or two iterations), plateau detection is permanently neutered.

---

## Finding 5: doc39 §12 Compaction "Conflict" Is a Spec/Implementation Mismatch, Not a Violation

**Severity: P2 (challenge to P0 severity classification)**
**File:** `crates/runtime/src/agent_kernel/compactor.rs`
**Conflict Matrix Status: §12.1 marked CONFLICT (P0-10)**

### Challenge to Conflict Matrix

The conflict matrix declares §12.1 (separate Flash model for compaction) as a P0 conflict because compaction is entirely in-process. This overstates both the violation and the requirement.

**doc39's actual text at §12:** The compaction requirement concerns model-driven compaction with role splitting -- using a different (cheaper/faster) model for compaction than for execution. The goal is to have LLM-driven summarization that preserves semantic meaning better than mechanical truncation.

**What the implementation actually does:**
- `Compactor::compact()` at compactor.rs generates a markdown `CompactionSummary` that is fed back to the execution model as a compacted context. This IS model-driven in the sense that the execution model receives the compacted context and must reason from it.
- The "in-process" nature means the compaction summary is generated by text processing rules rather than a separate LLM call. This is a QUALITY concern, not a compliance breach.

**Why P0 is wrong:**
1. The compaction SUMMARY is sent to the model and the model works from it. This satisfies the semantic intent of doc39 (model-driven context management).
2. The Flash model role exists purely for cost/latency optimization. An in-process compactor that produces equivalent summaries is not a spec violation -- it's a different implementation of the same architectural intent.
3. The actual P0 issues with compaction (P0-11: no L1 state object, P0-12: irreversible) are correctly classified. But §12.1 should be downgraded from P0 conflict to P2 partial.

**Corrected classification:** The conflict matrix should list §12.1 as "Partial -- in-process compaction produces semantic summaries; Flash model role is a cost optimization, not an architectural requirement."

---

## Finding 6: TurnBudget Hard Cap Structure Survives Despite "Compliant" Rating

**Severity: P2 (latent design risk)**
**File:** `crates/runtime/src/agent_kernel/turn_state.rs:27-33`, `crates/runtime/src/native_agent_loop.rs:493-499`
**Conflict Matrix Status: §7.6 (max_tool_calls unlimited) marked COMPLIANT**

### What We Missed

Agent 15 correctly notes that `TurnBudget.max_tool_calls` defaults to 0 which maps to `u32::MAX` via `effective_tool_call_budget`. However, the classification as "compliant" misses two architectural concerns:

1. **The TurnBudget struct at turn_state.rs:27-33 explicitly models hard caps:**
   ```rust
   pub struct TurnBudget {
       pub max_iterations: u32,     // hard cap
       pub max_tool_calls: u32,     // hard cap (0 = unlimited)
       pub max_input_tokens: u64,   // hard cap
       pub max_output_tokens: u64,  // hard cap
       pub max_reasoning_tokens: u64, // hard cap
   }
   ```
   The struct is named "Budget" but operates as a HARD CAP container. The doc39 specification rejects hard caps in favor of progress-based convergence (§7.3), yet the architecture still communicates "caps" as the primary control mechanism.

2. **The test at kernel.rs:283 sets `max_tool_calls: 1` explicitly:**
   ```rust
   max_tool_calls: 1,  // set in Kernel for_request test
   ```
   The test proves the cap mechanism is fully alive -- any caller can set `max_tool_calls` to a finite value and re-activate the rejected cap behavior.

3. **The `effective_tool_call_budget` conversion is a runtime patch, not an architectural fix:**
   ```rust
   // line 499
   let effective_max_tool_calls = turn_budget.max_tool_calls as usize;
   ```
   The variable name `effective_max_tool_calls` carries the semantic of "maximum tools" even when set to u32::MAX. The architecture still thinks in terms of tool caps.

**Why this matters:** A future developer reading the `TurnBudget` struct sees `max_tool_calls: u32` and naturally assumes tool capping is the intended mechanism. The default value hack (0 = unlimited) is a sentinel, not a design. Over time, this invites re-introduction of cap-based control.

---

## Finding 7: OpenClaudeCode Reference Design Drift -- Services Built Then Bypassed

**Severity: P1 (architectural divergence from reference)**
**Files:** Entire `crates/runtime/src/agent_kernel/` and `crates/runtime/src/native_agent_loop.rs`
**Not in conflict matrix -- ENTIRELY MISSED category**

### The Drift

The OpenClaudeCode reference design (from project memory and the `.zip` reference) uses a service-oriented kernel where:
- The kernel orchestrates turns by CALLING its services
- Each service is independently testable
- The native loop is a thin executor (~400 lines)

The Rust implementation inverts this:
- The kernel is a service CONSTRUCTOR (not an orchestrator)
- The native loop does ALL orchestration (2924 lines)
- Services are instantiated but only `turn_controller`, `evidence_ledger`, `permission_gate`, and `context_manager` have their methods called by the loop
- `compactor`, `tcml`, `convergence`, `completion`, `tool_orchestration` are fields on the kernel struct that the loop accesses DIRECTLY rather than through kernel-mediated interfaces

### Specific Architectural Inversions vs. OpenClaudeCode

| OpenClaudeCode Reference Pattern | Rust Implementation | Status |
|---|---|---|
| Kernel::execute_turn() calls compactor.compact() | Native loop calls kernel.context_manager.guard_prepared_request() which triggers in-process guard, not kernel.compactor.compact() | **Bypassed** |
| Kernel::execute_turn() routes ALL tool calls through tcml.mediate() | Native loop calls tcml.mediate_tool_call() for non-concurrent path, parse_tool_arguments() directly for concurrent path | **Partially bypassed** |
| Kernel::execute_turn() calls convergence.observe() then decides | Native loop calls iteration_controller.observe_completed_tool_iteration() which calls convergence_enforcer internally | **Service access pattern inverted** |
| Kernel owns the event loop exit decision | 6 loop owners can independently decide to stop (Agent 1 finding) | **Authority fragmented** |
| Kernel services are behind trait interfaces for testability | Services are concrete structs accessed directly; no trait abstraction | **Testability gap** |

### The Stub Pattern

The most characteristic drift is that `AgentKernel` at kernel.rs:203 literally stubs out its own interface:

```rust
pub fn run_turn<T: LiveHttpTransport>(&self, ...) -> Result<NativeAgentLoopResult, String> {
    let interrupt = std::sync::atomic::AtomicBool::new(false);
    self.run_turn_with_interrupt(transport, request, event_sink, &interrupt)
}

pub fn run_turn_with_interrupt<T: LiveHttpTransport>(&self, ...) -> Result<...> {
    if !self.request_scoped {
        return Err("AgentKernel::run_turn requires request-scoped services...");
    }
    // ... delegates to monolithic loop
    run_native_agent_loop_v2_deepseek_with_interrupt(transport, request, ...)
}
```

The "validation" at line 220-224 is the only kernel-mediated check. Everything else happens in the monolithic loop, which constructs its OWN kernel services copy at line 417.

---

## Finding 8: "Compliant" Permission Resume Model Has No Generation-Counter Protection

**Severity: P1 (race condition in "compliant" system)**
**File:** `crates/runtime/src/native_agent_loop.rs:864-872`
**Conflict Matrix Status: §10.1 and §10.2 marked COMPLIANT**

### Challenge to "Compliant" Classification

The conflict matrix marks §10.1 (plan approval) and §10.2 (tool permission resume) as compliant. The mechanism at line 864-872 appears correct:

```rust
if let Some(pending_tool) = streamed_pending_tool.take() {
    return Ok(loop_result_with_pending(
        NativeAgentLoopStatus::Blocked,
        session, tool_call_count, model_call_count,
        Some(pending_tool),
    ));
}
```

But the issue matrix identifies P1-04 (unlocked window between `decide_permission()` and `execute_tool()`) and P1-05 (pending_native_decision not cleared on cancel). These are classified as separate issues rather than as invalidating the §10.2 compliance claim.

**The architectural problem:** The "compliant" ratings for §10.1/§10.2 assume the resume mechanism works correctly. But the mechanism depends on:
1. `pending_native_decision` being present when resume is called
2. No generation counter / epoch check to prevent stale resume from a cancelled turn

If the turn is cancelled between permission blocking and user decision, `pending_native_decision` at session.rs is NOT cleared (P1-05). The resume will execute the EXACT same tool with the EXACT same parameters from the stale turn. This is an exactly-once/generation violation.

§10.1 should be reclassified to **Partial** because the resume model is correct but the generation guard is missing.

---

## Finding 9: `loop_budget_reached` Event Retained -- "Compliant" Because It Does Not Disable Tools, But Architecture Carries Budget Enforcement DNA

**Severity: P2 (monitoring, not blocking)**
**File:** `crates/runtime/src/agent_kernel/turn_controller.rs:61-66`
**Conflict Matrix Status: §7.4 marked COMPLIANT -- correct classification, but under-analyzed**

### Refinement, Not Rejection

Agent 15 correctly notes that `loop_budget_reached` fires as an event with `stop_with_structured_failure` (no tool disabling). This is technically compliant. However, the EVENT TYPE NAME itself encodes budget thinking. The event `agent.loop_budget_reached` communicates to telemetry consumers that the loop stopped because a budget was exhausted, not because progress plateaued or the task completed. This influences downstream monitoring dashboards that may surface budget exhaustion as an operational concern when it should be surfaced as a convergence signal.

**Recommendation:** Rename to `agent.loop_convergence_stopped` or `agent.loop_natural_exhaustion` to match the architectural intent.

---

## Finding 10: Mid-Loop `evidence_ledger.clear()` Creates ObservationCache / EvidenceLedger State Divergence

**Severity: P1 (state tracking inconsistency)**
**File:** `crates/runtime/src/native_agent_loop.rs:1705-1706`, `crates/runtime/src/agent_kernel/evidence_ledger.rs:249-255`
**Not covered in any audit report**

### New Finding

The `ObservationCache` (turn_state.observation_cache) is NEVER cleared during the loop. It accumulates distinct observation keys across ALL iterations. The `EvidenceLedger` IS cleared at line 1706.

This creates a state divergence:

- **ObservationCache:** Knows about ALL files/commands/patterns seen across the entire turn. `distinct_key_count()` at line 257 returns the CUMULATIVE distinct count.
- **EvidenceLedger (after clear):** Only knows about the current iteration's post-clear evidence. `new_evidence_count()` at line 273 returns the POST-CLEAR count.

When `record_tool_iteration_from_observation_cache()` at line 250 is called:

```rust
pub fn record_tool_iteration_from_observation_cache(
    &mut self,
    distinct_keys_before: usize,  // from ObservationCache (global)
    recovery_results: u32,         // from cleared EvidenceLedger (local)
    duplicate_results: u32,       // from cleared EvidenceLedger (local)
    error_results: u32,           // from cleared EvidenceLedger (local)
) -> (u32, ToolProgressDecision) {
    let distinct_keys_now = self.observation_cache.distinct_key_count();  // GLOBAL
    let new_observation_keys = distinct_keys_now.saturating_sub(distinct_keys_before) as u32;
    let decision = self.progress.record_iteration(IterationProgress {
        new_observation_keys,  // derived from global cache
        recovery_results,      // from local (cleared) ledger
        duplicate_results,     // from local (cleared) ledger
        error_results,         // from local (cleared) ledger
    });
    (new_observation_keys, decision)
}
```

The `new_observation_keys` is derived from the global ObservationCache, but `recovery_results`, `duplicate_results`, and `error_results` come from the post-clear EvidenceLedger which only reflects ONE iteration. This means `IterationProgress` mixes global (cumulative) and local (single-iteration) counts.

**Consequence:** `ToolProgressState::record_iteration()` receives `recovery_results`, `duplicate_results`, and `error_results` that ONLY count the most recent batch, while `new_observation_keys` counts the entire turn. This distorts plateau detection:
- `consecutive_duplicate_iterations` only increments when the SINGLE post-clear batch is all duplicates
- But `consecutive_no_progress_iterations` includes any iteration where no new global observations were made, even if the batch had recovery/error results

These differing counting scopes make plateau detection behave non-intuitively and produce false negatives (long plateaus before detection, P0-09).

---

## Summary

### New Findings (not in any existing audit report)

| # | Finding | Severity |
|---|---|---|
| F1 | AgentKernel is a hollow pass-through, not an orchestrator | P0 |
| F3 | Evidence clearing destroys streaming evidence before convergence | P1 |
| F7 | Services built then bypassed -- complete inversion of OpenClaudeCode pattern | P1 |
| F10 | ObservationCache/EvidenceLedger state divergence from different clearing policies | P1 |

### Challenges to Existing Classifications

| # | What Was Classified | Challenge | New Classification |
|---|---|---|---|
| F2 | §15: visible_finalizer_answer REMOVED | `visible_text_looks_like_transition_statement` IS the same pattern with negated logic | CONFLICT |
| F4 | §2.1: EscalateToCodeEdit as P1 violation | Mechanism is more destructive than reported -- silent model confusion + no de-escalation + split route state | P1 (maintained) with increased detail |
| F5 | §12.1: No Flash model as P0 conflict | In-process compaction satisfies semantic intent; Flash model is optimization, not requirement | P2 Partial |
| F6 | §7.6: max_tool_calls unlimited as COMPLIANT | TurnBudget struct retains cap architecture; sentinel value (0=unlimited) is a hack not a design | Partial (monitor) |
| F8 | §10.1/§10.2: Permission resume as COMPLIANT | No generation counter; P1-04 + P1-05 together invalidate full compliance | Partial |

### Missed Violations (doc39 requirements with no audit coverage)

| § | Requirement | Finding |
|---|---|---|
| §1.1 | AgentKernel as authoritative service facade | AgentKernel has no authority -- it delegates to monolithic loop (F1) |
| §12.3 | Reversible compaction | Compaction IS irreversible (correctly flagged by Agent 8), but the associated evidence clearing at line 1705-1706 makes this WORSE by also destroying non-compacted evidence (F3) |
| (none) | State consistency across tracking systems | ObservationCache (never cleared) vs EvidenceLedger (cleared every iteration) create divergent state views (F10) |
