# 18 Architecture Decision Records

These ADRs convert the prior architecture direction into explicit decisions with reversal criteria. They are not permanent ideology; each decision remains valid only while its assumptions survive prototypes, threat review, and eval results.

## ADR-001: Runtime API Shape: Tauri IPC vs Local HTTP/WebSocket vs Both

**Status:** Provisional Accepted. Superseded/refined by `33_updated_adr_bundle.md`.

### Context

The product needs a desktop GUI, a CLI/TUI, long-running agent sessions, logs, streaming events, approvals, and later remote/team surfaces. OpenCode shows a runtime HTTP API (`server/server.ts`, `createHttpApi`, `OpenAPI`), while a Tauri desktop app benefits from direct privileged IPC for local filesystem and shell control.

### Options Considered

| Option | Description | Strength | Weakness |
|---|---|---|---|
| Tauri IPC only | GUI calls Rust commands directly. | Simple desktop security boundary; good app packaging. | CLI/TUI and future automation need a second interface. |
| Local HTTP/WebSocket only | GUI, CLI, and automation call a local server. | One client API; streams events naturally. | Harder local permission boundary; more spoofing/CSRF concerns. |
| Both | Tauri IPC owns privileged bootstrap; local HTTP/WebSocket exposes versioned runtime API. | Supports GUI, CLI, streaming, and future remote mode. | More surface area; needs auth token and origin policy. |

### Decision

Use **Tauri IPC first**, with local HTTP/WebSocket treated as a later CLI/remote-runtime adapter after an auth/streaming spike:

- Tauri IPC handles app bootstrap, local capability grants, DB path discovery, and starting/stopping the runtime.
- A loopback-only local HTTP/WebSocket API may handle runtime operations, event streaming, CLI/TUI integration, and eventual remote adapters only after spike validation.
- Every API event uses a versioned envelope from the Product Kernel.

### Consequences

- Runtime can be tested without launching GUI.
- CLI/TUI can reuse the same session/event model.
- GUI must display API-origin and permission-request identity clearly.
- Local server must require a per-launch auth token and bind to loopback only.

### Risks

- Two API paths can drift.
- Local HTTP endpoints can be abused by local malware or browser pages if auth/origin handling is weak.
- Streaming semantics must be identical in GUI and CLI.

### What Would Make Us Reverse This Decision

- A prototype shows maintaining both APIs doubles implementation time without CLI benefits.
- Tauri IPC streaming is sufficient for CLI/TUI through a stable sidecar bridge.
- Threat model finds local HTTP/WebSocket unacceptably risky for the intended user base.

## ADR-002: SQLite Placement: App DB vs Per-Project DB vs Hybrid

**Status:** Accepted for Product Kernel v0.

### Context

The product stores projects, sessions, events, tool calls, permissions, patches, artifacts, eval events, and model telemetry. OpenCode persists sessions and permissions in SQLite-like schema tables. Research workflows also need project-local reproducibility manifests and artifact lineage.

### Options Considered

| Option | Description | Strength | Weakness |
|---|---|---|---|
| App DB only | One global DB under app data. | Simple indexing across projects. | Harder to hand off project history; sensitive research metadata centralized. |
| Per-project DB only | `.researchcode/agent.db` inside each project. | Portable and project-scoped. | Home dashboard and global settings become awkward. |
| Hybrid | App DB for global registry/settings; per-project DB for sessions/artifacts. | Balances discoverability and privacy. | Requires sync/linking rules. |

### Decision

Use a **hybrid** placement:

- App DB: project registry, global settings, model profiles, provider credentials references, UI preferences, eval catalog.
- Per-project DB: sessions, events, messages, tool calls, permissions, patches, research jobs, artifacts, memories, eval runs tied to project files.
- Project DB location defaults to `.researchcode/agent.db`, configurable for sensitive projects.

### Consequences

