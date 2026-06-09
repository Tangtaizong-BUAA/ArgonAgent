# Phase 4: Architecture Layer-by-Layer Review

## Layer 0: System Topology

```mermaid
graph TB
    subgraph GUI["GUI Layer (Tauri/React)"]
        App[AppShellLayout]
        RT[RuntimeStatus]
        TS[Transcript]
        PB[PermissionBar]
    end

    subgraph Tauri["Tauri Bridge (desktop/src-tauri)"]
        CMD[runtime_submit_user_message]
        INT[runtime_interrupt_session]
        PERM[submit_permission_decision]
        ACT[active_turns: Mutex&lt;HashSet&gt;]
    end

    subgraph API["Local API Server"]
        LAS[local_api_server.rs]
        LACT[active_turns: Mutex&lt;HashSet&gt;]
    end

    subgraph Facade["Runtime Facade"]
        RFI[runtime_facade_impl.rs]
        PS[PermissionService]
        SS[SessionStore]
    end

    subgraph Kernel["Agent Kernel"]
        AK[AgentKernel]
        NTC[NativeLoopTurnController]
        CE[ConvergenceEnforcer]
        TPS[ToolProgressState]
        EL[EvidenceLedger]
        PG[PermissionGate]
    end

    subgraph TCML["TCML Pipeline"]
        ALIAS[AliasRegistry]
        SCHEMA[SchemaValidator]
        REPAIR[IssueGuidedRepairer]
        REL[RelationalResolver]
        MANIFEST[ManifestBuilder]
    end

    subgraph Provider["Provider Layer"]
        DS[DeepSeek Native]
        QW[Qwen Native]
        COMPAT[Compatible Provider]
        SIDECAR[Sidecar HTTP Transport]
    end

    subgraph Exec["Tool Execution"]
        TE[tool_execution.rs]
        CMD_EXEC[command.rs]
        FS[File System Tools]
        SHELL[Shell Command]
    end

    GUI --> Tauri
    GUI --> API
    Tauri --> Facade
    API --> Facade
    Facade --> Kernel
    Kernel --> TCML
    TCML --> Provider
    Kernel --> Exec
    Exec --> FS
    Exec --> SHELL
```

## Layer 1: Agent Loop Main Cycle

```mermaid
stateDiagram-v2
    [*] --> Preflight
    Preflight --> Interrupted: interrupt flag set
    Preflight --> ToolLimitCheck: ok
    ToolLimitCheck --> Failed: tool_call_count >= max
    ToolLimitCheck --> CompactionCheck: ok
    CompactionCheck --> Compacting: over threshold
    CompactionCheck --> ModelCall: under threshold
    Compacting --> Blocked: no valid summary
    Compacting --> ModelCall: compacted ok
    ModelCall --> HTTPError: 400/5xx
    ModelCall --> StreamResponse: 200
    HTTPError --> RetryDualProtocol: Anthropic→OpenAI fallback
    HTTPError --> Failed: no fallback
    RetryDualProtocol --> StreamResponse: ok
    StreamResponse --> ContentAnalysis
    ContentAnalysis --> VisibleFinalAnswer: text only, no tools
    ContentAnalysis --> ToolExecution: has tool calls
    ContentAnalysis --> TruncationRecovery: stop_reason=length
    ContentAnalysis --> EmptyResponse: no content, no tools
    VisibleFinalAnswer --> Completed: transition pattern match
    VisibleFinalAnswer --> Continue: preamble/transition text
    ToolExecution --> PermissionGate: requires permission
    ToolExecution --> ExecuteDirect: allowed
    PermissionGate --> Blocked: waiting for approval
    PermissionGate --> ExecuteDirect: approved
    ExecuteDirect --> NextIteration
    TruncationRecovery --> NextIteration
    NextIteration --> Preflight: iteration < max
    NextIteration --> LoopExhausted: iteration >= max
    Blocked --> [*]
    Completed --> [*]
    Failed --> [*]
    LoopExhausted --> [*]
    Interrupted --> [*]
```

### Critical Path: 6 Loop Owners

