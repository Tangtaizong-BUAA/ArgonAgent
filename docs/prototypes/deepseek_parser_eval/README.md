# DeepSeek Parser Eval Fixture Spike

`eval/fixtures/deepseek/parser_golden.json` is the Phase 0 golden file for DeepSeek native parser behavior.

Rules:

- Native provider tool calls have priority.
- DSML/XML/text fallback is compatibility, not primary path.
- JSON argument repair is logged.
- Unknown/wrong tool names must deny or retry.
- Low-confidence repaired args must not execute.
- Reasoning content must pass sanitizer before persistence or replay.

Validation:

```bash
python3 -m json.tool eval/fixtures/deepseek/parser_golden.json >/dev/null
```

