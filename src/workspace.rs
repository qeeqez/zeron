use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::*;

use crate::model::{Agent, Chat};
use crate::send_queue::SendQueue;

pub use crate::chat_ops::RenameMode;

pub struct Workspace {
    pub chats: Vec<Chat>,
    pub active: usize,
    pub sidebar_collapsed: bool,
    pub agents: Vec<Agent>,
    /// Which list the sidebar shows — chats or the file explorer.
    pub sidebar_tab: crate::views::sidebar::SidebarTab,
    /// File explorer state — expanded dirs, last-clicked file (see
    /// `crate::views::explorer::ExplorerState`).
    pub explorer: crate::views::explorer::ExplorerState,
    /// Monotonic id source for agents — survives `clear_finished_agents`.
    pub next_agent_id: u64,
    /// Monotonic id source for chats — survives deletions.
    pub next_chat_id: u64,
    /// Messages queued while a reply runs — per-workspace, so windows never
    /// share queues even when their chats reuse the same ids.
    pub send_queue: SendQueue,
    pub agents_panel_open: bool,
    pub changes_panel_open: bool,
    /// Snapshots panel + retention policy (see `crate::snapshots`).
    pub snapshots: crate::snapshots::SnapshotsState,
    /// The activity-center dropdown is open — see `crate::activity`.
    pub activity_open: bool,
    /// Recent turn/approval/error events behind the titlebar bell —
    /// persisted per project (see `crate::activity`).
    pub activity: crate::activity::ActivityFeed,
    /// Working-tree git changes shown in the Changes panel — refreshed on
    /// open and via the panel's refresh button.
    pub changes: Vec<crate::git::FileChange>,
    /// Bumped per `refresh_changes` request; a collection or row-diff result
    /// stamped with an older generation is discarded, so a slow earlier
    /// refresh can't overwrite a newer snapshot.
    pub(crate) changes_generation: u64,
    /// Pending diff review — comments collected from the Changes panel's
    /// diff lines plus the inline editor's state (see `crate::review`).
    pub review: crate::review::Review,
    /// Expanded diffs render unified or split — the panel header's
    /// segmented control; persisted as `Settings.diff_mode`.
    pub diff_mode: crate::changes_diff::DiffMode,
    /// Git actions for the Changes panel — branch header, commit input,
    /// busy flag and status note (see `crate::changes`).
    pub git: crate::changes::ChangesGit,
    pub sidebar_width: f32,
    pub resizing_sidebar: bool,
    pub composer: Entity<TextareaState>,
    pub search: Entity<InputState>,
    pub scroller: Entity<MessageScrollerState>,
    pub model: SharedString,
    /// Selected provider instance id — an entry in `providers`. The
    /// backend is rebuilt from it on change; persisted as
    /// `Settings.selected_provider`.
    pub selected_provider: String,
    /// Configured provider instances — the picker's provider list.
    pub providers: Vec<crate::providers::ProviderInstance>,
    /// Per-instance model lists for the picker — seeded from each
    /// backend's `models()`, overlaid by the cache, refreshed by
    /// `ProviderKindInfo::fetch`. Keyed by instance id.
    pub model_catalog: std::collections::HashMap<String, Vec<crate::model::ModelInfo>>,
    pub mode: SharedString,
    /// Filesystem access granted to the active thread's Agent-mode turns —
    /// Plan/Ask are always read-only. Passed to the backend via
    /// `TurnContext` at send time; each chat stamps its own on creation.
    pub access: crate::backend::AccessMode,
    /// "Always allow" on a command-run approval — later shell-block runs
    /// this session skip the prompt (see `crate::run_cmd`).
    pub run_approved: bool,
    /// Reasoning effort for the active thread's turns — `None` sends no
    /// override so the model's `default_effort` applies. Stamped per chat
    /// like `access`; the composer picker writes it via `set_effort`.
    pub effort: Option<String>,
    /// Provider+model new threads start on — `Settings.default_model`.
    /// Empty fields follow the current selection.
    pub default_model: crate::persist::DefaultModel,
    /// Access mode new threads start on; `None` = the current `access`.
    pub default_permissions: Option<crate::backend::AccessMode>,
    /// Where new threads run: project checkout or a per-thread worktree.
    pub default_workspace: crate::worktree::WorkspaceMode,
    pub palette: Entity<CommandState>,
    /// Cmd-Shift-F cross-chat search dialog's state (see `crate::global_search`).
    pub global_search: Entity<CommandState>,
    /// Cmd-P go-to-file picker's state (see `crate::file_palette`).
    pub file_palette: Entity<CommandState>,
    /// Files picked via Cmd-P, most recent first — ranks the picker.
    pub recent_files: Vec<SharedString>,
    pub rename: Entity<InputState>,
    /// Chat id being renamed — stable across deletions, unlike a vec index.
    pub renaming: Option<u64>,
    /// Which surface owns the rename — only `Inline` mounts the row editor.
    pub rename_mode: RenameMode,
    /// Shared text field for the folder dialogs — "Move to folder" seeds it
    /// empty, "Rename folder" with the current name.
    pub folder_input: Entity<InputState>,
    /// Folder names collapsed in the sidebar — runtime-only; folders are
    /// just `Chat::folder` values, so this set may name a folder that no
    /// longer exists.
    pub collapsed_folders: std::collections::HashSet<String>,
    /// Index into user messages for Cmd+Shift+Up/Down recall cycling.
    pub recall_ix: Option<usize>,
    /// Composer text stashed when a recall cycle starts — restored when the
    /// cycle steps past the newest message.
    pub recall_saved: Option<String>,
    /// A user message open in the inline editor — commit truncates the
    /// transcript after it and resends (see `crate::chat_edit`).
    pub editing: Option<crate::chat_edit::EditMessage>,
    /// Thumbs-down note editor state — which message it's open on plus the
    /// shared input (see `crate::feedback`).
    pub feedback: crate::feedback::FeedbackState,
    pub chat_search: Entity<InputState>,
    /// In-chat find bar state (Cmd-F) — highlights + navigates matches
    /// without filtering the transcript (see `crate::chat_find`).
    pub find: crate::chat_find::FindBar,
    /// Keyboard message navigation: the focused transcript row plus the
    /// focus handle the scroller wrapper tracks while navigating (see
    /// `crate::msg_nav`). `pending_g` is the `g` double-tap arm for `gg`.
    pub nav: Option<crate::msg_nav::MsgNav>,
    pub nav_focus: FocusHandle,
    pub pending_g: Option<std::time::Instant>,
    /// Agents-panel task input — Enter spawns a standalone backend turn.
    pub task_input: Entity<InputState>,
    pub chat_search_open: bool,
    /// Codex-style settings overlay — built eagerly in `new` so its inputs
    /// keep their state across opens.
    pub settings_open: bool,
    pub settings_panel: Entity<crate::views::settings::SettingsPanel>,
    /// Cmd-/ cheat sheet — a centered overlay rendered over the workspace.
    pub shortcuts_open: bool,
    pub search_match_ix: usize,
    /// One-shot bypass for the close prompt — `remove_window` re-fires
    /// `on_window_should_close`, so the confirmed path sets this to skip it.
    pub close_confirmed: std::cell::Cell<bool>,
    pub notify_on_done: bool,
    /// System bell when a turn finishes — independent of `notify_on_done`.
    pub notify_sound: bool,
    pub word_wrap: bool,
    /// Preferred editor for "Open in Editor" — `Settings.preferred_editor`.
    pub preferred_editor: crate::open_in::PreferredEditor,
    pub font_size: u8,
    /// Interface font family; empty = system default.
    pub font_family: String,
    /// Code font family; empty = theme default mono.
    pub code_font_family: String,
    pub code_font_size: u8,
    /// Chrome contrast percentage, clamped to [50, 200].
    pub contrast: u16,
    /// Sidebar translucency over the blurred window background.
    pub sidebar_frosted: bool,
    /// Appearance: "system" | "light" | "dark". "system" follows the OS.
    pub theme: String,
    /// The theme value this window last wrote to settings.json — lets
    /// `save_settings` tell "this window changed the theme" (persist it)
    /// apart from "another window changed it" (preserve the file's value).
    pub(crate) theme_persisted: String,
    /// Project-relative file paths for the @-mention picker.
    pub project_files: Vec<SharedString>,
    /// The folder this window runs against — chats, @-mentions, git and
    /// the backend all scope to it (see `crate::project`).
    pub project: crate::project::Project,
    /// Voice dictation settings + live take state (see `crate::voice`).
    pub voice: crate::voice::VoiceState,
    /// Bottom terminal panel: PTY session, input line, scroll state (see
    /// `crate::views::terminal`). `open` persists via `Settings`.
    pub terminal: crate::views::terminal::TerminalPanel,
    /// Global custom instructions — `Settings.instructions`, merged with
    /// the project's instructions file into every turn's `TurnContext`.
    pub instructions: String,
    /// The Custom Instructions settings field — lives on the workspace so
    /// typed text survives settings open/close like the other inputs.
    pub instructions_input: Entity<TextareaState>,
    /// Update-check state for the About row and toasts (see `crate::update`).
    pub update: crate::update::UpdateState,

    /// Past threads the backend can reopen — the sidebar's Resume section.
    /// Filled by `refresh_sessions`; empty until the first fetch lands.
    pub sessions: Vec<crate::backend::SessionInfo>,
    /// A `list_sessions` fetch is in flight — the section shows a loading
    /// row and won't spawn a second fetch.
    pub sessions_loading: bool,
    /// The Resume section is expanded in the sidebar.
    pub resume_open: bool,
    /// Per-instance sign-in state + in-flight login flows — see
    /// `crate::auth`. Seeded from the auth cache, probed by `refresh_auth`.
    pub(crate) auth: crate::auth::AuthBook,

    pub backend: std::sync::Arc<dyn crate::backend::AgentBackend>,
}
