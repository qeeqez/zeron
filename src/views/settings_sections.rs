//! Content pane bodies for each settings section — the controls that used to
//! live in the flat settings sheet, grouped by nav section.

use gpui_kit::base::StyledExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::select::{SearchableVec, SelectState};
use gpui_kit::component::slider::SliderState;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::collections::HashMap;

use crate::backend::AccessMode;
use crate::views::settings::SettingsPanel;
use crate::views::settings_nav::Section;
use crate::views::settings_provider_env::EnvRow;
use crate::views::settings_providers::ProviderInputs;
use crate::views::settings_search::SearchCtx;
use crate::workspace::Workspace;

/// Snapshot of workspace state + owned inputs the section bodies render from.
pub struct SettingsView {
    pub notify: bool,
    /// "Notification sound" switch — `Workspace::notify_sound`.
    pub notify_sound: bool,
    /// "Notify on background replies" switch — `Workspace::notify_background`.
    pub notify_background: bool,
    pub font_size: f32,
    pub code_font_size: f32,
    pub sidebar_frosted: bool,
    pub contrast: u16,
    pub backend: &'static str,
    pub word_wrap: bool,
    pub theme: String,
    pub ws: Entity<Workspace>,
    /// The settings panel entity — provider rows/wizard drive its selection
    /// and wizard state.
    pub panel: Entity<SettingsPanel>,
    /// Per-instance detail inputs, keyed by instance id.
    pub provider_inputs: HashMap<String, ProviderInputs>,
    /// Per-instance Variables inputs, keyed by instance id.
    pub provider_env_inputs: HashMap<String, Vec<EnvRow>>,
    /// The instance the Providers detail panel shows.
    pub provider_selection: Option<String>,
    /// Per-instance "Test connection" probe outcomes, keyed by instance id.
    pub test_state: HashMap<String, crate::views::settings_provider_test::TestState>,
    pub access: AccessMode,
    pub font_select: Entity<SelectState<SearchableVec<String>>>,
    pub code_font_select: Entity<SelectState<SearchableVec<String>>>,
    pub contrast_slider: Entity<SliderState>,
    /// Default permissions for new threads — `AccessMode::ALL` labels.
    pub permissions_select: Entity<SelectState<Vec<String>>>,
    /// Default workspace for new threads — `WorkspaceMode::ALL` labels.
    pub workspace_select: Entity<SelectState<Vec<String>>>,
    /// Preferred editor for "Open in Editor" — `PreferredEditor::ALL` labels.
    pub editor_select: Entity<SelectState<Vec<String>>>,
    /// Configured MCP servers — the MCP Servers section's list.
    pub mcp_servers: Vec<crate::mcp::McpServer>,
    /// Live per-server status keyed by name (codex `mcpServerStatus/list`).
    pub mcp_status: HashMap<String, crate::mcp::McpStatus>,
    /// A status fetch is in flight.
    pub mcp_status_loading: bool,
    /// The add-server form is open.
    pub mcp_adding: bool,
    /// Add-form validation error.
    pub mcp_error: Option<String>,
    /// The add form's inputs.
    pub mcp_inputs: crate::views::settings_mcp::McpInputs,
    /// Voice dictation state for the Voice section.
    pub voice_enabled: bool,
    pub voice_language_select: Entity<SelectState<Vec<String>>>,
    pub voice_on_device: bool,
    pub voice_phase: crate::voice::DictationPhase,
    /// Last test-mic result line (or live partial while recording).
    pub voice_test_result: Option<String>,
    /// The Custom Instructions section's multiline field — owned by the
    /// workspace so typed text survives settings open/close.
    pub instructions_input: Entity<gpui_kit::component::input::TextareaState>,
    /// The Project section's setup-script field — same workspace-owned
    /// rationale as `instructions_input`.
    pub setup_script_input: Entity<gpui_kit::component::input::TextareaState>,
    /// Update-check state for the Profile section's About row.
    pub update: crate::update::UpdateState,

    /// Settings-search state for this render — query, match wash and the
    /// scroll anchor the first matching row claims (see `settings_search`).
    pub search: SearchCtx,
}

