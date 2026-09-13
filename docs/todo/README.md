# Rixl Code — Codex UI clone: current state & roadmap

Native macOS app in Rust on `gpui-kit` 0.6 (gpui-pre + gpui-component), Codex-desktop-style UI.

## Current state

- App shell: `gpui_kit::application()` + `Root`, `Workspace` view in `src/workspace.rs`.
- Tooling: nightly toolchain, clippy/rustfmt/sloc-guard (250 SLOC/file), lefthook hooks, nextest.
- Backend: `codex exec --json` via `CodexCliBackend`; `SimBackend` for offline dev.
- Persistence: chats + settings under `~/.rixl/rixlcode/`, atomic writes, retention cap.

## Feature areas (each file = one area, checkboxes = remaining work)

| File | Area |
|---|---|
| `agents.md` | Subagent/task panels, parallel runs, worktree lanes |
| `backend.md` | Real agent backend (replace simulation) |
| `persistence.md` | Chat history on disk, resume sessions |

Completed areas live in `docs/done/`: `sidebar.md`, `chat-ui.md`, `composer.md`, `settings.md`, `chrome.md`, `gpui-shell.md`, `scaffold.md`.

## Conventions

- One file ≤ 250 SLOC; split views into `src/views/`, state into `src/state/`.
- Conventional commits; commit per feature slice.
- Move a file from `docs/todo/` to `docs/done/` when the area is complete; keep a `## Done` log inside each file meanwhile.
