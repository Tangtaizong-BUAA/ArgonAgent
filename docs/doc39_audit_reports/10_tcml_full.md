# Audit 10: TCML Layer Full Audit vs doc39

**Date:** 2026-05-19 | **Files:** All 13 files in crates/runtime/src/tcml/

## Overall Alignment: ~58%

## 1. Per-File Assessment

| File | Lines | Alignment | Key Gap |
|---|---|---|---|
| mod.rs | 50 | 100% | Pure re-export |
| alias_registry.rs | 265 | 80% | 116 aliases (>>50 spec floor); no edit-distance fuzzy suggest |
| schema_validator.rs | 115 | 50% | Only checks missing required keys; no JSON Schema type validation |
| issue_guided_repairer.rs | 251 | 60% | Repairs BEFORE validation (one-pass), not doc39's validate→repair→re-validate |
| repair_catalog.rs | 109 | 40% | Only 2 of 5 spec rules implemented |
| relational_resolver.rs | 83 | 50% | offset/limit defaults exist; NO base_hash injection |
| error_factory.rs | 60 | 30% | 5 of 9 error codes; ModelReadableToolError missing 4 fields |
| content_extractor.rs | 3 | 20% | Thin wrapper; no ExtractedContentCall struct, no scan() |
| parser.rs | 1201 | 90% | Good DSML/XML/JSON parsing; no ValidationIssue integration |
| contract.rs | 1036 | 70% | Monolithic mediation, not chained pipeline |
| manifest.rs | 479 | 100% | Full manifest builder |
| pipeline.rs | 89 | 30% | Named "pipeline" but chains only 2 of 11 steps |
| streaming_accumulator.rs | 9 | 50% | Thin re-export wrapper |

## 2. 11-Step Pipeline Check

| Step | Status |
|---|---|
| 1. AliasRegistry.resolve | PARTIAL (in contract.rs, not standalone) |
| 2. SchemaValidator.validate | PARTIAL (missing keys only, no type checks) |
| 3. IssueGuidedRepairer.repair | PARTIAL (before validation, not after) |
| 4. SchemaValidator.validate (re-validate) | **MISSING** |
| 5. RelationalInvariantResolver | PARTIAL (no base_hash) |
| 6. ProviderCapability check | **MISSING** |
| 7. PermissionPolicy.evaluate | **MISSING** (outside TCML) |
| 8. ToolDispatcher | **MISSING** (outside TCML) |
| 9. ToolExecutor | **MISSING** (outside TCML) |
| 10. ResultFormatter | **MISSING** |
| 11. ConversationHistory append | **MISSING** |

## 3. RepairCatalog: 2/5 Rules

| Rule | Status |
|---|---|
| strip_optional_null | IMPLEMENTED |
| parse_stringified_array | **MISSING** |
| wrap_bare_string_to_array | **MISSING** |
| unwrap_markdown_link_path | IMPLEMENTED |
| empty_object_to_array | **MISSING** |

`never_apply_to` coverage: 16 tuples (2 required + 14 extra protective rules). More conservative than spec.

## 4. ModelReadableToolError Gaps

**Error codes:** 5 of 9 spec variants present. Missing: PlanModeRequired, SafetyDenied, RelationalInvariantFailed, BudgetExhausted.

**Fields:** Only 3 of 7 spec fields: error_code, tool_name, short_message, retryable. Missing: field_errors, retry_hint, retry_example, counts_against_budget.

## 5. Key Missing Items

**Critical:**
- Re-validate after repair (step 4)
- base_hash injection for file.write/edit (eval R10)
- field_errors, retry_hint, retry_example fields on error
- 3 missing repair rules
- 4 missing error code variants

**High:**
- Unified pipeline (only 2 of 11 steps)
- Provider capability check
- Permission policy integration
- Tool dispatcher concurrency control
- Result formatter
- Conversation history append
