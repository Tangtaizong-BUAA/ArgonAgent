# 24 Revised First 30 Codex Tasks

These tasks replace the earlier first-30 list. They are intentionally smaller and harder-edged. The first 8 tasks are Phase 0 hardening/prototypes and must not start large product implementation. Each task must be executable by Codex in one session.

Convergence update: `docs/implementation/phase0_execution_order.md` is the authoritative execution order before scaffold. This file remains the task backlog, but scaffold is blocked until the Phase 0 go/no-go checklist passes.

## Phase 0: Hardening / Prototypes First

### Task 01: Create Kernel Schema Draft Package

**Goal:** Define versioned JSON Schema files for `KernelEvent`, `PlanApprovalRequest`, `PlanApprovalDecision`, `PermissionRequest`, `PermissionDecision`, `PatchProposal`, `ContextBundle`, `ToolSpec`, `ModelAdapterConfig`, `CompatibleProviderConfig`, `ModelAliasMapping`, and `EvalEvent`.

**Why this matters:** All future modules depend on stable payload contracts.

**Input context:** `21_product_kernel_v0.md`, `18_architecture_decision_records.md`.

**Files to create/modify:** `docs/schemas/kernel/*.schema.json`, `docs/schemas/README.md`.

**Implementation plan:**

1. Create a docs-only schema directory.
2. Draft one JSON Schema per kernel type.
3. Add schema version fields and required ids.
4. Add examples for one event, one plan approval request, one patch, one permission request, and one compatible provider config.

**Acceptance criteria:** `rg '"schema_version"' docs/schemas/kernel` finds every schema; plan approval is not represented as `request_type = plan`; compatible providers cannot use `optimization_level = native`.

**Test command:** `find docs/schemas/kernel -name '*.json' -maxdepth 1 -print`

**Cannot do:** Do not create runtime source code or app packages.

**Risk:** Schemas become too detailed before prototypes.

**Rollback:** Delete `docs/schemas/kernel` and restore docs-only state.

**Human approval required:** No.

### Task 02: Build Event Log Replay Paper Prototype

**Goal:** Write a small docs fixture showing a full event sequence for one coding task from user message to patch applied and test passed.

**Why this matters:** Validates that the event model can reconstruct GUI state.

**Input context:** `21_product_kernel_v0.md`, `23_gui_user_flows.md`.

**Files to create/modify:** `docs/prototypes/event_log_replay/coding_task_sequence.jsonl`, `docs/prototypes/event_log_replay/README.md`.

**Implementation plan:**

1. Create a JSONL event sequence with 20-30 events.
2. Include hash/prev_hash placeholders.
3. Add README explaining how GUI reconstructs state.
4. Note gaps where schema needs clarification.

**Acceptance criteria:** Sequence includes session, model, tool, permission, patch, artifact, eval events.

**Test command:** `wc -l docs/prototypes/event_log_replay/coding_task_sequence.jsonl`

**Cannot do:** Do not build an event log implementation.

**Risk:** Prototype hides schema ambiguity.

**Rollback:** Remove `docs/prototypes/event_log_replay`.

**Human approval required:** No.

### Task 03: Patch Invariant Fixture Spec

**Goal:** Specify fixtures for read-before-write, stale hash, ambiguous patch, protected path, and generated-file creation.

**Why this matters:** Patch safety is a kernel invariant.

**Input context:** `18_architecture_decision_records.md` ADR-008, `20_eval_suite_v0.md` P-01..P-05.

**Files to create/modify:** `docs/prototypes/patch_invariants/fixture_spec.md`.

**Implementation plan:**

1. Describe five fixture directories.
2. For each, list initial files, model proposal, expected validation result.
3. Include base hash expectations.
4. Include pass/fail rules.

**Acceptance criteria:** All five patch evals can be implemented directly from the spec.

**Test command:** `rg 'P-0[1-5]' docs/prototypes/patch_invariants/fixture_spec.md`

**Cannot do:** Do not implement patch manager.

