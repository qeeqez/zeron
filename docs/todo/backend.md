# Backend

Replace the in-process simulation with a real agent backend.

## Done

- [x] Define `AgentBackend` trait: send_message, events stream, cancel, list_models
- [x] Event model: TextDelta / ToolCallStart / ToolCallDelta / ToolCallEnd / Diff / Done / Error
- [x] Wire streaming events into chat state (replace timer simulation)
- [x] Local CLI transport: `codex exec --json` → JSONL → `AgentEvent`
- [x] Cancellation propagates to backend (Stop kills the child process)
- [x] Retry with backoff on transport errors (3 attempts, 400/800ms, only when nothing emitted)

## Todo

- [ ] Pluggable transports: MCP, HTTP
- [ ] Auth/config for provider keys
