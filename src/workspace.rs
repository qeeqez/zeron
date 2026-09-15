use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
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
    pub rename: Entity<InputState>,
    /// Chat id being renamed — stable across deletions, unlike a vec index.
    pub renaming: Option<u64>,
    /// Which surface owns the rename — only `Inline` mounts the row editor.
    pub rename_mode: RenameMode,
    /// Index into user messages for Cmd+Shift+Up/Down recall cycling.
    pub recall_ix: Option<usize>,
    /// Composer text stashed when a recall cycle starts — restored when the
    /// cycle steps past the newest message.
    pub recall_saved: Option<String>,
    pub chat_search: Entity<InputState>,
    /// In-chat find bar state (Cmd-F) — highlights + navigates matches
    /// without filtering the transcript (see `crate::chat_find`).
    pub find: crate::chat_find::FindBar,
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
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Resolve the project first — `enter` re-roots the process cwd so
        // the backend, git and file scans all agree on the opened folder.
        let project = crate::project::Project::launch();
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("Ask anything — @ to mention files, / for commands")
        });
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats"));

        cx.subscribe_in(&search, window, |_this, _s, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify()
            }
        })
        .detach();
        cx.subscribe_in(&composer, window, |this, _composer, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
            // `set_value` suppresses Change, so this only fires on real edits.
            InputEvent::Change => {
                this.recall_ix = None;
                this.recall_saved = None;
                cx.notify();
            },
            _ => {},
        })
        .detach();

        let palette = cx.new(|cx| CommandState::new(window, cx));
        let chat_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search in chat"));
        cx.subscribe_in(&chat_search, window, |this, _s, event: &InputEvent, _window, cx| match event {
            InputEvent::Change => {
                this.search_match_ix = 0;
                let count = this.filtered_count(cx);
                this.scroller.update(cx, |s, cx| s.reset(count, cx));
                cx.notify();
            },
            InputEvent::PressEnter { shift, .. } => this.jump_to_match(*shift, cx),
            _ => {},
        })
        .detach();
        let global_search = cx.new(|cx| CommandState::new(window, cx));
        let task_input = cx.new(|cx| InputState::new(window, cx).placeholder("New task…"));
        cx.subscribe_in(&task_input, window, |this, s, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let prompt = s.read(cx).value().to_string();
                s.update(cx, |s, cx| s.set_value("", window, cx));
                this.spawn_task_agent(prompt, cx);
            }
        })
        .detach();
        // Cmd+Q / QuitApp bypasses the window close gate — save drafts here.
        cx.on_app_quit(|this, cx| {
            this.chats[this.active].draft = this.composer.read(cx).value().to_string();
            this.save();
            async {}
        })
        .detach();

        let settings = crate::persist::load_settings();
        project.migrate_legacy_chats(settings.active_chat);
        let providers = settings.providers.clone();
        let selected_provider = crate::model_catalog::resolve_provider(&providers, &settings.selected_provider)
            .unwrap_or_default()
            .to_string();
        let model_catalog = crate::model_catalog::seed_catalog(&settings);
        let backend = providers
            .iter()
            .find(|p| p.id == selected_provider)
            .map_or_else(|| std::sync::Arc::new(crate::backend::SimBackend) as _, crate::backend::backend_for);
        // Built eagerly — creating it inside open_settings would re-enter the
        // workspace borrow (the click listener already holds it).
        let ws = cx.entity();
        let settings_panel = cx.new(|cx| crate::views::settings::SettingsPanel::new(ws.clone(), &settings, window, cx));
        let mut this = Self {
            chats: Vec::new(),
            active: 0,
            sidebar_collapsed: settings.sidebar_collapsed,
            sidebar_tab: crate::views::sidebar::SidebarTab::Chats,
            explorer: crate::views::explorer::ExplorerState::default(),
            sidebar_width: settings.sidebar_width.clamp(180.0, 480.0),
            agents: Vec::new(),
            next_agent_id: 0,
            next_chat_id: 0,
            send_queue: SendQueue::default(),
            changes_panel_open: false,
            activity_open: false,
            activity: crate::activity::ActivityFeed::load(project.dir()),
            resizing_sidebar: false,
            agents_panel_open: false,
            snapshots: crate::snapshots::SnapshotsState::default(),
            changes: Vec::new(),
            changes_generation: 0,
            composer,
            review: crate::review::Review::new(window, cx),
            git: crate::changes::ChangesGit::new(window, cx),
            search,
            scroller,
            model: providers.iter().find(|p| p.id == selected_provider).map_or_else(SharedString::default, |p| {
                crate::model_catalog::resolve_model(
                    model_catalog.get(&p.id).map_or(&[], Vec::as_slice),
                    &p.models,
                    &settings.selected_model,
                )
                .into()
            }),
            selected_provider,
            providers,
            model_catalog,
            task_input,
            mode: if ["Agent", "Plan", "Ask"].contains(&settings.mode.as_str()) {
                settings.mode.clone().into()
            } else {
                "Agent".into()
            },
            access: crate::backend::AccessMode::from_name(&settings.access),
            effort: None,
            default_model: settings.default_model.clone(),
            default_permissions: (!settings.default_permissions.is_empty())
                .then(|| crate::backend::AccessMode::from_name(&settings.default_permissions)),
            default_workspace: crate::worktree::WorkspaceMode::from_name(&settings.default_workspace),
            rename: cx.new(|cx| InputState::new(window, cx).placeholder("Chat title")),
            renaming: None,
            rename_mode: RenameMode::Inline,
            recall_ix: None,
            palette,
            global_search,
            chat_search,
            find: crate::chat_find::FindBar::new(crate::chat_find::new_find_input(window, cx)),
            chat_search_open: false,
            search_match_ix: 0,
            recall_saved: None,
            close_confirmed: std::cell::Cell::new(false),
            settings_open: false,
            settings_panel,
            shortcuts_open: false,
            notify_on_done: settings.notify_on_done,
            notify_sound: settings.notify_sound,
            backend,
            word_wrap: settings.word_wrap,
            preferred_editor: crate::open_in::PreferredEditor::from_name(&settings.preferred_editor),
            font_size: settings.font_size.clamp(crate::appearance::FONT_SIZE_MIN, crate::appearance::FONT_SIZE_MAX),
            font_family: settings.font_family.clone(),
            code_font_family: settings.code_font_family.clone(),
            code_font_size: settings.code_font_size.clamp(crate::appearance::FONT_SIZE_MIN, crate::appearance::FONT_SIZE_MAX),
            contrast: settings.contrast.clamp(crate::appearance::CONTRAST_MIN, crate::appearance::CONTRAST_MAX),
            sidebar_frosted: settings.sidebar_frosted,
            theme: settings.theme.clone(),
            theme_persisted: settings.theme.clone(),
            project_files: Vec::new(),
            project,
            sessions: Vec::new(),
            sessions_loading: false,
            voice: crate::voice::VoiceState::new(settings.voice_enabled, settings.voice_language.clone(), settings.voice_on_device),
            resume_open: false,
            auth: crate::auth::AuthBook::seeded(),
        };
        this.snapshots.retention_days = settings.snapshot_retention_days.unwrap_or(crate::snapshots::DEFAULT_RETENTION_DAYS);
        this.snapshots.cap_mb = settings.snapshot_cap_mb.unwrap_or(0);
        let loaded = crate::persist::load_chats(&this.project.chats_dir(), &mut this.next_chat_id, !crate::lifecycle::any_turn_running(cx));
        if loaded.is_empty() {
            this.new_chat(cx);
        } else {
            this.chats = loaded;
            this.active = this.project.load_state().active_chat.min(this.chats.len().saturating_sub(1));
            // The resumed thread's own provider/model/access replace the
            // settings selection — the picker shows the active thread.
            this.restore_thread_selection(cx);
        }
        this.apply_theme(window, cx);
        // "system" follows the OS — re-resolve when the appearance flips.
        window
            .observe_window_appearance(move |window, cx| ws.update(cx, |this, cx| this.apply_theme(window, cx)))
            .detach();
        this.start_background(cx);
        this.refresh_model_catalogs(cx);
        this.refresh_auth(cx);
        // Launch-time retention pass — prunes aged/over-cap snapshots.
        this.refresh_snapshots(cx);
        this
    }
}
