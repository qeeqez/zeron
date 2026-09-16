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
    /// The explorer tree's inline name input — shared by new-file,
    /// new-folder and rename edits (`explorer.editing` says which).
    pub explorer_input: Entity<InputState>,
    /// Monotonic id source for agents — survives `clear_finished_agents`.
    pub next_agent_id: u64,
    /// Monotonic id source for chats — survives deletions.
    pub next_chat_id: u64,
    /// Messages queued while a reply runs — per-workspace, so windows never
    /// share queues even when their chats reuse the same ids.
    pub send_queue: SendQueue,
    pub agents_panel_open: bool,
    pub changes_panel_open: bool,
    /// Plan panel state — the active chat's checklist side panel (see
    /// `crate::plan_panel`). `open` persists via `Settings`.
    pub plan_panel: crate::plan_panel::PlanPanel,
    /// Scheduled prompts — this project's automations, persisted as
    /// `automations.json` (see `crate::automations`).
    pub automations: Vec<crate::automations::Automation>,
    /// Monotonic id source for automations — survives deletions.
    pub next_automation_id: u64,
    /// Scheduled panel open/closed — persisted as
    /// `Settings.scheduled_panel_open`.
    pub scheduled_panel_open: bool,
    /// Shared multiline field for the "Schedule prompt…" dialog — seeded
    /// with the chat's draft on open.
    pub schedule_prompt_input: Entity<TextareaState>,
    /// The interval chip last picked in the schedule dialog — reset to
    /// `H1` on each open.
    pub schedule_interval: crate::automations::AutomationInterval,
    /// Bookmarks panel state — every loaded chat's starred messages in one
    /// side panel (see `crate::chat_msg::bookmarks_panel`). `open` persists via
    /// `Settings`.
    pub bookmarks_panel: crate::chat_msg::bookmarks_panel::BookmarksPanel,
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
    /// Sidebar filter chips under the chat search — ANDed with the title
    /// query (see `crate::sidebar_filter`). Runtime-only, never persisted.
    pub sidebar_filters: crate::sidebar_filter::SidebarFilters,
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
    /// Whether the project folder is trusted — untrusted folders run
    /// restricted: `access` is forced to `Supervised` (read-only, every
    /// action asks) until the user trusts the folder (see `crate::trust`).
    /// `for_project` assumes trusted; `lifecycle::open_workspace_window_for`
    /// downgrades untrusted folders at open.
    pub trusted: bool,
    /// "Always allow" on a command-run approval — later shell-block runs
    /// this session skip the prompt (see `crate::run_cmd`).
    pub run_approved: bool,
    /// "Always allow" on an apply-code-block approval — later applies this
    /// session skip the prompt (see `crate::apply_code`).
    pub apply_approved: bool,
    /// Durable "always allow" grants — `ProjectState.approval_rules`,
    /// loaded at open and persisted on every change (see
    /// `crate::approval_ops`).
    pub approval_rules: Vec<crate::backend::ApprovalRule>,
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
    /// The global-search dialog's filter row — an entity so the dialog can
    /// read it while the workspace is mid-render; session-scoped, never
    /// persisted (see `crate::global_search::SearchFilters`).
    pub search_filters: Entity<crate::global_search::SearchFilters>,
    /// Cmd-P go-to-file picker's state (see `crate::file_palette`).
    pub file_palette: Entity<CommandState>,
    /// Apply-code-block picker's state (see `crate::apply_code`).
    pub apply_palette: Entity<CommandState>,
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
    /// Shared text field for the saved-prompt dialogs — "Save prompt" seeds
    /// it empty, "Rename prompt" with the current name.
    pub prompt_input: Entity<InputState>,
    /// Shared text field for the "Split chat…" dialog — the 1-based message
    /// number the split starts the new chat at.
    pub split_input: Entity<InputState>,
    /// Shared multiline field for the per-chat "Custom instructions…"
    /// dialog — seeded with the chat's current text on open.
    pub chat_instructions_input: Entity<TextareaState>,
    /// Folder names collapsed in the sidebar — runtime-only; folders are
    /// just `Chat::folder` values, so this set may name a folder that no
    /// longer exists.
    pub collapsed_folders: std::collections::HashSet<String>,
    /// Index into user messages for Cmd+Shift+Up/Down recall cycling.
    pub recall_ix: Option<usize>,
    /// Composer text stashed when a recall cycle starts — restored when the
    /// cycle steps past the newest message.
    pub recall_saved: Option<String>,
    /// Index into `Chat::prompt_history` for Up/Down recall — `Some` while
    /// a history session is live (see `crate::send::composer_history`).
    pub history_ix: Option<usize>,
    /// Composer text stashed when a history session starts — restored when
    /// the session steps past either end. Runtime-only, not persisted.
    pub draft_before_recall: String,
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
    /// Visible-row count when the transcript left the tail — the "N new"
    /// count on the jump-to-latest pill (see `crate::chat_search`).
    pub pill_anchor: Option<usize>,
    /// Named reusable prompts — the composer ★ popover lists them and
    /// `/save` adds to them; persisted per project as `prompts.json`.
    pub prompts: crate::prompts::PromptStore,
    /// Agents-panel task input — Enter spawns a standalone backend turn.
    pub task_input: Entity<InputState>,
    pub chat_search_open: bool,
    /// Codex-style settings overlay — built eagerly in `new` so its inputs
    /// keep their state across opens.
    pub settings_open: bool,
    pub settings_panel: Entity<crate::views::settings::SettingsPanel>,
    /// Cmd-/ cheat sheet — a centered overlay rendered over the workspace.
    pub shortcuts_open: bool,
    /// View Logs overlay (Cmd-Shift-L) — a centered panel over the
    /// workspace listing the captured log buffer (see `crate::logs`).
    pub logs_open: bool,
    /// The logs panel's minimum-level filter (see `crate::logs::LogFilter`).
    pub logs_filter: crate::logs::LogFilter,
    /// Usage dashboard overlay — a centered panel aggregating tokens and
    /// cost across every chat (see `crate::views::usage_dashboard`).
    pub usage_dashboard_open: bool,
    /// File-inspect overlay — the open "File History"/"Blame" panel for one
    /// project file; `None` is closed (see `crate::views::file_inspect`).
    pub file_inspect: Option<crate::views::file_inspect::FileInspect>,
    /// Image lightbox — the open image path while the full-size overlay is
    /// up; `None` is closed (see `crate::image_view`).
    pub image_view: Option<SharedString>,
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
    /// Per-project setup script run inside each new worktree —
    /// `ProjectState.setup_script` (see `crate::setup_script`).
    pub setup_script: String,
    /// The Project section's setup-script field — same survive-open/close
    /// rationale as `instructions_input`.
    pub setup_script_input: Entity<TextareaState>,
    /// Update-check state for the About row and toasts (see `crate::update`).
    pub update: crate::update::UpdateState,
    /// First-run onboarding card dismissed via Skip — persisted as
    /// `Settings.onboarding_dismissed`; a configured provider hides the
    /// card regardless of this flag.
    pub onboarding_dismissed: bool,
    /// Provider kinds the last detection scan found installed — `None`
    /// until the first scan lands. Runtime only; the onboarding card and
    /// the provider wizard read it (see `crate::provider_detect`).
    pub detected_providers: Option<Vec<crate::providers::ProviderKind>>,
    /// A detection scan is in flight on the background executor.
    pub detection_pending: bool,
    /// System-wide summon hotkey toggle — `Settings.global_hotkey_enabled`;
    /// the monitor itself lives in `crate::app_setup::global_hotkey`.
    pub global_hotkey_enabled: bool,
    /// The summon chord in gpui keystroke syntax — `Settings.global_hotkey`.
    pub global_hotkey: String,
    /// The General section's hotkey field — workspace-owned so typed text
    /// survives settings open/close like the other inputs.
    pub hotkey_input: Entity<InputState>,
    /// Validation error from the last hotkey commit; shown under the field.
    pub hotkey_error: Option<String>,
    /// Global default spend cap in USD — `Settings.budget_alert_usd`; a
    /// chat's own `Chat::budget_alert_usd` overrides it. `None` = no cap.
    pub budget_alert_usd: Option<f64>,
    /// The General section's budget-cap field — workspace-owned so typed
    /// text survives settings open/close like the other inputs.
    pub budget_cap_input: Entity<InputState>,
    /// Shared text field for the per-chat "Budget alert…" dialog — seeded
    /// with the chat's current override on open.
    pub budget_input: Entity<InputState>,

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

impl Workspace {
    /// Session-wide usage folded across this window's chats — the usage
    /// popover's bottom row. A chat's cost is priced on its own model,
    /// falling back to the current selection for legacy chats (empty
    /// `model`); chats on unknown models contribute tokens only and mark
    /// the cost a lower bound.
    pub fn session_usage(&self) -> crate::usage::SessionUsage {
        let mut s = crate::usage::SessionUsage::default();
        for chat in &self.chats {
            s.total += chat.usage.total;
            if chat.usage.total == 0 {
                continue;
            }
            let model = if chat.model.is_empty() { self.model.as_ref() } else { chat.model.as_str() };
            match chat.usage.cost(model) {
                Some(c) => s.cost += c,
                None => s.cost_partial = true,
            }
        }
        s
    }
}