- Projects remain mostly portable and auditable.
- Sensitive session history can stay near the project and be excluded from cloud sync by policy.
- GUI home can still show all projects.

### Risks

- App DB references can become stale if projects move.
- Per-project DB may accidentally be committed unless `.gitignore` is managed.
- Cross-project eval aggregation needs careful anonymization.

### What Would Make Us Reverse This Decision

- Users primarily work on read-only repositories where writing `.researchcode` is unacceptable.
- Enterprise deployment requires centralized policy/audit storage from day one.
- DB corruption or migration complexity becomes higher than privacy benefit.

## ADR-003: Event Log Schema and Versioning

**Status:** Accepted for Product Kernel v0.

### Context

Agent behavior must be replayable, auditable, and evaluable. Prior architecture included logs, but not enough versioning discipline. Tool calls, model calls, approvals, patches, artifact creation, and eval events must share a durable envelope.

### Options Considered

| Option | Description | Strength | Weakness |
|---|---|---|---|
| Append-only JSONL files | Simple and inspectable. | Easy debugging and replay. | Hard querying and migration. |
| SQLite relational only | Strong queries and transactions. | Fits GUI and reports. | Replay/export less direct. |
| SQLite + exportable JSONL envelope | Store relational projections plus canonical event payload. | Queryable and replayable. | Requires projection code. |

### Decision

Use **SQLite as source of truth** with an append-only `events` table and exportable JSONL:

- Every event has `event_id`, `schema_version`, `project_id`, `session_id`, `task_id`, `sequence`, `event_type`, `actor`, `created_at`, `payload_json`, `hash`, `prev_hash`.
- Derivative tables (`tool_calls`, `patches`, `permissions`, `artifacts`) are projections for UI/query speed.
- No event payload is mutated after write; corrections are new events.

### Consequences

- Enables deterministic replay and eval fixture generation.
- Enables audit chains through `prev_hash`.
- Allows schema evolution through versioned payload parsers.

### Risks

- Event volume can grow quickly.
- Payloads may contain secrets unless redaction is applied before persistence.
- Projection bugs can desync from canonical events.

### What Would Make Us Reverse This Decision

- Prototype shows event sourcing delays the first usable runtime excessively.
- SQLite write contention blocks multi-agent workflows.
- A simpler JSONL-first design proves easier to validate and query for v0.

## ADR-004: Artifact Store Layout

**Status:** Accepted for Product Kernel v0.

### Context

Artifacts include patches, diffs, command output, logs, charts, notebooks, reports, generated scripts, data profiles, and research manifests. Large artifacts should not bloat model context or event rows.

### Options Considered

| Option | Description | Strength | Weakness |
|---|---|---|---|
| Store artifacts in DB blobs | Transactional and simple references. | Easy backup. | DB grows fast; poor for notebooks/images. |
| Store files only by path | Simple and inspectable. | Native tooling can open artifacts. | Weak integrity and lineage. |
| Content-addressed local store | Hash-addressed files with DB metadata. | Dedup, integrity, reproducibility. | More implementation work. |

### Decision

Use a **content-addressed artifact store** per project:

```text
.researchcode/
  artifacts/
    sha256/
      ab/
        abcdef...
  manifests/
  exports/
```

DB `artifacts` rows store `artifact_id`, `kind`, `sha256`, `size_bytes`, `mime_type`, `logical_name`, `source_event_id`, `privacy_class`, and optional `display_path`.

### Consequences

- Large output stays outside event payloads.
- Data lineage can reference immutable artifact hashes.
- GUI can render previews while preserving auditability.

### Risks

- Users may delete artifact files manually.
- Hashing very large datasets can be slow.
- Sensitive artifacts need retention policies and encryption options.

### What Would Make Us Reverse This Decision

- Research workflows mostly reference external data without creating derived artifacts.
- Content-addressing complexity blocks early eval fixture generation.
- Users require all state in one portable DB file.

## ADR-005: Model Provider Scope: Multi-provider Product Layer vs DeepSeek/Qwen Native Optimization Layer