**Risk:** Spec may miss binary file or symlink cases.

**Rollback:** Delete fixture spec.

**Human approval required:** No.

### Task 04: Permission UI Data Contract Prototype

**Goal:** Define the exact data fields the GUI must show for command, file write, package install, cloud model, and protected path approvals.

**Why this matters:** Prevents vague approvals and spoofing.

**Input context:** `19_threat_model.md`, `23_gui_user_flows.md`.

**Files to create/modify:** `docs/prototypes/permission_contract.md`.

**Implementation plan:**

1. Add one approval payload example per request type.
2. Mark required visible fields.
3. Mark fields that must be included in `request_hash`.
4. Add denial feedback shape.

**Acceptance criteria:** Each request type has raw request, normalized summary, affected paths, risk, policy rule, and decision options.

**Test command:** `rg 'request_hash|normalized_summary|affected_paths' docs/prototypes/permission_contract.md`

**Cannot do:** Do not implement UI.

**Risk:** Overly verbose approvals hurt UX.

**Rollback:** Delete prototype doc.

**Human approval required:** No.

### Task 05: DeepSeek Native Parser Eval Fixture Design

**Goal:** Create a docs-only fixture spec for DeepSeek XML/JSON tool call parsing, hallucinated tool names, and malformed arguments.

**Why this matters:** DeepSeek optimization must be measurable.

**Input context:** `17_claim_traceability_matrix.md` rows 56-72, `20_eval_suite_v0.md` DS-01..DS-02.

**Files to create/modify:** `docs/prototypes/deepseek_parser_eval.md`.

**Implementation plan:**

1. Add raw model-output examples for valid XML, valid JSON, malformed JSON, wrong tool name.
2. Define expected parser output and confidence.
3. Define fail-fast cases where tool must not execute.
4. Define metrics.

**Acceptance criteria:** At least 12 parser fixtures with expected parse/deny results.

**Test command:** `rg '^### Fixture' docs/prototypes/deepseek_parser_eval.md | wc -l`

**Cannot do:** Do not implement parser.

**Risk:** Fixtures may not match live DeepSeek V4 outputs.

**Rollback:** Delete prototype doc.

**Human approval required:** No.

### Task 06: Qwen3.6-27B Native Mode Evidence Refresh

**Goal:** Create a source-backed doc that pins Qwen3.6-27B context, tool-call, reasoning, serving, and parser assumptions.

**Why this matters:** Current Qwen evidence is weaker than DeepSeek evidence.

**Input context:** User-provided Qwen3.6-27B URL, `15_native_deepseek_qwen_modes.md`, `17_claim_traceability_matrix.md` rows 85-90.

**Files to create/modify:** `docs/agent_architecture_planning/25_qwen36_27b_evidence_refresh.md`.

**Implementation plan:**

1. Review local docs and Qwen URLs if network is available.
2. Record exact claims with source links/paths.
3. Separate confirmed facts from assumptions.
4. Update Qwen native profile risks.

**Acceptance criteria:** Every Qwen profile assumption is marked Confirmed/Assumption/Unknown.

**Test command:** `rg 'Confirmed|Assumption|Unknown' docs/agent_architecture_planning/25_qwen36_27b_evidence_refresh.md`

**Cannot do:** Do not change architecture decisions or code based on unverified assumptions.

**Risk:** Network may be unavailable; mark unknowns explicitly.

**Rollback:** Delete `25_qwen36_27b_evidence_refresh.md`.

**Human approval required:** No.

### Task 07: Research Worker Job Manifest Prototype

**Goal:** Draft a concrete ResearchJob manifest and artifact lineage example for CSV profiling.

**Why this matters:** Research Worker needs reproducibility before implementation.

**Input context:** `22_research_worker_deep_spec.md`.

**Files to create/modify:** `docs/prototypes/research_worker/job_manifest_example.json`, `docs/prototypes/research_worker/lineage_example.json`, `docs/prototypes/research_worker/README.md`.

**Implementation plan:**

