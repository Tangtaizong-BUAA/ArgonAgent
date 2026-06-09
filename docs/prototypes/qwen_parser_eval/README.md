# Qwen Parser and Executor Fixture Spike

`eval/fixtures/qwen/parser_golden.json` is the Phase 0 golden file for Qwen3.6-27B native behavior.

Rules:

- Qwen3.6-27B is the canonical target.
- Qwen2/Qwen2-7B cannot start native Qwen mode.
- Qwen parser/template capability must be verified.
- `qwen3` reasoning parser and `qwen3_coder` tool parser are capability gates where available.
- Generic OpenAI-compatible transport is not enough for Qwen-native tool use.
- Patch-sized edits still require read-before-write and stale-file validation.

Validation:

```bash
python3 -m json.tool eval/fixtures/qwen/parser_golden.json >/dev/null
```

