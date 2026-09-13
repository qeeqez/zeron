# Backend

Replace the in-process simulation with a real agent backend.

## Done

- [x] Define `AgentBackend` trait: send_message, events stream, cancel, list_models
- [x] Event model: TextDelta / ToolCallStart / ToolCallDelta / ToolCallEnd / Diff / Done / Error
- [x] Wire streaming events into chat state (replace timer simulation)
- [x] Local CLI transport: `codex exec --json` → JSONL → `AgentEvent`
- [x] Cancellation propagates to backend (Stop kills the child process)
- [x] Retry with backoff on transport errors (3 attempts, 400/800ms, only when nothing emitted)
- [x] HTTP transport — POST + NDJSON stream, bearer token from env var (`http_url`/`http_key_env` settings)
- [x] Backend selector setting — `backend: "codex-cli"|"sim"|"http"`, migrated from `use_codex_cli`

## Todo

- [ ] MCP transport — needs a JSON-RPC client; no crate pinned yet
- [ ] Settings UI for `http_url`/`http_key_env` (settings.json only today)
