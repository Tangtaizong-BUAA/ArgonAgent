# Event Log Replay Prototype

This Phase 0 prototype verifies that Product Kernel v0 can represent a full coding task without product runtime code.

File:

- `coding_task_sequence.jsonl`: one event per line.

Expected properties:

- Plan approval is represented by `plan.approval_requested` / `plan.approval_decided`, not `permission.requested`.
- Permission is used only for safety actions: command, file write, cloud model, package install, protected path, artifact export, or network.
- Patch proposal records base file hashes before apply.
- Large outputs/artifacts are references, not inline blobs.
- Eval event can attach metrics to the run.

Validation:

```bash
python3 scripts/validate_event_sequence.py docs/prototypes/event_log_replay/coding_task_sequence.jsonl
```

