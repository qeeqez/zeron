//! The `Workspace` constructor — split from `workspace.rs` for the SLOC cap.
//! `for_project` binds a window to any project folder; the test-only
//! launch-project `new` lives in `workspace_tests.rs`. Input entities and
//! their subscriptions live in `workspace_inputs.rs`.

use crate::send_queue::SendQueue;
use crate::workspace::{RenameMode, Workspace};
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;

#[path = "workspace_inputs.rs"]
mod workspace_inputs;
use workspace_inputs::WorkspaceInputs;

impl Workspace {
    /// A workspace bound to `project` — its chats, @-mentions, git and
    /// backend turns all scope to that root. The process cwd stays at the
    /// launch project; turns carry the root via `TurnContext`.
    pub fn for_project(project: crate::project::Project, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = crate::persist::load_settings();
        let inputs = WorkspaceInputs::build(&settings.global_hotkey, settings.budget_alert_usd, window, cx);
        let project_state = project.load_state();
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
        // An entity, not a field: the search dialog reads it while the
        // workspace is mid-render, and the observe below re-renders on change.
        let search_filters = cx.new(|_| crate::global_search::SearchFilters::default());
        let mut this = Self {
            chats: Vec::new(),
            active: 0,
            secondary: None,
            sidebar_collapsed: settings.sidebar_collapsed,
            sidebar_tab: crate::views::sidebar::SidebarTab::Chats,
            explorer: crate::views::explorer::ExplorerState::default(),
            explorer_input: cx.new(|cx| InputState::new(window, cx).placeholder("Name")),
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
            plan_panel: crate::plan_panel::PlanPanel { open: settings.plan_panel_open },
            automations: crate::persist::load_automations(project.dir()),
            next_automation_id: 0,
            scheduled_panel_open: settings.scheduled_panel_open,
            schedule_prompt_input: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(2, 8)
                    .placeholder("Prompt to send on every run — e.g. \"Summarize new commits on main.\"")
            }),
            schedule_interval: crate::automations::AutomationInterval::H1,
            bookmarks_panel: crate::chat_msg::bookmarks_panel::BookmarksPanel { open: settings.bookmarks_panel_open },
            snapshots: crate::snapshots::SnapshotsState::default(),
            changes: Vec::new(),
            changes_generation: 0,
            changes_scope: crate::changes::ChangesScope { dir: project.root().to_path_buf(), base: None },
            composer: inputs.composer,
            review: crate::review::Review::new(window, cx),
            git: crate::changes::ChangesGit::new(window, cx),
            diff_mode: crate::changes_diff::DiffMode::from_name(&settings.diff_mode),
            search: inputs.search,
            sidebar_filters: crate::sidebar_filter::SidebarFilters::default(),
            sidebar_hits: Vec::new(),
            sidebar_hits_extra: 0,
            sidebar_search_gen: 0,
            scroller: inputs.scroller,
            secondary_scroller: cx.new(|cx| gpui_kit::component::message_scroller::MessageScrollerState::new(0, cx)),
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
            task_input: inputs.task_input,
            mode: if ["Agent", "Plan", "Ask"].contains(&settings.mode.as_str()) {
                settings.mode.clone().into()
            } else {
                "Agent".into()
            },
            access: crate::backend::AccessMode::from_name(&settings.access),
            // Assumed trusted — `open_workspace_window_for` downgrades an
            // untrusted folder via `restrict_untrusted` right after open.
            trusted: true,
            run_approved: false,
            apply_approved: false,
            approval_rules: project_state.approval_rules.clone(),
            effort: None,
            default_model: settings.default_model.clone(),
            default_permissions: (!settings.default_permissions.is_empty())
                .then(|| crate::backend::AccessMode::from_name(&settings.default_permissions)),
            default_workspace: crate::worktree::WorkspaceMode::from_name(&settings.default_workspace),
            rename: cx.new(|cx| InputState::new(window, cx).placeholder("Chat title")),
            renaming: None,
            rename_mode: RenameMode::Inline,
            folder_input: cx.new(|cx| InputState::new(window, cx).placeholder("Folder name")),
            prompt_input: cx.new(|cx| InputState::new(window, cx).placeholder("Prompt name")),
            template_input: cx.new(|cx| InputState::new(window, cx).placeholder("Template name")),
            split_input: cx.new(|cx| InputState::new(window, cx).placeholder("Message number")),
            chat_instructions_input: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(3, 12)
                    .placeholder("Instructions applied to this chat's turns — appended after the global and project instructions.")
            }),
            collapsed_folders: std::collections::HashSet::new(),
            folder_colors: project_state
                .folder_colors
                .iter()
                .filter_map(|(name, color)| crate::model::ChatColor::from_name(color).map(|c| (name.clone(), c)))
                .collect(),
            selected_chats: std::collections::HashSet::new(),
            recall_ix: None,
            palette: inputs.palette,
            global_search: inputs.global_search,
            search_filters: search_filters.clone(),
            file_palette: inputs.file_palette,
            apply_palette: cx.new(|cx| CommandState::new(window, cx)),
            recent_files: Vec::new(),
            chat_search: inputs.chat_search,
            find: crate::chat_find::FindBar::new(crate::chat_find::new_find_input(window, cx)),
            nav: None,
            nav_focus: cx.focus_handle(),
            pending_g: None,
            pill_anchor: None,
            chat_search_open: false,
            search_match_ix: 0,
            recall_saved: None,
            history_ix: None,
            draft_before_recall: String::new(),
            draft_save_ticks: None,
            editing: None,
            feedback: crate::feedback::FeedbackState::new(window, cx),
            close_confirmed: std::cell::Cell::new(false),
            settings_open: false,
            settings_panel,
            shortcuts_open: false,
            logs_open: false,
            logs_filter: crate::logs::LogFilter::All,
            usage_panel_open: settings.usage_panel_open,
            file_inspect: None,
            image_view: None,
            notify_on_done: settings.notify_on_done,
            notify_sound: settings.notify_sound,
            notify_background: settings.notify_background,
            backend,
            word_wrap: settings.word_wrap,
            show_timestamps: settings.show_timestamps,
            preferred_editor: crate::open_in::PreferredEditor::from_name(&settings.preferred_editor),
            font_size: settings.font_size.clamp(crate::appearance::FONT_SIZE_MIN, crate::appearance::FONT_SIZE_MAX),
            font_family: settings.font_family.clone(),
            code_font_family: settings.code_font_family.clone(),
            code_font_size: settings
                .code_font_size
                .clamp(crate::appearance::FONT_SIZE_MIN, crate::appearance::CODE_FONT_SIZE_MAX),
            contrast: settings.contrast.clamp(crate::appearance::CONTRAST_MIN, crate::appearance::CONTRAST_MAX),
            sidebar_frosted: settings.sidebar_frosted,
            theme: settings.theme.clone(),
            theme_persisted: settings.theme.clone(),
            project_files: Vec::new(),
            prompts: crate::persist::load_prompts(project.dir()),
            templates: crate::persist::load_templates(project.dir()),
            project,
            sessions: Vec::new(),
            sessions_loading: false,
            voice: crate::voice::VoiceState::new(settings.voice_enabled, settings.voice_language.clone(), settings.voice_on_device),
            instructions: settings.instructions.clone(),
            instructions_input: cx.new(|cx| {
                let mut input = TextareaState::new(window, cx)
                    .auto_grow(4, 16)
                    .placeholder("Instructions applied to every turn — e.g. \"Always write tests first.\"");
                input.set_value(settings.instructions.clone(), window, cx);
                input
            }),
            setup_script: project_state.setup_script.clone(),
            setup_script_input: cx.new(|cx| {
                let mut input = TextareaState::new(window, cx)
                    .auto_grow(2, 12)
                    .placeholder("e.g. \"ln -sf ../.env .env && npm install\"");
                input.set_value(project_state.setup_script.clone(), window, cx);
                input
            }),
            update: crate::update::UpdateState::restored(&settings),
            onboarding_dismissed: settings.onboarding_dismissed,
            detected_providers: None,
            detection_pending: false,
            resume_open: false,
            auth: crate::auth::AuthBook::seeded(),
            terminal: crate::views::terminal::TerminalPanel::new(settings.terminal_open, inputs.terminal_input, inputs.terminal_find_input),
            global_hotkey_enabled: settings.global_hotkey_enabled,
            global_hotkey: settings.global_hotkey.clone(),
            hotkey_input: inputs.hotkey_input,
            hotkey_error: None,
            budget_alert_usd: settings.budget_alert_usd,
            budget_cap_input: inputs.budget_cap_input,
            budget_input: cx.new(|cx| InputState::new(window, cx).placeholder("e.g. 5.00 — empty = global default")),
        };
        // Filter changes re-render the workspace — the dialog rebuilds its
        // Command (and re-runs `search`) on every render.
        cx.observe(&search_filters, |_, _, cx| cx.notify()).detach();
        this.snapshots.retention_days = settings.snapshot_retention_days.unwrap_or(crate::snapshots::DEFAULT_RETENTION_DAYS);
        // Loaded automations keep their ids — the counter resumes past the
        // highest so a new schedule never reuses one.
        this.next_automation_id = this.automations.iter().map(|a| a.id + 1).max().unwrap_or(0);
        this.git.ignore_ws = settings.diff_ignore_ws;
        this.snapshots.cap_mb = settings.snapshot_cap_mb.unwrap_or(0);
        let loaded = crate::persist::load_chats(&this.project.chats_dir(), &mut this.next_chat_id, !crate::lifecycle::any_turn_running(cx));
        if loaded.is_empty() {
            this.new_chat(cx);
        } else {
            this.chats = loaded;
            this.active = project_state.active_chat.min(this.chats.len().saturating_sub(1));
            // Restore the active chat's unsent draft into the composer —
            // `set_value` suppresses Change, so no stash/dirty flag trips.
            let draft = this.chats[this.active].draft.clone();
            this.composer.update(cx, |s, cx| s.set_value(draft, window, cx));
            // The resumed thread's own provider/model/access replace the
            // settings selection — the picker shows the active thread.
            this.restore_thread_selection(cx);
        }
        // Stale-worktree hygiene: drop orphaned `thread-*` checkouts no
        // chat owns (a deleted chat's cleanup never ran, a crash). Only
        // clean worktrees go — dirty ones stay for Settings → Project.
        // Skipped when another window already owns this project: its live
        // chats aren't all in `this.chats` yet (a worktree chat's file
        // lands after `git worktree add`), so pruning could eat a
        // just-created checkout.
        if crate::lifecycle::project_window(this.project.root(), cx).is_none() {
            crate::worktree::prune_orphans(this.project.root(), &this.chats);
        }
        this.apply_theme(window, cx);
        // "system" follows the OS — re-resolve when the appearance flips.
        window
            .observe_window_appearance(move |window, cx| ws.update(cx, |this, cx| this.apply_theme(window, cx)))
            .detach();
        this.start_background(cx);
        this.start_update_check(cx);
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
