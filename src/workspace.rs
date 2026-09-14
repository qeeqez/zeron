use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::*;

use crate::model::{Agent, Chat};
use crate::send_queue::SendQueue;

/// How an in-flight rename is driven: the sidebar row's inline editor, or
/// the rename dialog. The row only mounts its editor for `Inline` — a
/// dialog rename shares `Workspace::rename`, so without the split the row
/// would mount an editor whose outside-click commits behind the dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenameMode {
    Inline,
    Dialog,
}

pub struct Workspace {
    pub chats: Vec<Chat>,
    pub active: usize,
    pub sidebar_collapsed: bool,
    pub agents: Vec<Agent>,
    /// Monotonic id source for agents — survives `clear_finished_agents`.
    pub next_agent_id: u64,
    /// Monotonic id source for chats — survives deletions.
    pub next_chat_id: u64,
    /// Messages queued while a reply runs — per-workspace, so windows never
    /// share queues even when their chats reuse the same ids.
    pub send_queue: SendQueue,
    pub agents_panel_open: bool,
    pub changes_panel_open: bool,
    /// Working-tree git changes shown in the Changes panel — refreshed on
    /// open and via the panel's refresh button.
    pub changes: Vec<crate::git::FileChange>,
    /// Bumped per `refresh_changes` request; a collection or row-diff result
    /// stamped with an older generation is discarded, so a slow earlier
    /// refresh can't overwrite a newer snapshot.
    pub(crate) changes_generation: u64,
    pub sidebar_width: f32,
    pub resizing_sidebar: bool,
    pub composer: Entity<TextareaState>,
    pub search: Entity<InputState>,
    pub scroller: Entity<MessageScrollerState>,
    pub model: SharedString,
    pub mode: SharedString,
    /// Filesystem access granted to Agent-mode turns — Plan/Ask are always
    /// read-only. Published to `crate::backend` on change so the backend
    /// `send` signature (called from files outside this lane) stays stable.
    pub access: crate::backend::AccessMode,
    pub palette: Entity<CommandState>,
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
    /// Agents-panel task input — Enter spawns a standalone backend turn.
    pub task_input: Entity<InputState>,
    pub chat_search_open: bool,
    /// Codex-style settings overlay — built eagerly in `new` so its inputs
    /// keep their state across opens.
    pub settings_open: bool,
    pub settings_panel: Entity<crate::views::settings::SettingsPanel>,
    pub search_match_ix: usize,
    /// One-shot bypass for the close prompt — `remove_window` re-fires
    /// `on_window_should_close`, so the confirmed path sets this to skip it.
    pub close_confirmed: std::cell::Cell<bool>,
    pub notify_on_done: bool,
    pub word_wrap: bool,
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

    pub backend: std::sync::Arc<dyn crate::backend::AgentBackend>,
    /// HTTP transport config — kept on the workspace so `toggle_backend`
    /// can rebuild `HttpBackend` without re-reading settings.json.
    pub http_url: String,
    pub http_key_env: String,
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
        let rename = cx.new(|cx| InputState::new(window, cx).placeholder("Chat title"));
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
        // Built eagerly — creating it inside open_settings would re-enter the
        // workspace borrow (the click listener already holds it).
        let ws = cx.entity();
        let settings_panel = cx.new(|cx| crate::views::settings::SettingsPanel::new(ws.clone(), &settings, window, cx));
        let mut this = Self {
            chats: Vec::new(),
            active: 0,
            sidebar_collapsed: settings.sidebar_collapsed,
            sidebar_width: settings.sidebar_width.clamp(180.0, 480.0),
            agents: Vec::new(),
            next_agent_id: 0,
            next_chat_id: 0,
            send_queue: SendQueue::default(),
            resizing_sidebar: false,
            agents_panel_open: false,
            changes_panel_open: false,
            changes: Vec::new(),
            changes_generation: 0,
            composer,
            search,
            scroller,
            model: if crate::model::MODELS.contains(&settings.model.as_str()) {
                settings.model.clone().into()
            } else {
                "default".into()
            },
            task_input,
            mode: if ["Agent", "Plan", "Ask"].contains(&settings.mode.as_str()) {
                settings.mode.clone().into()
            } else {
                "Agent".into()
            },
            access: crate::backend::AccessMode::from_name(&settings.access),
            rename,
            palette,
            renaming: None,
            rename_mode: RenameMode::Inline,
            recall_ix: None,
            recall_saved: None,
            chat_search,
            chat_search_open: false,
            search_match_ix: 0,
            close_confirmed: std::cell::Cell::new(false),
            settings_open: false,
            settings_panel,
            notify_on_done: settings.notify_on_done,
            word_wrap: settings.word_wrap,
            backend: crate::backend::make_backend(&settings),
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
            http_url: settings.http_url.clone(),
            http_key_env: settings.http_key_env.clone(),
        };
        crate::backend::set_access_mode(this.access);
        let loaded = crate::persist::load_chats(&this.project.chats_dir(), &mut this.next_chat_id, !crate::lifecycle::any_turn_running(cx));
        if loaded.is_empty() {
            this.new_chat(cx);
        } else {
            this.chats = loaded;
            this.active = this.project.load_state().active_chat.min(this.chats.len().saturating_sub(1));
        }
        this.apply_theme(window, cx);
        // "system" follows the OS — re-resolve when the appearance flips.
        window.observe_window_appearance(move |window, cx| ws.update(cx, |this, cx| this.apply_theme(window, cx))).detach();
        this.start_background(cx);
        this
    }

    pub(crate) fn save(&mut self) {
        // Retention: drop oldest non-pinned chats beyond the cap. Storage
        // order is oldest-first, so retain() hits the oldest first.
        const MAX_CHATS: usize = 50;
        if self.chats.len() > MAX_CHATS {
            let dropped_before_active = self.retention_drops_before_active(MAX_CHATS);
            let mut drop_left = self.chats.len() - MAX_CHATS;
            self.chats.retain(|c| {
                let drop = drop_left > 0 && !c.pinned;
                drop_left -= usize::from(drop);
                !drop
            });
            self.active = self.active.saturating_sub(dropped_before_active).min(self.chats.len().saturating_sub(1));
        }
        crate::persist::save_chats(&self.project.chats_dir(), &self.chats);
        self.project.save_state(&crate::project::ProjectState { active_chat: self.active });
    }

    /// How many chats `retain` will drop before `self.active` — the first
    /// `len - max` non-pinned chats go, so count those under the index.
    fn retention_drops_before_active(&self, max: usize) -> usize {
        let mut drop_left = self.chats.len() - max;
        let mut dropped = 0usize;
        for (ix, c) in self.chats.iter().enumerate() {
            if drop_left > 0 && !c.pinned {
                drop_left -= 1;
                dropped += usize::from(ix < self.active);
            }
        }
        dropped
    }

    /// Change the Agent-mode access level, publish it to the backend, and
    /// persist it. Called from the settings picker.
    pub fn set_access(&mut self, access: crate::backend::AccessMode, cx: &mut Context<Self>) {
        self.access = access;
        crate::backend::set_access_mode(access);
        self.save_settings();
        cx.notify();
    }

    /// Toggle the Changes panel; opening refreshes the change list so the
    /// first render never shows stale rows.
    pub fn toggle_changes_panel(&mut self, cx: &mut Context<Self>) {
        self.changes_panel_open = !self.changes_panel_open;
        if self.changes_panel_open {
            self.refresh_changes(cx);
        }
        cx.notify();
    }
}