```mermaid
graph LR
    subgraph Owner1["1. NativeLoopTurnController"]
        P1[Preflight: interrupt + tool limit]
    end
    subgraph Owner2["2. Main Loop Body"]
        P2[HTTP status, streaming, visible content]
    end
    subgraph Owner3["3. ToolOrchestrationService"]
        P3[Repeated/alternating batch patterns]
    end
    subgraph Owner4["4. NativeLoopTurnController"]
        P4[Progress aggregation, convergence]
    end
    subgraph Owner5["5. ConvergenceEnforcer"]
        P5[Duplicate/stagnation/plateau decisions]
    end
    subgraph Owner6["6. ToolProgressState"]
        P6[Error plateau thresholds]
    end

    P1 --> P2 --> P3 --> P4 --> P5 --> P6
```

**Issue:** 6 owners with 23 exit paths. No single stop authority. `EscalateToCodeEdit` changes manifest mid-loop between Owner 4-5.

## Layer 2: Event Identity Chain

```mermaid
graph TB
    subgraph Creation["ID Creation Points"]
        TC[tool_call_id<br/>format!: native_loop_v2_tool_{iter}_{idx}]
        PC[provider_tool_call_id<br/>model response or synthetic]
        PM[permission_id<br/>format!: {tc_id}_permission]
        PA[plan_approval_id<br/>format!: {tc_id}_plan_approval]
        LC[ledger_tool_call_id<br/>format!: native_loop_v2_ledger_{iter}_{idx}]
        MC[call_id<br/>session.rs record_model_call_started]
        SC[stream_id<br/>session.rs record_model_stream_delta]
    end

    subgraph Storage["Event Log Storage"]
        EV[EventLog: Vec&lt;KernelEvent&gt;]
        PAYLOAD[Payload carries multiple ID fields]
    end

    subgraph Projection["Conversation Projection"]
        CH[conversation_history.rs]
        OPENAI[OpenAI format: prefers provider_tool_call_id]
        ANTHROPIC[Anthropic format: uses tool_call_id]
    end

    subgraph Merge["Cross-Invocation Merge"]
        MERGE[merge_events_with_id_suffix]
        REWRITE[REWRITABLE_ID_KEYS: id, call_id, stream_id, tool_call_id, provider_tool_use_id]
        MISSING[Missing: permission_id, plan_approval_id]
    end

    subgraph Reverse["Reverse Lookup (Fragile)"]
        REV1[permission_id → tool_call_id: strip_suffix _permission]
        REV2[plan_approval_id → tool_call_id: strip_suffix _plan_approval]
    end

    TC --> EV
    PC --> EV
    PM --> EV
    PA --> EV
    LC --> EV
    MC --> EV
    SC --> EV
    EV --> CH
    EV --> MERGE
    CH --> OPENAI
    CH --> ANTHROPIC
    PM --> REV1
    PA --> REV2
```

**Issues:**
- `ledger_tool_call_id` ≠ `tool_call_id` format (different prefixes, same logical call)
- Merge rewrites 5 ID fields but omits `permission_id` and `plan_approval_id`
- Reverse lookup uses string suffix stripping (fragile, breaks on ID format changes)

## Layer 3: Provider Projection Pipeline

```mermaid
graph TB
    subgraph Input["Event Log → Conversation"]
        EVENTS[EventLog events]
        CH2[conversation_messages_from_event_log]
    end

    subgraph Native["Native Paths"]
        DS_ANT[DeepSeek → Anthropic<br/>live_model_request.rs:305-363]
        DS_OAI[DeepSeek → OpenAI<br/>live_model_request.rs:590-662]
        QW_OAI[Qwen → OpenAI<br/>live_model_request.rs]
    end

    subgraph Compat["Compatible Provider Path"]
        COMPAT_PATH[compatible_provider.rs:117-153]
        BROKEN["❌ Flat strings only<br/>No structured content blocks<br/>Role 'tool' invalid in Anthropic"]
    end

    subgraph Reasoning["Reasoning Replay"]
        RRM[ReasoningReplayManager]
        DS_REASON[DeepSeek reasoning_content]
        ANTH_THINK[Anthropic thinking block]
        DUAL["⚠️ Dual injection:<br/>thinking block + non-standard field"]
    end

    subgraph Fallback["Dual-Protocol Fallback"]
        TRY_ANT[Try Anthropic endpoint]
        ON_400[On 400: retry OpenAI]
        DIRTY["❌ Dirty events:<br/>ContentBlockStarted/Finished leak<br/>from failed Anthropic attempt"]
    end

    EVENTS --> DS_ANT
    EVENTS --> DS_OAI
    EVENTS --> QW_OAI
    EVENTS --> COMPAT_PATH
    DS_ANT --> RRM
    DS_OAI --> RRM
    RRM --> DS_REASON
    RRM --> ANTH_THINK
    DS_ANT --> TRY_ANT
    TRY_ANT --> ON_400
```

