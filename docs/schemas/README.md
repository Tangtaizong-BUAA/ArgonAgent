# Phase 0 Schema Drafts

These schemas are Phase 0 contracts, not production migrations. They exist to make architecture decisions executable before scaffold.

Authoritative scope:

- Kernel events and safety/governance payloads: `kernel/`
- Compatible provider and alias mapping: `provider/`
- Bounded long-task autonomy: `task_contract/`

Rules:

- `PlanApproval*` is task governance.
- `Permission*` is safety boundary.
- Compatible providers cannot use `optimization_level = "native"`.
- DeepSeek/Qwen native optimization is outside `CompatibleProviderConfig`.

