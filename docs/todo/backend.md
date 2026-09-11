# Backend

Replace the in-process simulation with a real agent backend.

## Done

- [ ] (pending)

## Todo

- [x] Define `AgentBackend` trait: send_message, events stream, cancel, list_models
- [x] Event model: TextDelta / ToolCallStart / ToolCallDelta / ToolCallEnd / Diff / Done / Error
- [ ] Wire streaming events into chat state (replace timer simulation)
- [ ] Pluggable transports: local CLI process, MCP, HTTP
- [ ] Auth/config for provider keys
- [ ] Cancellation propagates to backend
- [ ] Retry with backoff on transport errors
