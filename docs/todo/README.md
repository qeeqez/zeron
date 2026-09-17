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

Completed areas live in `docs/done/`: `agents.md`, `sidebar.md`, `chat-ui.md`, `composer.md`, `settings.md`, `chrome.md`, `gpui-shell.md`, `scaffold.md`, `persistence.md`, `backend.md`.

## Gaps vs Codex desktop (lane candidates)

- Global search + terminal find bar: no match-case / whole-word toggles (the chat find bar has them; `find_opts.rs::FindOpts` is shared-ready).
- Toast notifications have click-to-open only — no action buttons (blocked on gpui-kit `Notification` API support).
- No drag-to-tear-off for chats — "Open in New Window" exists, drag-detach into an existing window does not.
- Parallel-load test flakes: `filter_chips_narrow_live_results`, `role_chip_narrows_live_results`, `check_for_updates_action_reports_available` share process-wide env/static state — need per-test isolation.

## Done recently (from this list)

- ~~MCP transport~~ — `ProviderKind::Mcp` + `McpBackend` (`backend/mcp*.rs`).
- ~~Diff review inline comments~~ — `review.rs`, `model_review.rs`, `views/diff.rs`.
- ~~Multi-window~~ — `chat_window.rs` opens a chat in its own window.
- ~~Terminal shell integration~~ — `terminal_blocks.rs` command blocks + OSC 133 marks.
- ~~Update channel UI~~ — `update.rs` dialog, release notes, daily check, skip state.
- ~~Notification actions~~ — activity dropdown has per-entry dismiss + mark-all-read/clear-read footer.
- ~~Live worktree refresh~~ — `git_watch.rs` fingerprint poll updates Changes/Explorer.
- ~~Send terminal block output to chat~~ — block header send button into the composer.
- ~~Help menu~~ — shortcuts, release notes, report issue, reveal logs.

## Conventions

- One file ≤ 250 SLOC; split views into `src/views/`, state into `src/state/`.
- Conventional commits; commit per feature slice.
- Move a file from `docs/todo/` to `docs/done/` when the area is complete; keep a `## Done` log inside each file meanwhile.
