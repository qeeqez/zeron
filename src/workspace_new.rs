//! The `Workspace` constructor — split from `workspace.rs` for the SLOC cap.
//! `for_project` binds a window to any project folder; the test-only
//! launch-project `new` lives in `workspace_tests.rs`.

use crate::send_queue::SendQueue;
use crate::workspace::{RenameMode, Workspace};
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::*;

impl Workspace {
    /// A workspace bound to `project` — its chats, @-mentions, git and
    /// backend turns all scope to that root. The process cwd stays at the
    /// launch project; turns carry the root via `TurnContext`.
    pub fn for_project(project: crate::project::Project, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let terminal_input = cx.new(|cx| InputState::new(window, cx).placeholder("Run a command…"));
        cx.subscribe_in(&terminal_input, window, |this, _s, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.terminal_send(window, cx);
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
            diff_mode: crate::changes_diff::DiffMode::from_name(&settings.diff_mode),
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
            run_approved: false,
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
            editing: None,
            feedback: crate::feedback::FeedbackState::new(window, cx),
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
            terminal: crate::views::terminal::TerminalPanel::new(settings.terminal_open, terminal_input),
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
        // A persisted-open terminal spawns its shell now rather than on
        // first toggle.
        if this.terminal.open {
            this.ensure_terminal(cx);
        }
        this
    }
}