1. Create one job manifest for `csv-quality-small`.
2. Create matching lineage example.
3. Include privacy classification and sandbox settings.
4. Add README explaining lifecycle.

**Acceptance criteria:** Manifest includes inputs, environment, limits, privacy, outputs, and event ids.

**Test command:** `rg 'privacy_class|sandbox|environment|outputs' docs/prototypes/research_worker`

**Cannot do:** Do not run Python or create actual data analysis code.

**Risk:** Manifest too idealized; later worker may need fields changed.

**Rollback:** Delete `docs/prototypes/research_worker`.

**Human approval required:** No.

### Task 08: Threat Model Regression Checklist

**Goal:** Convert `19_threat_model.md` into a concise checklist for PR/release review.

**Why this matters:** Threat model must influence implementation, not sit as prose.

**Input context:** `19_threat_model.md`.

**Files to create/modify:** `docs/security/kernel_v0_security_checklist.md`.

**Implementation plan:**

1. Extract kernel v0 security requirements.
2. Add checklist items for command, patch, model, artifact, research worker.
3. Add "must fail release" criteria.
4. Link each item back to threat ids or sections.

**Acceptance criteria:** Checklist has at least 25 actionable checkbox items and fail-release criteria.

**Test command:** `rg '^- \\[ \\]' docs/security/kernel_v0_security_checklist.md | wc -l`

**Cannot do:** Do not implement security features.

**Risk:** Checklist may duplicate threat model; keep it operational.

**Rollback:** Delete checklist file.

**Human approval required:** No.

## Phase 1: Kernel Interfaces and Fixtures

### Task 09: Create Eval Fixture Directory Skeleton

**Goal:** Create empty fixture directories and README files for the 30 eval cases.

**Why this matters:** Eval-first development needs stable fixture paths.

**Input context:** `20_eval_suite_v0.md`.

**Files to create/modify:** `eval/fixtures/**/README.md`.

**Implementation plan:** Create directory skeleton only; include each case goal and pass command in README.

**Acceptance criteria:** All IDs C-01..R-05 have matching fixture directories.

**Test command:** `find eval/fixtures -name README.md | wc -l`

**Cannot do:** Do not implement fixtures yet.

**Risk:** Directory churn if IDs change.

**Rollback:** Remove `eval/fixtures`.

**Human approval required:** No.

### Task 10: Write Eval Case Index

**Goal:** Add a machine-readable eval case index matching `20_eval_suite_v0.md`.

**Why this matters:** Future runner needs structured case metadata.

**Input context:** `20_eval_suite_v0.md`.

**Files to create/modify:** `eval/eval_cases.v0.json`.

**Implementation plan:** Encode case id, category, fixture path, allowed tools, pass command, compared profiles.

**Acceptance criteria:** Contains 30 cases and no duplicate ids.

**Test command:** `python3 -m json.tool eval/eval_cases.v0.json >/dev/null`

**Cannot do:** Do not write eval runner.

**Risk:** Metadata drifts from markdown.

**Rollback:** Delete JSON index.

**Human approval required:** No.

### Task 11: Define Tool Registry Contract

**Goal:** Write a docs contract for built-in v0 tools and their permission policies.

**Why this matters:** Model prompts, GUI approvals, and runtime execution need one source of truth.

**Input context:** `21_product_kernel_v0.md`, `17_claim_traceability_matrix.md`.

**Files to create/modify:** `docs/runtime/tool_registry_contract.md`.

**Implementation plan:** Document read, rg, patch, shell, artifact, data_profile, python_worker stubs.

**Acceptance criteria:** Each tool has id, schema summary, read-only/destructive classification, output budget, permission policy.

**Test command:** `rg 'tool_id|permission_policy|output_budget' docs/runtime/tool_registry_contract.md`

**Cannot do:** Do not implement tools.

**Risk:** Contract may overfit current assumptions.

**Rollback:** Delete contract.

**Human approval required:** No.

### Task 12: Model Adapter Contract for DeepSeek and Qwen

