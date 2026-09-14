use crate::views::settings_nav::Section;
use crate::workspace::Workspace;
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::select::{SearchableVec, SelectEvent, SelectState};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

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
    url_input: Entity<InputState>,
    key_input: Entity<InputState>,
    /// Interface font family picker — `SearchableVec<String>` delegate over
    /// the installed font names (a plain `Vec` delegate never filters).
    pub(crate) font_select: Entity<SelectState<SearchableVec<String>>>,
    /// Code (mono) font family picker.
    pub(crate) code_font_select: Entity<SelectState<SearchableVec<String>>>,
    /// Contrast slider, 50–200%.
    pub(crate) contrast_slider: Entity<SliderState>,
}

impl SettingsPanel {
    /// `settings` seeds the inputs and pickers — the workspace entity can't
    /// be read here because the panel is built while `Workspace::new` still
    /// holds it.
    pub fn new(ws: Entity<Workspace>, settings: &crate::persist::Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ws, |_, _, cx| cx.notify()).detach();
        let ws = ws.downgrade();
        let url_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("https://…");
            s.set_value(settings.http_url.clone(), window, cx);
            s
        });
        let key_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("ENV_VAR_NAME");
            s.set_value(settings.http_key_env.clone(), window, cx);
            s
        });
        // Persist on every edit — the backend reads these at send time.
        for (input, field) in [(url_input.clone(), Field::Url), (key_input.clone(), Field::KeyEnv)] {
            let ctx = FieldCtx { field, ws: ws.clone() };
            cx.subscribe_in(&input, window, move |_, state, event: &InputEvent, _window, cx| {
                on_http_field(&ctx, state, event, cx);
            })
            .detach();
        }
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
        let font_select = font_picker(&fonts, &settings.font_family, window, cx);
        let code_font_select = font_picker(&fonts, &settings.code_font_family, window, cx);
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

        Self {
            ws,
            section: Section::General,
            search,
            url_input,
            key_input,
            font_select,
            code_font_select,
            contrast_slider,
        }
    }
}

/// A searchable font-family picker; `current` selects the matching row when
/// it's a real family name (empty = default → no selection). The delegate is
/// `SearchableVec`, not `Vec` — only it implements `perform_search`, so a
/// plain `Vec` would render the search box but never filter the list.
fn font_picker(fonts: &[String], current: &str, window: &mut Window, cx: &mut Context<SettingsPanel>) -> Entity<SelectState<SearchableVec<String>>> {
    let selected = fonts.iter().position(|f| f == current).map(gpui_kit::component::IndexPath::new);
    cx.new(|cx| SelectState::new(SearchableVec::new(fonts.to_vec()), selected, window, cx).searchable(true))
}

/// Write a changed http config field to the workspace and persist it.
fn on_http_field(ctx: &FieldCtx, state: &Entity<InputState>, event: &InputEvent, cx: &mut App) {
    if !matches!(event, InputEvent::Change) {
        return;
    }
    let value = state.read(cx).value().to_string();
    let _ = ctx.ws.update(cx, |this, _cx| {
        match ctx.field {
            Field::Url => this.http_url = value.clone(),
            Field::KeyEnv => this.http_key_env = value.clone(),
        }
        // HttpBackend clones url/key_env at construction — rebuild it so
        // the next send uses the edited values instead of the stale ones.
        if this.backend.name() == "http" {
            this.backend = std::sync::Arc::new(crate::backend::HttpBackend::new(this.http_url.clone(), this.http_key_env.clone()));
        }
        this.save_settings();
    });
}

/// Everything a field-change handler needs — bundled so the subscribe
/// closure and handler stay under the argument-count lint.
struct FieldCtx {
    field: Field,
    ws: WeakEntity<Workspace>,
}

/// Which http config field an input writes — keeps the subscribe loop
/// under the argument-count lint.
#[derive(Clone, Copy)]
enum Field {
    Url,
    KeyEnv,
}

impl Render for SettingsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ws) = self.ws.upgrade() else {
            return div().id("settings-screen").test_support();
        };
        let s = ws.read(cx);
        let view = crate::views::settings_sections::SettingsView {
            notify: s.notify_on_done,
            font_size: s.font_size,
            code_font_size: s.code_font_size,
            sidebar_frosted: s.sidebar_frosted,
            contrast: s.contrast,
            backend: s.backend.name(),
            access: s.access,
            word_wrap: s.word_wrap,
            theme: s.theme.clone(),
            ws: ws.clone(),
            url_input: self.url_input.clone(),
            key_input: self.key_input.clone(),
            font_select: self.font_select.clone(),
            code_font_select: self.code_font_select.clone(),
            contrast_slider: self.contrast_slider.clone(),
        };
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
            .child(self.render_content(&view, cx))
    }
}

impl SettingsPanel {
    fn render_content(&self, view: &crate::views::settings_sections::SettingsView, cx: &App) -> impl IntoElement {
        div().id("settings-content").test_support().flex_1().min_w_0().h_full().overflow_y_scroll().child(
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
        )
    }
}
