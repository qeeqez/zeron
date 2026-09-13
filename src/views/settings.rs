use crate::workspace::Workspace;
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Codex-style settings screen: a left nav rail over a content pane, rendered
/// as a full-window overlay from `Workspace::render` (sheets/dialogs can't
/// host a two-pane layout and weren't mounted anyway).
pub struct SettingsPanel {
    ws: Entity<Workspace>,
    pub(crate) section: Section,
    search: Entity<InputState>,
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

/// A nav-rail section. `name` feeds element ids; `label` is what renders.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    General,
    Appearance,
    Voice,
    Profile,
    Shortcuts,
    McpServers,
}

impl Section {
    const ALL: [Section; 6] = [Self::General, Self::Appearance, Self::Voice, Self::Profile, Self::Shortcuts, Self::McpServers];
    /// (group header, sections) pairs for the rail — matches the Codex
    /// settings sidebar grouping.
    const GROUPS: [(&'static str, &'static [Section]); 3] = [
        ("Personal", &[Self::General, Self::Appearance, Self::Voice, Self::Profile]),
        ("Coding", &[Self::Shortcuts]),
        ("Integrations", &[Self::McpServers]),
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Voice => "voice",
            Self::Profile => "profile",
            Self::Shortcuts => "shortcuts",
            Self::McpServers => "mcp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Voice => "Voice",
            Self::Profile => "Profile",
            Self::Shortcuts => "Shortcuts",
            Self::McpServers => "MCP Servers",
        }
    }

    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::General => IconName::SlidersHorizontal,
            Self::Appearance => IconName::Palette,
            Self::Voice => IconName::Mic,
            Self::Profile => IconName::CircleUser,
            Self::Shortcuts => IconName::Keyboard,
            Self::McpServers => IconName::PlugZap,
        }
    }
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
        let panel = cx.entity();
        let query = self.search.read(cx).value().to_lowercase();
        let theme = cx.theme();
        div()
            .id("settings-screen")
            .test_support()
            .absolute()
            .inset_0()
            .occlude()
            .bg(theme.background)
            .text_color(theme.foreground)
            .flex()
            .child(self.render_nav(&query, &panel, cx))
            .child(self.render_content(&view, cx))
    }
}

impl SettingsPanel {
    fn render_nav(&self, query: &str, panel: &Entity<SettingsPanel>, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let width = self.ws.read(cx).settings_nav_width;
        let ws = self.ws.clone();
        let rail = div()
            .id("settings-nav")
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .relative()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(crate::views::settings_nav::back_row(&self.ws, cx))
            .child(Input::new(&self.search).prefix(IconName::Search).appearance(true))
            // Drag handle on the rail's right edge — same pattern as the main
            // sidebar's `sidebar-resize`.
            .child(
                div()
                    .id("settings-nav-resize")
                    .test_support()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .w(px(5.))
                    .cursor_col_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        move |_, _, cx| {
                            ws.update(cx, |this, cx| {
                                this.resizing_settings_nav = true;
                                cx.notify();
                            });
                        },
                    ),
            );
        if query.is_empty() {
            rail.children(Section::GROUPS.iter().map(|(name, sections)| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pt_3()
                    .child(div().px_2().pb_1().text_xs().text_color(theme.muted_foreground).child(*name))
                    .children(sections.iter().map(|s| crate::views::settings_nav::nav_item(*s, *s == self.section, panel, cx)))
            }))
        } else {
            rail.children(
                Section::ALL
                    .iter()
                    .filter(|s| s.label().to_lowercase().contains(query))
                    .map(|s| crate::views::settings_nav::nav_item(*s, *s == self.section, panel, cx)),
            )
        }
    }

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
