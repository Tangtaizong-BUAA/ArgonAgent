# Desktop GUI

React UI is now wired to two runtime transports:

- `tauri` (default for desktop app): Rust `RuntimeFacade` direct invoke + event stream.
- `http` (fallback): existing local HTTP bridge for browser-only debug.

## Run (Tauri native path)

1. Install frontend deps once:

```bash
cd /Users/gongyuxuan/Documents/deep-code/desktop
npm install
```

2. Ensure runtime environment variables for your model provider are set (for example `DEEPSEEK_API_KEY` / `QWEN_API_KEY`, and `QWEN_BASE_URL` when using Qwen).

3. Start desktop app:

```bash
cd /Users/gongyuxuan/Documents/deep-code/desktop
npm run tauri:dev
```

## Browser debug fallback (HTTP bridge)

1. Start local API bridge:

```bash
cd /Users/gongyuxuan/Documents/deep-code
cargo run -p researchcode-cli -- local-api-server 8765
```

2. Start frontend:

```bash
cd /Users/gongyuxuan/Documents/deep-code/desktop
npm run dev
```

Then open `http://127.0.0.1:5173`.

For automated browser smoke tests, `gui_three_round_smoke.mjs` injects
`window.__ARGON_RUNTIME_BOOTSTRAP__` so the Vite page talks to this Rust bridge
without Electron IPC.

## Runtime Incident Verification

Calibrate the incident detector against the pre-doc39 production fixture:

```bash
cd /Users/gongyuxuan/Documents/deep-code/desktop
npm run gui:incident-fixture
```

Run the real GUI + Rust runtime approval regression probe:

```bash
cd /Users/gongyuxuan/Documents/deep-code/desktop
npm run gui:incident-live
```

The live probe creates a temporary workspace under `desktop/.gui-smoke-runs/`,
drives the browser with Playwright, approves `file.write` and `shell.command`,
then fails if the event log shows repeated un-escalated tool contract errors,
`PermissionRequired("shell.command")` as a tool result, stale
`no pending permission` errors, or missing continuation prompt/tool hashes.
