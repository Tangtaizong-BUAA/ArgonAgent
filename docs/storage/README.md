# Storage Schema Draft

`sqlite_schema_v0.sql` is a Phase 0 storage draft. It is not a production migration.

Key corrections captured:

- Plan approvals and permissions are separate tables.
- `permissions.request_type` cannot be `plan`.
- Event log is append-oriented with sequence and hash fields.
- Artifacts are separate from event payloads.
- Eval results keep fixture/model/profile metadata.

Validation:

```bash
sqlite3 :memory: < docs/storage/sqlite_schema_v0.sql
```

