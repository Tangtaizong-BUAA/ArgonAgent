# Patch Invariant Fixtures

These fixtures exercise Product Kernel read-before-write invariants:

- P-01 exact old-string replacement passes.
- P-02 ambiguous match fails.
- P-03 stale base hash fails.
- P-04 explicit generated-file creation passes.
- P-05 protected path fails.

Run:

```bash
python3 scripts/prototype_patch_validator.py eval/fixtures/patch
```