## Layer 4: TCML Pipeline (Main Path vs Concurrent Bypass)

```mermaid
graph TB
    subgraph Main["Main Path (Correct)"]
        M1[parse_tool_arguments] --> M2[AliasRegistry.resolve]
        M2 --> M3[ManifestBuilder.check]
        M3 --> M4[SchemaValidator.validate]
        M4 --> M5[IssueGuidedRepairer.repair]
        M5 --> M6[RelationalResolver.apply_defaults]
        M6 --> M7[PermissionGate.evaluate]
        M7 --> M8[ToolExecution.dispatch]
    end

    subgraph Concurrent["Concurrent Path (Bypassed)"]
        C1["parse_tool_arguments (raw)"] --> C2["normalize_tool_id (alias only)"]
        C2 --> C3["Manual ToolExecutionArgs<br/>❌ No schema validation<br/>❌ No repair<br/>❌ No relational defaults<br/>❌ No markdown link repair<br/>❌ Only 6 fields copied"]
        C3 --> C4["execute_tool (direct)"]
    end

    subgraph Safety["Repair Safety (Both Paths)"]
        S1["✅ file.write.content: NEVER repaired"]
        S2["✅ shell.command.command: NEVER repaired"]
        S3["✅ Schema errors: retryable observations"]
    end
```

**Critical Gap:** The concurrent path at `native_agent_loop.rs:1719-1789` constructs `ToolExecutionArgs` manually, bypassing 5 of 7 TCML stages. Read-only tools executed concurrently lose all mediation.

## Layer 5: Permission System

```mermaid
graph TB
    subgraph L1["Layer A: Hard Block (Facade)"]
        A1[command_contains_hard_deny]
        A2["Patterns: rm, git push, curl, |, >, $(, .env, id_rsa"]
        A3["Result: BlockedByPolicy — NO user override"]
    end

    subgraph L2["Layer B: Classifier (PermissionGate)"]
        B1[classify_command_with_reasons]
        B2["Priority chain:<br/>1. DENY_SUBSTRINGS<br/>2. Shell operators<br/>3. Network programs<br/>4. Filesystem mutators<br/>5. Sensitive paths<br/>6. Package installs<br/>7. Allowlist"]
        B3["Result: Allow | Ask | AskPackageInstall | Deny"]
        B4["⚠️ Deny → SafetyCheck → Ask<br/>User CAN approve 'denied' commands"]
    end

    subgraph L3["Layer C: PermissionPolicy"]
        C1[TSV persistent rules]
        C2[Session inline rules]
        C3[PermissionMode: Default | DontAsk | Bypass]
        C4["Result: Allow | Ask | Deny"]
    end

    A1 --> A2 --> A3
    B1 --> B2 --> B3 --> B4
    C1 --> C4
    C2 --> C4
    C3 --> C4
```

## Layer 6: Context & Compaction