/// The content pane for the selected section.
pub fn section_body(section: Section, s: &SettingsView, cx: &App) -> impl IntoElement {
    let body = match section {
        Section::General => crate::views::settings_general::general_section(s, cx).into_any_element(),
        Section::Instructions => crate::views::settings_instructions::instructions_section(s, cx).into_any_element(),
        Section::Appearance => crate::views::settings_appearance::appearance_section(s, cx).into_any_element(),
        Section::Profile => crate::views::settings_profile::profile_section(s, cx).into_any_element(),
        Section::Project => crate::views::settings_project::project_section(s, cx).into_any_element(),
        Section::Providers => crate::views::settings_providers::providers_section(s, cx).into_any_element(),
        Section::Shortcuts => crate::views::settings_shortcuts::shortcuts_section(s, cx).into_any_element(),
        Section::Voice => crate::views::settings_voice::voice_section(s, cx).into_any_element(),
        Section::McpServers => crate::views::settings_mcp::mcp_section(s, cx).into_any_element(),
    };
    div()
        .id(SharedString::from(format!("settings-section-{}", section.name())))
        .test_support()
        .flex()
        .flex_col()
        .gap_4()
        .child(body)
}

impl SettingsView {
    /// Snapshot the workspace + panel inputs for one render. Lives here (not
    /// `settings.rs`) to keep that file under the SLOC cap.
    pub(crate) fn snapshot(panel: &SettingsPanel, ws: &Entity<Workspace>, cx: &Context<SettingsPanel>) -> Self {
        let s = ws.read(cx);
        Self {
            notify: s.notify_on_done,
            notify_sound: s.notify_sound,
            notify_background: s.notify_background,
            font_size: s.font_size,
            code_font_size: s.code_font_size,
            sidebar_frosted: s.sidebar_frosted,
            contrast: s.contrast,
            backend: s.backend.name(),
            access: s.access,
            word_wrap: s.word_wrap,
            theme: s.theme.clone(),
            ws: ws.clone(),
            panel: cx.entity(),
            provider_inputs: panel.provider_inputs.clone(),
            mcp_servers: panel.mcp_servers.clone(),
            mcp_status: panel.mcp_status.clone(),
            mcp_status_loading: panel.mcp_status_loading,
            mcp_adding: panel.mcp_adding,
            mcp_error: panel.mcp_error.clone(),
            mcp_inputs: panel.mcp_inputs.clone(),
            provider_env_inputs: panel.provider_env_inputs.clone(),
            provider_selection: panel.provider_selection.clone(),
            test_state: panel.test_state.clone(),
            font_select: panel.font_select.clone(),
            code_font_select: panel.code_font_select.clone(),
            contrast_slider: panel.contrast_slider.clone(),
            editor_select: panel.editor_select.clone(),
            voice_enabled: s.voice.enabled,
            voice_language_select: panel.voice_language_select.clone(),
            voice_on_device: s.voice.on_device,
            voice_phase: s.voice.phase,
            voice_test_result: s.voice.test_result.clone(),
            update: s.update.clone(),
            permissions_select: panel.permissions_select.clone(),
            workspace_select: panel.workspace_select.clone(),
            instructions_input: s.instructions_input.clone(),
            setup_script_input: s.setup_script_input.clone(),
            search: SearchCtx::new(panel.search.read(cx).value().trim().to_lowercase(), panel.match_anchor.clone(), cx),
        }
    }
}

pub(crate) fn group_label(text: &'static str, search: &SearchCtx, cx: &App) -> AnyElement {
    search.wrap(text, div().pt_2().text_sm().font_semibold().text_color(cx.theme().muted_foreground).child(text))
}

/// A label + `Switch` row that writes a workspace flag, then persists
/// settings and re-renders — `set` applies the requested value plus any side
/// effects. `row` bundles the element id and label to stay under the
/// arg-count lint.
pub(crate) fn toggle_row(
    row: (&'static str, &'static str), on: bool, ws: Entity<Workspace>,
    set: fn(&mut Workspace, bool, &mut Window, &mut Context<Workspace>), search: &SearchCtx,
) -> AnyElement {
    search.wrap(
        row.1,
        div().flex().items_center().gap_2().text_xs().child(row.1).child(div().flex_1()).child(
            Switch::new(row.0).checked(on).small().accessibility_label(row.1).on_click(move |next, window, cx| {
                let next = *next;
                ws.update(cx, |this, cx| {
                    set(this, next, window, cx);
                    this.save_settings();
                    cx.notify();
                });
            }),
        ),
    )
}

/// A searchable font-family picker; `current` selects the matching row when
/// it's a real family name (empty = default → no selection). The delegate is
/// `SearchableVec`, not `Vec` — only it implements `perform_search`, so a
/// plain `Vec` would render the search box but never filter the list.
/// Lives here (not `settings.rs`) to keep that file under the SLOC cap.
pub(crate) fn font_picker(
    fonts: &[String], current: &str, window: &mut Window, cx: &mut Context<SettingsPanel>,
) -> Entity<SelectState<SearchableVec<String>>> {
    let selected = fonts.iter().position(|f| f == current).map(gpui_kit::component::IndexPath::new);
    cx.new(|cx| SelectState::new(SearchableVec::new(fonts.to_vec()), selected, window, cx).searchable(true))
}
