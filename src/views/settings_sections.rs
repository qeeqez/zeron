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
use crate::workspace::Workspace;

/// Snapshot of workspace state + owned inputs the section bodies render from.
pub struct SettingsView {
    pub notify: bool,
    /// "Notification sound" switch — `Workspace::notify_sound`.
    pub notify_sound: bool,
    pub font_size: u8,
    pub code_font_size: u8,
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
        Section::Shortcuts => crate::views::settings_shortcuts::shortcuts_section(cx).into_any_element(),
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

pub(crate) fn group_label(text: &'static str, cx: &App) -> Div {
    div().pt_2().text_sm().font_semibold().text_color(cx.theme().muted_foreground).child(text)
}

/// A label + `Switch` row that writes a workspace flag, then persists
/// settings and re-renders — `set` applies the requested value plus any side
/// effects. `row` bundles the element id and label to stay under the
/// arg-count lint.
pub(crate) fn toggle_row(
    row: (&'static str, &'static str), on: bool, ws: Entity<Workspace>, set: fn(&mut Workspace, bool, &mut Window, &mut Context<Workspace>),
) -> impl IntoElement {
    div().flex().items_center().gap_2().text_xs().child(row.1).child(div().flex_1()).child(
        Switch::new(row.0).checked(on).small().accessibility_label(row.1).on_click(move |next, window, cx| {
            let next = *next;
            ws.update(cx, |this, cx| {
                set(this, next, window, cx);
                this.save_settings();
                cx.notify();
            });
        }),
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