```mermaid
graph TB
    subgraph Current["Current Implementation"]
        CUR1[Compactor: pure Rust struct]
        CUR2[compact method: EventLog → CompactionResult]
        CUR3[No HTTP client, no model, no endpoint]
        CUR4[CompactionSummary: markdown blob]
        CUR5[reasoning: 240-char preview]
        CUR6["Token estimation: max(chars/4, word_count)"]
    end

    subgraph Doc39["doc39 §12 Target"]
        DOC1[Separate Flash model role]
        DOC2[LLM-based compaction call]
        DOC3[L1 state object: serializable, reconstructable]
        DOC4[Reversible: model can see old events]
        DOC5[Full reasoning_content preserved]
        DOC6[Proper tokenizer-based estimation]
    end

    subgraph Threshold["Threshold: 192K ✅"]
        T1[min(192000, context_window * 3/4)]
        T2[DeepSeek 256K → 192K]
        T3[Correctly implemented]
    end

    CUR1 -.->|gap| DOC1
    CUR3 -.->|gap| DOC2
    CUR4 -.->|gap| DOC3
    CUR5 -.->|gap| DOC5
    CUR6 -.->|gap| DOC6
```

## Layer 7: Active Turn & Cancel Lifecycle

```mermaid
sequenceDiagram
    participant GUI as GUI
    participant Tauri as Tauri Commands
    participant Facade as RuntimeFacade
    participant Session as SessionStore
    participant Interrupt as InterruptService
    participant Sidecar as Sidecar Process

    Note over GUI,Sidecar: Happy Path Cancel
    GUI->>Tauri: runtime_interrupt_session(session_id)
    Tauri->>Facade: cancel_session(session_id)
    Facade->>Interrupt: interrupt() → AtomicBool = true
    Facade->>Session: transition_to(Cancelled)
    Tauri->>Tauri: release_active_turn(session_id)
    Tauri-->>GUI: { ok: true, state: "Cancelled" }
    
    Note over Sidecar: Up to 250ms gap!
    Sidecar->>Sidecar: poll interrupt every 250ms
    Sidecar->>GUI: ❌ Leaked streaming events

    Note over GUI,Sidecar: TOCTOU Race
    GUI->>Tauri: runtime_submit_user_message (new question)
    Tauri->>Tauri: lock(active_turns)
    Note over Tauri: If old task hasn't released yet:<br/>sees active turn → rejects with<br/>runtime_turn_in_progress
    Note over Tauri: If old task released after new insert:<br/>active_turns clobbered → third turn<br/>can start concurrently
```

## Layer 8: GUI Event Processing

```mermaid
graph TB
    subgraph EventFlow["Event Processing Chain"]
        E1[session events] --> E2[model events]
        E2 --> E3[tool events]
        E3 --> E4[permission events]
        E4 --> E5[plan events]
        E5 --> E6[observability events]
        E6 --> E7[error events]
    end

    subgraph StreamState["Streaming State (Refs)"]
        SS1[terminalStreamClosedRef]
        SS2[activeStreamBufferRef]
        SS3[suppressNextCallCompletedSettleRef]
        SS4[callCompletedSettleTimerRef]
        SS5[seenEventKeysRef: max 5000]
    end

    subgraph Issues["State Leaks & Loss"]
        I1["❌ terminalStreamClosedRef:<br/>drops inter-turn narration"]
        I2["❌ suppressNextCallCompletedSettleRef:<br/>leaks across sessions"]
        I3["❌ seenEventKeysRef:<br/>prunes at 5000, loses dedup"]
        I4["❌ pendingPermissions:<br/>stale closure reference"]
    end

    EventFlow --> StreamState
    StreamState --> Issues
```

## Architecture Debt Summary

| Layer | Critical Issue | Severity |
|---|---|---|
| L1: Agent Loop | 6 loop owners, 23 exit paths, mid-loop manifest change | P1 |
| L2: Event Identity | String-based ID reverse lookup, merge omits permission IDs | P1 |
| L3: Provider | Broken compatible Anthropic path, dirty dual-protocol events | P0 |
| L4: TCML | Concurrent path bypasses 5 of 7 stages | P1 |
| L5: Permission | Classifier Deny ≠ true deny, missing dangerous programs | P1 |
| L6: Compaction | No Flash model, no L1 state, irreversible | P0 |
| L7: Cancel | TOCTOU race, 250ms streaming leak | P1 |
| L8: GUI | Narrative loss, cross-session ref leaks, unbounded memory | P2 |
