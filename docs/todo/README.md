# Rixl Code — Codex UI clone: current state & roadmap

Native macOS app in Rust on `gpui-kit` 0.6 (gpui-pre + gpui-component), Codex-desktop-style UI.

## Current state

- App shell: `gpui_kit::application()` + `Root`, counter demo in `src/main.rs`.
- Tooling: nightly toolchain, clippy/rustfmt/sloc-guard (250 SLOC/file), lefthook hooks, nextest.
- No real agent backend — all chat data is simulated in-process.

## Feature areas (each file = one area, checkboxes = remaining work)

| File | Area |
|---|---|
| `sidebar.md` | Chat history sidebar: sections, search, context menus, collapse |
| `chat-ui.md` | Message list, tool-call cards, diffs, streaming, markdown |
| `composer.md` | Input box, attachments, model/mode pickers, slash commands |
| `agents.md` | Subagent/task panels, parallel runs, worktree lanes |
| `backend.md` | Real agent backend (replace simulation) |
| `persistence.md` | Chat history on disk, resume sessions |
| `settings.md` | Settings UI, theme switch, config |
| `chrome.md` | Title bar, status bar, notifications, keyboard shortcuts |

## Conventions

- One file ≤ 250 SLOC; split views into `src/views/`, state into `src/state/`.
- Conventional commits; commit per feature slice.
- Move a file from `docs/todo/` to `docs/done/` when the area is complete; keep a `## Done` log inside each file meanwhile.