**Goal:** Create a model adapter contract with separate DeepSeek/Qwen prompt/parser/profile fields.

**Why this matters:** Prevents generic OpenAI-compatible mode from erasing native optimizations.

**Input context:** `18_architecture_decision_records.md` ADR-005, `10_model_optimization_architecture.md`.

**Files to create/modify:** `docs/runtime/model_adapter_contract.md`.

**Implementation plan:** Define request, stream delta, tool-call parse result, error class, profile metadata.

**Acceptance criteria:** DeepSeek and Qwen sections have distinct parser, context, retry, and reasoning policies.

**Test command:** `rg 'DeepSeek|Qwen|parser|reasoning|context' docs/runtime/model_adapter_contract.md`

**Cannot do:** Do not call model APIs.

**Risk:** Contract may miss provider-specific streaming details.

**Rollback:** Delete contract.

**Human approval required:** No.

### Task 13: ContextBundle Construction Spec

**Goal:** Define how repo maps, file snippets, tool outputs, memories, and research profiles enter context.

**Why this matters:** Good agents are context compilers, not chat wrappers.

**Input context:** `07_how_good_agents_make_llms_better.md`, `21_product_kernel_v0.md`.

**Files to create/modify:** `docs/runtime/context_bundle_spec.md`.

**Implementation plan:** Add item priority, trust labels, token budgets, privacy filters, DeepSeek prefix policy, Qwen parser metadata.

**Acceptance criteria:** Spec includes inclusion/exclusion rules and token budget example.

**Test command:** `rg 'trust_level|token_budget|privacy|prefix|qwen' docs/runtime/context_bundle_spec.md`

**Cannot do:** Do not implement retrieval.

**Risk:** Budget numbers need later tuning.

**Rollback:** Delete spec.

**Human approval required:** No.

### Task 14: Permission Decision State Table

**Goal:** Specify permission state transitions and invalid transitions.

**Why this matters:** Approvals must be auditable and race-safe.

**Input context:** `21_product_kernel_v0.md`, `23_gui_user_flows.md`.

**Files to create/modify:** `docs/runtime/permission_state_table.md`.

**Implementation plan:** Define requested, expired, allowed, denied, modified, consumed states and request_hash validation.

**Acceptance criteria:** Invalid transition table includes stale request, duplicate decision, modified command, expired request.

**Test command:** `rg 'Invalid|request_hash|expired|duplicate' docs/runtime/permission_state_table.md`

**Cannot do:** Do not implement permission manager.

**Risk:** Too many states for v0; keep minimum viable.

**Rollback:** Delete doc.

**Human approval required:** No.

### Task 15: Artifact Store Layout Spec

**Goal:** Define artifact directory layout and DB metadata fields in detail.

**Why this matters:** Research and coding outputs need integrity and privacy.

**Input context:** ADR-004, `21_product_kernel_v0.md`, `22_research_worker_deep_spec.md`.

**Files to create/modify:** `docs/runtime/artifact_store_spec.md`.

**Implementation plan:** Add path layout, hash rules, retention policies, privacy classes, export/delete behavior.

**Acceptance criteria:** Spec covers diff, command output, chart, report, notebook, data profile, manifest.

**Test command:** `rg 'sha256|retention|privacy_class|manifest' docs/runtime/artifact_store_spec.md`

**Cannot do:** Do not implement file storage.

**Risk:** Retention policy may need legal review.

**Rollback:** Delete spec.

**Human approval required:** No.

### Task 16: SQLite Schema Draft

**Goal:** Draft SQL tables for kernel event log and projections.

**Why this matters:** Storage design must match event model before code.

**Input context:** `18_architecture_decision_records.md` ADR-002/003.

**Files to create/modify:** `docs/storage/sqlite_schema_v0.sql`, `docs/storage/README.md`.

**Implementation plan:** Add tables: projects, sessions, events, permissions, patches, artifacts, model_calls, tool_calls, eval_events.

**Acceptance criteria:** SQL parses with sqlite and includes foreign keys where possible.

