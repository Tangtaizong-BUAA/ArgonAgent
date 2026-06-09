# Tauri Host

`src-tauri` now hosts the desktop runtime bridge:

- `RuntimeFacade` is managed directly in Rust (no Python local API in the main path).
- Frontend calls `runtime_*` commands via Tauri `invoke`.
- Runtime JSONL events are emitted incrementally to frontend via `runtime://event`.
- Existing HTTP bridge remains as fallback transport for browser-only debugging.
