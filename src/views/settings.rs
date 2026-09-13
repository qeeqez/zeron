use crate::views::settings_nav::Section;
use crate::workspace::Workspace;
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Codex-style settings screen: a left nav rail over a content pane, rendered
/// as a full-window overlay from `Workspace::render` (sheets/dialogs can't
/// host a two-pane layout and weren't mounted anyway).
pub struct SettingsPanel {
    pub(crate) ws: Entity<Workspace>,
    pub(crate) section: Section,
    pub(crate) search: Entity<InputState>,
    url_input: Entity<InputState>,
    key_input: Entity<InputState>,
}

impl SettingsPanel {
    /// `http` seeds the url/key inputs — the workspace entity can't be read
    /// here because the panel is built while `Workspace::new` still holds it.
    pub fn new(ws: Entity<Workspace>, http: (String, String), window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ws, |_, _, cx| cx.notify()).detach();
        let url_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("https://…");
            s.set_value(http.0, window, cx);
            s
        });
        let key_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("ENV_VAR_NAME");
            s.set_value(http.1, window, cx);
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
        Self { ws, section: Section::General, search, url_input, key_input }
    }
}

/// Write a changed http config field to the workspace and persist it.
fn on_http_field(ctx: &FieldCtx, state: &Entity<InputState>, event: &InputEvent, cx: &mut App) {
    if !matches!(event, InputEvent::Change) {
        return;
    }
    let value = state.read(cx).value().to_string();
    ctx.ws.update(cx, |this, _cx| {
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
    ws: Entity<Workspace>,
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
        let s = self.ws.read(cx);
        let view = crate::views::settings_sections::SettingsView {
            notify: s.notify_on_done,
            font_size: s.font_size,
            backend: s.backend.name(),
            access: s.access,
            word_wrap: s.word_wrap,
            theme: s.theme.clone(),
            ws: self.ws.clone(),
            url_input: self.url_input.clone(),
            key_input: self.key_input.clone(),
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
                        .child(self.section.label()),
                )
                .test_support(),
            )
            .child(self.render_content(&view, cx))
    }
}

impl SettingsPanel {
    fn render_content(&self, view: &crate::views::settings_sections::SettingsView, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let ws = self.ws.clone();
        div().id("settings-content").test_support().flex_1().min_w_0().h_full().overflow_y_scroll().child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .w_full()
                .max_w(px(680.))
                .mx_auto()
                .p_6()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(div().text_lg().font_semibold().child(self.section.label()))
                        .child(div().flex_1())
                        .child(
                            div()
                                .id("settings-close")
                                .test_support()
                                .cursor_pointer()
                                .text_color(theme.muted_foreground)
                                .child(IconName::X)
                                .on_click(move |_, _, cx| {
                                    ws.update(cx, |this, cx| this.close_settings(cx));
                                }),
                        ),
                )
                .child(crate::views::settings_sections::section_body(self.section, view, cx)),
        )
    }
}