**Test command:** `sqlite3 :memory: < docs/storage/sqlite_schema_v0.sql`

**Cannot do:** Do not create runtime migrations.

**Risk:** Schema may evolve after prototype.

**Rollback:** Delete schema draft.

**Human approval required:** No.

## Phase 2: Prototype Runners and Validators

### Task 17: JSON Schema Validation Script Prototype

**Goal:** Add a small validation script for docs schema examples.

**Why this matters:** Schemas need executable checks.

**Input context:** Task 01 outputs.

**Files to create/modify:** `scripts/validate_kernel_schemas.py`, `docs/schemas/examples/*.json`.

**Implementation plan:** Use Python standard library where possible; validate presence of required fields if no jsonschema dependency.

**Acceptance criteria:** Script exits nonzero on missing required fields.

**Test command:** `python3 scripts/validate_kernel_schemas.py`

**Cannot do:** Do not add third-party dependency unless already present.

**Risk:** Lightweight validation may be incomplete.

**Rollback:** Delete script/examples.

**Human approval required:** No.

### Task 18: Event Replay Validator Prototype

**Goal:** Add a script that reads JSONL prototype events and checks sequence, hash links, and required state transitions.

**Why this matters:** Event log must be replayable.

**Input context:** Task 02 outputs.

**Files to create/modify:** `scripts/validate_event_sequence.py`.

**Implementation plan:** Parse JSONL, check monotonic sequence, known event types, prev_hash chain placeholders or real hashes.

**Acceptance criteria:** Validator passes known-good sequence and fails shuffled sequence fixture.

**Test command:** `python3 scripts/validate_event_sequence.py docs/prototypes/event_log_replay/coding_task_sequence.jsonl`

**Cannot do:** Do not implement persistent event log.

**Risk:** Prototype state machine too shallow.

**Rollback:** Delete script.

**Human approval required:** No.

### Task 19: Patch Fixture Generator

**Goal:** Create minimal patch invariant fixtures from Task 03 spec.

**Why this matters:** Patch manager work needs concrete files.

**Input context:** `docs/prototypes/patch_invariants/fixture_spec.md`.

**Files to create/modify:** `eval/fixtures/patch/*`.

**Implementation plan:** Create small files and expected metadata only.

**Acceptance criteria:** P-01..P-05 directories contain initial file and expected result README.

**Test command:** `find eval/fixtures/patch -maxdepth 2 -type f | sort`

**Cannot do:** Do not implement patch validator.

**Risk:** Fixtures too artificial.

**Rollback:** Remove `eval/fixtures/patch`.

**Human approval required:** No.

### Task 20: DeepSeek Parser Golden Fixtures

**Goal:** Convert DeepSeek parser fixture spec into JSON golden cases.

**Why this matters:** Parser implementation should be test-driven.

**Input context:** Task 05 output.

**Files to create/modify:** `eval/fixtures/deepseek/parser_golden.json`.

**Implementation plan:** Encode raw output, expected tool id, expected args, expected action execute/retry/deny.

**Acceptance criteria:** At least 12 cases and valid JSON.

**Test command:** `python3 -m json.tool eval/fixtures/deepseek/parser_golden.json >/dev/null`

**Cannot do:** Do not write parser.

**Risk:** Live model outputs may differ.

**Rollback:** Delete golden file.

**Human approval required:** No.

### Task 21: Qwen Parser Golden Fixtures

**Goal:** Create Qwen3.6-27B tool/reasoning parser golden cases based on refreshed evidence.

**Why this matters:** Qwen native mode must be independent from DeepSeek mode.

**Input context:** Task 06 output.

**Files to create/modify:** `eval/fixtures/qwen/parser_golden.json`.

**Implementation plan:** Encode Qwen-style tool calls, reasoning separation, malformed outputs, retry cases.

**Acceptance criteria:** At least 10 cases, each marked confirmed or assumed.

**Test command:** `python3 -m json.tool eval/fixtures/qwen/parser_golden.json >/dev/null`

