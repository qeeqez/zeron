use crate::views::settings_nav::Section;
use crate::views::settings_provider_env::EnvRow;
use crate::views::settings_provider_wizard::ProviderWizard;
use crate::views::settings_providers::ProviderInputs;
use crate::workspace::Workspace;
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::select::{SearchableVec, SelectEvent, SelectState};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::collections::HashMap;

use crate::backend::AccessMode;
use crate::views::settings_general::{access_mode_from_label, workspace_mode_from_label, workspace_mode_label};
use crate::worktree::WorkspaceMode;

/// Codex-style settings screen: a left nav rail over a content pane, rendered
/// as a full-window overlay from `Workspace::render` (sheets/dialogs can't
/// host a two-pane layout and weren't mounted anyway).
pub struct SettingsPanel {
    /// Weak on purpose: `Workspace` owns this panel, so a strong handle here
    /// (and in the subscriptions below) would cycle and leak the whole
    /// workspace — chats, running subprocesses and all — on window close.
    pub(crate) ws: WeakEntity<Workspace>,
    pub(crate) section: Section,
    pub(crate) search: Entity<InputState>,
    /// Per-provider-instance detail inputs (name/command/key_env), keyed by
    /// instance id — created lazily by `sync_provider_inputs` on render.
    pub(crate) provider_inputs: HashMap<String, ProviderInputs>,
    /// Per-instance Variables inputs (key/value rows), keyed by instance
    /// id — created lazily by `sync_provider_env_inputs` on render.
    pub(crate) provider_env_inputs: HashMap<String, Vec<EnvRow>>,
    /// The instance the Providers detail panel shows.
    pub(crate) provider_selection: Option<String>,
    /// Per-instance "Test connection" probe outcomes, keyed by instance id
    /// — runtime only, never persisted.
    pub(crate) test_state: HashMap<String, crate::views::settings_provider_test::TestState>,
    /// Per-instance health probe results (status dot + reason), keyed by
    /// instance id — runtime only, never persisted.
    pub(crate) provider_health: HashMap<String, crate::providers::provider_detect::ProviderHealth>,
    /// A health pass is in flight on the background executor.
    pub(crate) health_pending: bool,
    /// In-flight "Add provider" wizard state — `None` when closed.
    pub(crate) provider_wizard: Option<ProviderWizard>,
    /// Interface font family picker — `SearchableVec<String>` delegate over
    /// the installed font names (a plain `Vec` delegate never filters).
    pub(crate) font_select: Entity<SelectState<SearchableVec<String>>>,
    /// Code (mono) font family picker.
    pub(crate) code_font_select: Entity<SelectState<SearchableVec<String>>>,
    /// Contrast slider, 50–200%.
    pub(crate) contrast_slider: Entity<SliderState>,
    /// Default permissions for new threads — `AccessMode::ALL` labels;
    /// empty selection = follow the current access.
    pub(crate) permissions_select: Entity<SelectState<Vec<String>>>,
    /// Default workspace for new threads — `WorkspaceMode::ALL` labels.
    pub(crate) workspace_select: Entity<SelectState<Vec<String>>>,
    /// Preferred editor for "Open in Editor" — `PreferredEditor::ALL` labels.
    pub(crate) editor_select: Entity<SelectState<Vec<String>>>,
    /// Configured MCP servers — the MCP Servers section's list; persisted
    /// to settings.json and mirrored into `~/.codex/config.toml`.
    pub(crate) mcp_servers: Vec<crate::mcp::McpServer>,
    /// Live per-server status from codex `mcpServerStatus/list`, keyed by
    /// server name — empty until a fetch lands.
    pub(crate) mcp_status: HashMap<String, crate::mcp::McpStatus>,
    /// A status fetch is in flight — the section shows "checking…".
    pub(crate) mcp_status_loading: bool,
    /// The add-server form is open.
    pub(crate) mcp_adding: bool,
    /// Add-form validation error shown under the inputs.
    pub(crate) mcp_error: Option<String>,
    /// The add form's inputs — created once so typed text survives renders.
    pub(crate) mcp_inputs: crate::views::settings_mcp::McpInputs,
    /// Dictation-language picker — `VOICE_LANGUAGES` labels; Confirm maps
    /// back to the locale id (empty = system default).
    pub(crate) voice_language_select: Entity<SelectState<Vec<String>>>,
    /// Scroll state of the settings content pane — the search anchor
    /// targets it so the first matching row scrolls into view.
    pub(crate) content_scroll: ScrollHandle,
    /// Anchor the first search-matching row claims each render.
    pub(crate) match_anchor: ScrollAnchor,
    /// (section, query) the last match scroll ran for — re-scrolls only
    /// when the target changes, so unrelated re-renders don't snap back.
    pub(crate) last_match_scroll: Option<(Section, String)>,
}