**Status:** Provisional Accepted. Superseded/refined by `27_model_scope_and_provider_layer.md` and `33_updated_adr_bundle.md`.

### Context

The user clarified that the native code agent target is **DeepSeek and Qwen3.6-27B**, each with dedicated optimization modes. ClaudeCode remains an architectural reference for how a runtime scaffolds a model, not a native product target. OpenAI/Claude/GLM/local/custom providers are compatible-only unless a future explicit product decision creates a new native optimization program. They do not enter DeepSeek/Qwen native promotion.

### Options Considered

| Option | Description | Strength | Weakness |
|---|---|---|---|
| Generic multi-provider first | Build router for every major provider. | Broad future compatibility. | Dilutes optimization; contradicts native DeepSeek/Qwen focus. |
| DeepSeek/Qwen only | Hard-code two providers. | Fast focused optimization. | Harder future extension and eval comparison. |
| Two-layer design | Native DeepSeek/Qwen profiles plus abstract provider interface. | Focused product with extensibility. | Requires discipline to avoid broad provider creep. |

### Decision

Adopt a **two-layer model system**:

- Product-native optimized profiles: `DeepSeekV4Profile`, `Qwen36_27BProfile`.
- `CompatibleProviderConfig` for Claude/OpenAI/GLM/local/OpenAI-compatible/Anthropic-compatible/custom providers.
- Compatible providers are allowed only for manual use, fallback with approval, compatibility tests, or baseline evals; they cannot drive kernel assumptions or native eval promotion.

### Consequences

- DeepSeek and Qwen modes can have separate prompt templates, parsers, tool-call recovery, context policies, and retry rules.
- ClaudeCode model scaffolding patterns are absorbed at architecture level: model-aware thinking, schema stability, cache/prefix strategy, strict tool output normalization, and output budget controls.
- Every profile optimization needs eval promotion evidence.

### Risks

- Qwen3.6-27B serving stack and parser behavior may differ across vLLM/SGLang/official tooling.
- DeepSeek V4 API naming, telemetry, and context capabilities may change.
- A narrow native target may slow adoption by users who already use other models.

### What Would Make Us Reverse This Decision

- DeepSeek/Qwen quality is insufficient for core coding workflows after optimization.
- Customers demand Claude/OpenAI as first-class native profiles for business viability.
- Provider APIs converge enough that a generic layer can preserve optimization quality.

## ADR-006: Rust/Tauri/Python Stack Decision

**Status:** Provisional Accepted for prototype choice; implementation still gated by Phase 0 spikes.

### Context

The prior architecture recommended Tauri + React, Rust runtime/CLI, Python research worker, SQLite, and local RPC. ClawCode demonstrates a Rust runtime with typed policy, sandbox, permissions, and compaction modules. Research data workflows require Python ecosystem access.

### Options Considered

| Option | Description | Strength | Weakness |
|---|---|---|---|
| Tauri + Rust + Python | Rust owns runtime/security; Python owns data science. | Strong local safety, distribution, CLI reuse. | More cross-language integration. |
| Electron + TypeScript + Python | Faster GUI/runtime iteration. | JS ecosystem speed. | Larger app, weaker native sandbox story. |
| Tauri + TypeScript service + Python | Familiar web stack with Tauri shell. | Faster model/provider work. | Runtime security split is less clean. |
| Python-first backend | Excellent research tooling. | Data workflows easy. | Coding-agent shell/file permission kernel weaker. |

### Decision

Use **Tauri + React GUI, Rust runtime/CLI, Python research worker, SQLite, Tauri IPC first, and local HTTP/WebSocket only after adapter spike** as the prototype choice.

### Consequences

- Permission manager, patch manager, shell execution, event log, and artifact hashing live in Rust.
- Python worker runs as a sidecar with explicit job manifests and artifact outputs.
- React GUI consumes the same event stream as CLI/TUI.