**Cannot do:** Do not invent unsupported parser behavior as confirmed.

**Risk:** Evidence may remain insufficient; mark assumptions.

**Rollback:** Delete golden file.

**Human approval required:** No.

### Task 22: Research CSV Fixture

**Goal:** Create the smallest CSV quality fixture and expected profile output.

**Why this matters:** Research Worker needs executable non-code-agent eval.

**Input context:** `20_eval_suite_v0.md` R-01, `22_research_worker_deep_spec.md`.

**Files to create/modify:** `eval/fixtures/research/csv-quality-small/*`.

**Implementation plan:** Add CSV with missing values, duplicate rows, suspicious PII column; add expected JSON summary.

**Acceptance criteria:** Fixture contains input CSV and expected issues.

**Test command:** `rg 'missing|duplicate|sensitive' eval/fixtures/research/csv-quality-small`

**Cannot do:** Do not implement profiler.

**Risk:** Fixture overfits simple heuristics.

**Rollback:** Remove fixture directory.

**Human approval required:** No.

## Phase 3: Minimal Kernel Prototypes

### Task 23: Patch Validator Prototype

**Goal:** Implement a small standalone patch validator script against patch fixtures.

**Why this matters:** Tests read-before-write invariant before runtime exists.

**Input context:** Patch fixture outputs.

**Files to create/modify:** `scripts/prototype_patch_validator.py`.

**Implementation plan:** Check file hash, path allowlist, old-string match count, creation path.

**Acceptance criteria:** P-01 pass, P-02 ambiguous fail, P-03 stale fail, P-05 protected fail.

**Test command:** `python3 scripts/prototype_patch_validator.py eval/fixtures/patch`

**Cannot do:** Do not apply real patches to workspace files.

**Risk:** Prototype diverges from future Rust implementation.

**Rollback:** Delete script.

**Human approval required:** No.

### Task 24: Command Permission Classifier Prototype

**Goal:** Implement docs/prototype classifier for safe/ask/deny command examples.

**Why this matters:** Shell permission policy needs concrete behavior.

**Input context:** `19_threat_model.md`, permission contract.

**Files to create/modify:** `eval/fixtures/shell/permission_cases.json`, `scripts/prototype_command_classifier.py`.

**Implementation plan:** Classify exact commands and dangerous metacharacter examples conservatively.

**Acceptance criteria:** Denies destructive/path-exfil examples; asks for package install; allows simple read/test commands.

**Test command:** `python3 scripts/prototype_command_classifier.py eval/fixtures/shell/permission_cases.json`

**Cannot do:** Do not execute commands.

**Risk:** Regex prototype is not final shell parser.

**Rollback:** Delete script and fixture.

**Human approval required:** No.

### Task 25: ContextBundle Builder Paper Prototype

**Goal:** Implement a non-model script that assembles a ContextBundle JSON from fixture repo snippets.

**Why this matters:** Context policy should be testable without models.

**Input context:** `docs/runtime/context_bundle_spec.md`.

**Files to create/modify:** `scripts/prototype_context_bundle.py`, `eval/fixtures/context/simple_repo`.

**Implementation plan:** Read selected files, apply token estimates, trust labels, privacy classes, output bundle JSON.

**Acceptance criteria:** Output includes repo, file snippets, omitted items, budget, prefix hash placeholder.

**Test command:** `python3 scripts/prototype_context_bundle.py eval/fixtures/context/simple_repo`

**Cannot do:** Do not call model APIs.

**Risk:** Token estimator inaccurate.

**Rollback:** Delete script/fixture.

**Human approval required:** No.

### Task 26: Research Profile Prototype

**Goal:** Implement a small CSV profiler script for R-01 fixture.

**Why this matters:** Tests Research Worker data-profile contract before sandbox implementation.

**Input context:** R-01 fixture, Research Worker manifest.

**Files to create/modify:** `scripts/prototype_csv_profiler.py`.

**Implementation plan:** Use Python standard library or pandas if available; detect row count, missing values, duplicates, sensitive columns.

