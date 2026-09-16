# Rixl Code — Codex UI clone: current state & roadmap

Native macOS app in Rust on `gpui-kit` 0.6 (gpui-pre + gpui-component), Codex-desktop-style UI.

## Current state

- App shell: `gpui_kit::application()` + `Root`, `Workspace` view in `src/workspace.rs`; app menus, dock menu, global summon hotkey.
- Tooling: nightly toolchain, clippy/rustfmt/sloc-guard (250 SLOC/file), lefthook hooks, nextest.
- Backends: `codex app-server` (default), Claude CLI, ACP subprocess, NDJSON HTTP, Ollama local, `SimBackend` for offline dev — multi-instance providers with per-instance env vars, auth flows, and model lists.
- Persistence: chats + settings under `~/.rixl/rixlcode/`, atomic writes, retention cap; per-chat checkpoints (git snapshot or dir copy) power Snapshots restore.
- Sidebar: pinned/date-grouped chat rows, search, archive, color tags, inline rename, Resume section (backend session list), Plan/Agents/Changes/Snapshots nav rows.
- Chat: streaming text/tool/diff/plan/approval messages, tool-call grouping, message nav (j/k/gg/G), edit+resend, reply versioning pager, split-at-message, bookmarks, per-chat instructions, jump-to-latest pill, chat find + global search with provider/model/date filters.
- Composer: @-mention files, image attach, dictation, send queue, steer-into-turn, model/permission/workspace pickers, usage popover.
- Panels: Agents (live subagent cards, stop-all), Changes (diff review, hunk staging, commits/PR/stash/conflicts views), Snapshots (checkpoint restore), Plan (step list), Explorer (file tree + git badges + context menu), Terminal (multi-tab PTY + find), Logs.
- Overlays: command palette (Cmd-K), go-to-file (Cmd-P), shortcuts sheet (Cmd-/), usage dashboard, onboarding card, trust dialog for untrusted folders.
- Settings sheet: General, Instructions, Appearance, Profile, Project (setup script + worktrees), Providers (wizard + per-instance detail), Shortcuts, Voice, MCP servers.
- Git surface: status badges, hunk-level staging, commit/branch/stash rows, PR status, file history + blame, conflict list.

## Feature areas (each file = one area, checkboxes = remaining work)

| File | Area |
|---|---|
| `agents.md` | Subagent/task panels, parallel runs, worktree lanes |
| `backend.md` | Real agent backend (replace simulation) |

Completed areas live in `docs/done/`: `sidebar.md`, `chat-ui.md`, `composer.md`, `settings.md`, `chrome.md`, `gpui-shell.md`, `scaffold.md`, `persistence.md`.

## Gaps vs Codex desktop (lane candidates)

- MCP transport — settings UI exists; no JSON-RPC client pinned yet (`backend.md`).
- Diff review: no inline comment-on-line flow; Changes is read/stage only.
- No multi-window chat drag-out / tear-off; windows are independent workspaces.
- Terminal: no shell-integration decorations (command blocks, exit-code marks).
- Notifications: activity bell exists; no per-notification action buttons (open chat, mark read).
- No update channel UI beyond "Check for Updates" menu item.

## Conventions

- One file ≤ 250 SLOC; split views into `src/views/`, state into `src/state/`.
- Conventional commits; commit per feature slice.
- Move a file from `docs/todo/` to `docs/done/` when the area is complete; keep a `## Done` log inside each file meanwhile.