### Risks

- Rust development speed may be slower for early agent iteration.
- Cross-language schema drift can cause bugs.
- Packaging Python environments cross-platform is non-trivial.

### What Would Make Us Reverse This Decision

- Rust prototypes fail to deliver shell/file sandbox behavior faster or safer than Node.
- Python worker packaging dominates delivery risk.
- A TypeScript runtime prototype proves substantially easier while meeting threat model constraints.

## ADR-007: Research Worker Isolation Model

**Status:** Accepted for Product Kernel-adjacent subsystem.

### Context

Research tasks run generated Python over local CSV/Excel/JSON/Parquet files and produce charts, reports, notebooks, and transformed datasets. This creates risks: data leakage, destructive writes, package install compromise, PII exposure, and irreproducible outputs.

### Options Considered

| Option | Description | Strength | Weakness |
|---|---|---|---|
| In-process Python | Embed Python in runtime. | Low overhead. | Poor isolation and dependency control. |
| Sidecar process per job | Spawn controlled Python job process. | Clear limits and kill behavior. | Startup overhead. |
| Long-lived worker daemon | Persistent Python service. | Fast repeated jobs. | State leakage between jobs. |
| Container-only | Docker/Podman sandbox. | Stronger isolation. | Heavy dependency and poor desktop fit. |

### Decision

Use a **sidecar process per ResearchJob** for v0:

- Job manifest declares inputs, allowed output directory, package environment, network policy, CPU/memory/time limits, privacy class, and expected artifacts.
- Runtime approves package installs and network access separately.
- Worker emits event-log-compatible progress and artifact manifests.

### Consequences

- Reproducibility is tied to job manifests.
- Failed or suspicious jobs can be killed and audited.
- Research Worker remains a first-class module without weakening the runtime kernel.

### Risks

- macOS/Windows/Linux sandbox implementation differs.
- Package installation approval UX can become noisy.
- Large datasets may exceed default time/memory limits.

### What Would Make Us Reverse This Decision

- Startup overhead makes iterative notebook-like analysis unusable.
- OS process controls are insufficient without containers.
- Users demand managed remote compute for large datasets before local sidecar matures.

## ADR-008: Patch Manager Read-before-write Invariant

**Status:** Accepted as non-negotiable Product Kernel invariant.

### Context

ClaudeCode and OpenCode both show structured file editing with old/new strings, read state, ambiguous match detection, and permission checks. Accidental file corruption is one of the highest-risk agent failures.

### Options Considered

| Option | Description | Strength | Weakness |
|---|---|---|---|
| Allow direct writes from model | Fastest implementation. | Simple. | High corruption risk; poor review. |
| Structured patch only | Model emits patch/diff. | Reviewable. | Patch failures can be common. |
| Read-before-write plus structured patch/edit | Runtime requires prior snapshot and validates against current file. | Strong safety and traceability. | More state tracking. |

### Decision

Adopt **read-before-write plus structured patch/edit**:

- Runtime must have a recorded file snapshot/hash before any model-originated write.
- PatchProposal includes base file hash, target paths, hunks, intent, generated diff, and required permissions.
- Apply fails if base hash mismatches, hunk is ambiguous, file path is outside scope, or sensitive path policy denies it.
- Shell writes are either blocked or lifted into PatchProposal when possible.

### Consequences

- GUI can show accurate diffs before apply.
- Eval can measure hallucinated-file and stale-write failures.
- Human approval can be based on concrete before/after content.

### Risks

- Some legitimate generated files do not have a prior snapshot; creation path must be explicit.
- Large file diffs can be expensive.
- Formatting tools may modify files after patch, requiring separate events.

### What Would Make Us Reverse This Decision

- Empirical eval shows the invariant blocks common coding tasks more than it prevents harm.
- A stronger filesystem sandbox with automatic rollback makes direct writes acceptable.
- User explicitly chooses a high-trust unattended mode with separate project backup.