impl SettingsPanel {
    /// `settings` seeds the inputs and pickers — the workspace entity can't
    /// be read here because the panel is built while `Workspace::new` still
    /// holds it.
    pub fn new(ws: Entity<Workspace>, settings: &crate::persist::Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ws, |_, _, cx| cx.notify()).detach();
        let ws = ws.downgrade();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings…"));
        cx.subscribe_in(&search, window, |_, _, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        // Font pickers list every installed family; an empty persisted value
        // means "default" and maps to no selection.
        let fonts = cx.text_system().all_font_names();
        let font_select = crate::views::settings_sections::font_picker(&fonts, &settings.font_family, window, cx);
        let code_font_select = crate::views::settings_sections::font_picker(&fonts, &settings.code_font_family, window, cx);
        let ws_font = ws.clone();
        cx.subscribe_in(&font_select, window, move |_, _, event: &SelectEvent<SearchableVec<String>>, window, cx| {
            let SelectEvent::Confirm(family) = event;
            let _ = ws_font.update(cx, |this, cx| this.set_interface_font(family.clone(), window, cx));
        })
        .detach();
        let ws_code = ws.clone();
        cx.subscribe_in(&code_font_select, window, move |_, _, event: &SelectEvent<SearchableVec<String>>, window, cx| {
            let SelectEvent::Confirm(family) = event;
            let _ = ws_code.update(cx, |this, cx| this.set_code_font(family.clone(), window, cx));
        })
        .detach();

        let content_scroll = ScrollHandle::new();
        let match_anchor = ScrollAnchor::for_handle(content_scroll.clone());

        let contrast_slider = cx.new(|_cx| {
            SliderState::new()
                .min(crate::appearance::CONTRAST_MIN as f32)
                .max(crate::appearance::CONTRAST_MAX as f32)
                .step(5.)
                .default_value(settings.contrast as f32)
        });
        let ws_slider = ws.clone();
        cx.subscribe_in(&contrast_slider, window, move |_, _, event: &SliderEvent, window, cx| {
            let _ = ws_slider.update(cx, |this, cx| this.set_contrast(event, window, cx));
        })
        .detach();

        // Thread-default selects: items are display labels; Confirm maps
        // them back via `access_mode_from_label`/`workspace_mode_from_label`.
        let permissions_select = cx.new(|cx| {
            let selected = if settings.default_permissions.is_empty() {
                None
            } else {
                Some(gpui_kit::component::IndexPath::new(
                    AccessMode::ALL
                        .iter()
                        .position(|m| *m == AccessMode::from_name(&settings.default_permissions))
                        .unwrap_or(0),
                ))
            };
            SelectState::new(AccessMode::ALL.map(|m| m.label().to_string()).to_vec(), selected, window, cx)
        });
        let ws_perms = ws.clone();
        cx.subscribe_in(&permissions_select, window, move |_, _, event: &SelectEvent<Vec<String>>, _window, cx| {
            let SelectEvent::Confirm(label) = event;
            let _ = ws_perms.update(cx, |this, cx| {
                this.set_default_permissions(label.as_deref().map(access_mode_from_label), cx);
            });
        })
        .detach();
        let workspace_select = cx.new(|cx| {
            let mode = WorkspaceMode::from_name(&settings.default_workspace);
            let selected = WorkspaceMode::ALL.iter().position(|m| *m == mode).map(gpui_kit::component::IndexPath::new);
            SelectState::new(WorkspaceMode::ALL.map(|m| workspace_mode_label(m).to_string()).to_vec(), selected, window, cx)
        });
        let ws_ws = ws.clone();
        cx.subscribe_in(&workspace_select, window, move |_, _, event: &SelectEvent<Vec<String>>, _window, cx| {
            let SelectEvent::Confirm(label) = event;
            if let Some(label) = label {
                let _ = ws_ws.update(cx, |this, cx| this.set_default_workspace(workspace_mode_from_label(label), cx));
            }
        })
        .detach();
        let voice_language_select = crate::views::settings_voice::language_picker(&ws, settings, window, cx);
        let editor_select = crate::views::settings_general::editor_picker(&ws, settings, window, cx);
        let mcp_inputs = Self::new_mcp_inputs(window, cx);
        // The detail panel opens on the active provider (or the first one).
        let provider_selection = settings
            .providers
            .iter()
            .find(|p| p.id == settings.selected_provider)
            .or_else(|| settings.providers.first())
            .map(|p| p.id.clone());

        Self {
            ws,
            section: Section::General,
            search,
            provider_inputs: HashMap::new(),
            provider_env_inputs: HashMap::new(),
            provider_selection,
            test_state: HashMap::new(),
            provider_health: HashMap::new(),
            health_pending: false,
            provider_wizard: None,
            font_select,
            code_font_select,
            contrast_slider,
            permissions_select,
            workspace_select,
            editor_select,
            mcp_servers: settings.mcp_servers.clone(),
            mcp_status: HashMap::new(),
            mcp_status_loading: false,
            mcp_adding: false,
            voice_language_select,
            mcp_error: None,
            mcp_inputs,
            content_scroll,
            match_anchor,
            last_match_scroll: None,
        }
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ws) = self.ws.upgrade() else {
            return div().id("settings-screen").test_support();
        };
        // Reconcile per-instance inputs + selection before the view snapshot.
        self.sync_provider_inputs(window, cx);
        self.sync_provider_env_inputs(window, cx);
        // Providers section: kick a health pass when any instance lacks a
        // result — first open, or an instance added while it's showing.
        // Scoped before the `s` borrow: `refresh_provider_health` needs cx.
        let needs_health_probe = self.section == Section::Providers
            && ws.read(cx).provider_instances().iter().any(|p| !self.provider_health.contains_key(&p.id));
        if needs_health_probe {
            self.refresh_provider_health(cx);
        }
        let s = ws.read(cx);
        let view = crate::views::settings_sections::SettingsView::snapshot(self, &ws, cx);
        let theme = cx.theme();
        // Left edge sits at the main sidebar's right edge — the sidebar (now
        // showing the settings nav) and the overlaid toggle stay visible and
        // functional while settings is open, and the chat composer keeps
        // focus so Esc still closes it.
        let left = if s.sidebar_collapsed { 0. } else { s.sidebar_width };
        div()
            .id("settings-screen")
            .test_support()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .left(px(left))
            .occlude()
            .bg(theme.background)
            .text_color(theme.foreground)
            .flex()
            .flex_col()
            // Top strip on the same line as the traffic lights + the overlaid
            // sidebar toggle. When the sidebar is collapsed the toggle sits on
            // this strip's left, so pad past it; when open the toggle is over
            // the sidebar and this strip needs only normal padding.
            .child(
                crate::window::titlebar_drag(
                    div()
                        .id("settings-topbar")
                        .h(px(crate::window::TOP_BAR_H))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(s.sidebar_collapsed, |d| d.pl(px(112.)))
                        .when(!s.sidebar_collapsed, |d| d.px_4())
                        .pr_4()
                        .text_sm()
                        .child(div().text_color(theme.muted_foreground).child("Settings"))
                        .child(div().text_color(theme.muted_foreground).child(IconName::ChevronRight))
                        .child(self.section.label())
                        // Collapsed sidebar → no nav rail, so the "Back to
                        // app" row is gone; give the header its own close
                        // control. Expanded → the nav row already closes.
                        .when(s.sidebar_collapsed, |d| {
                            d.child(div().flex_1()).child(crate::views::settings_nav::close_button(&self.ws, cx))
                        }),
                )
                .test_support(),
            )
            .child(self.render_content(&view, window, cx))
    }
}

impl SettingsPanel {
    fn render_content(
        &mut self, view: &crate::views::settings_sections::SettingsView, window: &mut Window, cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // While searching, scroll the first matching row into view — but
        // only when (section, query) changed, so unrelated re-renders
        // (toggles, status refreshes) don't yank the user's scroll back.
        let key = (self.section, view.search.query.clone());
        let scroll_to_match = !view.search.query.is_empty() && self.last_match_scroll.as_ref() != Some(&key);
        let content = div()
            .id("settings-content")
            .test_support()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .track_scroll(&self.content_scroll)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w_full()
                    .max_w(px(680.))
                    .mx_auto()
                    .p_6()
                    .child(div().text_lg().font_semibold().child(self.section.label()))
                    .child(crate::views::settings_sections::section_body(self.section, view, cx)),
            );
        if scroll_to_match && view.search.anchor_taken.get() {
            self.match_anchor.scroll_to(window, cx);
            self.last_match_scroll = Some(key);
        }
        if view.search.query.is_empty() {
            self.last_match_scroll = None;
        }
        content
    }
}
