# Compatible Provider Adapter Spike

This prototype directory is intentionally docs-only for Phase 0.

Rules validated by schema:

- Compatible providers use `CompatibleProviderConfig`.
- Other providers can be `compatible` or `baseline`, never `native`.
- `actual_model_name` is the model string sent to the endpoint.
- `display_model_name` is the name shown in GUI/logs.
- `model_alias` is a user-facing shorthand.
- Request/response transforms normalize protocol, not native behavior.

Native DeepSeek/Qwen adapters must not import compatible provider prompt/parser/context policy.