**Acceptance criteria:** Output matches expected fixture issues.

**Test command:** `python3 scripts/prototype_csv_profiler.py eval/fixtures/research/csv-quality-small/input.csv`

**Cannot do:** Do not create sidecar process manager.

**Risk:** Prototype not scalable to large data.

**Rollback:** Delete script.

**Human approval required:** No.

### Task 27: Eval Result Format Prototype

**Goal:** Define and generate one sample EvalResult JSON from prototype scripts.

**Why this matters:** Promotion rules need structured result data.

**Input context:** `20_eval_suite_v0.md`, `21_product_kernel_v0.md`.

**Files to create/modify:** `eval/results/sample_eval_result.v0.json`, `docs/eval/result_format.md`.

**Implementation plan:** Encode case id, profile id, metrics, verdict, artifact refs, event ids.

**Acceptance criteria:** JSON valid and includes security metrics.

**Test command:** `python3 -m json.tool eval/results/sample_eval_result.v0.json >/dev/null`

**Cannot do:** Do not build full eval runner.

**Risk:** Result shape may change.

**Rollback:** Delete sample/result doc.

**Human approval required:** No.

## Phase 4: Runtime Setup Preparation

### Task 28: Repo Scaffold Decision Check

**Goal:** Write a final pre-scaffold checklist confirming stack, workspace layout, and package boundaries.

**Why this matters:** Prevents premature repo generation before kernel decisions are accepted.

**Input context:** ADRs, Product Kernel v0, threat model.

**Files to create/modify:** `docs/implementation/pre_scaffold_checklist.md`.

**Implementation plan:** List Rust crates, Tauri app, React app, Python worker, eval, docs, schema directories and acceptance gates.

**Acceptance criteria:** Checklist references ADR-001..ADR-008 and kernel acceptance criteria.

**Test command:** `rg 'ADR-00[1-8]|Kernel v0' docs/implementation/pre_scaffold_checklist.md`

**Cannot do:** Do not scaffold project code.

**Risk:** Checklist becomes bureaucratic.

**Rollback:** Delete checklist.

**Human approval required:** Yes, before actual scaffold.

### Task 29: Minimal Rust Runtime Spike Plan

**Goal:** Write a detailed spike plan for a Rust runtime crate that only validates events, permissions, and patches.

**Why this matters:** Rust implementation should start with kernel behavior, not GUI.

**Input context:** `21_product_kernel_v0.md`, ClawCode evidence in `17_claim_traceability_matrix.md`.

**Files to create/modify:** `docs/implementation/rust_runtime_spike_plan.md`.

**Implementation plan:** Define crate names, modules, tests, no network/model calls, fixture-based validation.

**Acceptance criteria:** Plan includes exact first 10 Rust tests to write.

**Test command:** `rg 'test_' docs/implementation/rust_runtime_spike_plan.md`

**Cannot do:** Do not create Rust crates.

**Risk:** Plan may over-index on Rust before prototype evidence.

**Rollback:** Delete plan.

**Human approval required:** No.

### Task 30: First Implementation Gate Review

**Goal:** Create a review document deciding whether implementation can start after Tasks 01-29.

**Why this matters:** Forces explicit review of evidence, security, eval, and kernel specs.

**Input context:** Outputs of Tasks 01-29.

**Files to create/modify:** `docs/implementation/implementation_gate_review.md`.

**Implementation plan:** Summarize completed artifacts, unresolved risks, blocked decisions, and go/no-go checklist.

**Acceptance criteria:** Includes Go/No-Go decision fields for runtime, GUI, DeepSeek, Qwen, Research Worker.

**Test command:** `rg 'Go/No-Go|DeepSeek|Qwen|Research Worker|Runtime' docs/implementation/implementation_gate_review.md`

**Cannot do:** Do not start implementation inside this task.

**Risk:** Review may be skipped; make it required before scaffold.

**Rollback:** Delete review doc.

**Human approval required:** Yes.
