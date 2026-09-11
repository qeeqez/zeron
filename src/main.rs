mod chat_ops;
mod model;
mod palette;
mod simulate;
mod views;
mod workspace;

use gpui_kit::component::Root;
use gpui_kit::component::status_bar::StatusBar;

use gpui_kit::component::theme::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;
use workspace::Workspace;

actions!(
    workspace,
    [
        NewChat, DeleteChat, ToggleSidebar, ToggleAgents, OpenPalette, ThemeLight, ThemeDark, Chat1, Chat2, Chat3, Chat4, Chat5, Chat6,
        Chat7, Chat8, Chat9
    ]
);

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ws_new = cx.entity();
        let ws_del = cx.entity();
        let ws_side = cx.entity();
        let ws_agents = cx.entity();
        let ws_palette = cx.entity();
        div()
            .key_context("workspace")
            .on_action(move |_: &NewChat, _, cx| {
                ws_new.update(cx, |this, cx| this.new_chat(cx));
            })
            .on_action(move |_: &DeleteChat, _, cx| {
                ws_del.update(cx, |this, cx| this.delete_chat(this.active, cx));
            })
            .on_action(chat_switch::<Chat1>(cx))
            .on_action(chat_switch::<Chat2>(cx))
            .on_action(chat_switch::<Chat3>(cx))
            .on_action(chat_switch::<Chat4>(cx))
            .on_action(chat_switch::<Chat5>(cx))
            .on_action(chat_switch::<Chat6>(cx))
            .on_action(chat_switch::<Chat7>(cx))
            .on_action(chat_switch::<Chat8>(cx))
            .on_action(chat_switch::<Chat9>(cx))
            .on_action(move |_: &ToggleSidebar, _, cx| {
                ws_side.update(cx, |this, cx| this.toggle_sidebar(cx));
            })
            .on_action(move |_: &ToggleAgents, _, cx| {
                ws_agents.update(cx, |this, cx| this.toggle_agents_panel(cx));
            })
            .on_action(move |_: &OpenPalette, window, cx| {
                ws_palette.update(cx, |this, cx| this.open_palette(window, cx));
            })
            .on_action(move |_: &ThemeLight, window, cx| {
                Theme::change(ThemeMode::Light, Some(window), cx);
            })
            .on_action(move |_: &ThemeDark, window, cx| {
                Theme::change(ThemeMode::Dark, Some(window), cx);
            })
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(window, cx))
                    .child(self.render_chat(window, cx))
                    .when(self.agents_panel_open, |d| d.child(self.render_agents_panel(window, cx))),
            )
            .child(
                StatusBar::new()
                    .left(div().text_xs().child(format!("{} · {} · rixlcode", self.model, self.mode)))
                    .right(div().text_xs().text_color(cx.theme().muted_foreground).child(format!(
                        "{} chats · ~{} tok",
                        self.chats.len(),
                        self.token_estimate()
                    ))),
            )
    }
}

/// Build an `on_action` handler that selects chat `A::IX`.
fn chat_switch<A: Action + ChatIx>(cx: &mut Context<Workspace>) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    let ws = cx.entity();
    move |_: &A, _, cx| {
        ws.update(cx, |this, cx| this.select_chat(A::IX, cx));
    }
}

trait ChatIx {
    const IX: usize;
}
macro_rules! chat_ix {
    ($($t:ident => $n:literal),*) => { $(impl ChatIx for $t { const IX: usize = $n; })* };
}
chat_ix!(Chat1 => 0, Chat2 => 1, Chat3 => 2, Chat4 => 3, Chat5 => 4, Chat6 => 5, Chat7 => 6, Chat8 => 7, Chat9 => 8);

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-n", NewChat, Some("workspace")),
            KeyBinding::new("cmd-shift-backspace", DeleteChat, Some("workspace")),
            KeyBinding::new("cmd-b", ToggleSidebar, Some("workspace")),
            KeyBinding::new("cmd-j", ToggleAgents, Some("workspace")),
            KeyBinding::new("cmd-k", OpenPalette, Some("workspace")),
            KeyBinding::new("cmd-1", Chat1, Some("workspace")),
            KeyBinding::new("cmd-2", Chat2, Some("workspace")),
            KeyBinding::new("cmd-3", Chat3, Some("workspace")),
            KeyBinding::new("cmd-4", Chat4, Some("workspace")),
            KeyBinding::new("cmd-5", Chat5, Some("workspace")),
            KeyBinding::new("cmd-6", Chat6, Some("workspace")),
            KeyBinding::new("cmd-7", Chat7, Some("workspace")),
            KeyBinding::new("cmd-8", Chat8, Some("workspace")),
            KeyBinding::new("cmd-9", Chat9, Some("workspace")),
        ]);
        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| Workspace::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
