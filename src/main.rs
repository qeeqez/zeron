mod backend;
mod backend_run;
mod chat_ops;
mod files;
mod model;
mod palette;
mod send;
mod simulate;

mod persist;
mod views;
mod window;
mod workspace;

use gpui_kit::component::Root;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;
use workspace::Workspace;

actions!([
    NewChat, DeleteChat, ToggleSidebar, ToggleAgents, OpenPalette, ThemeLight, ThemeDark, Chat1, Chat2, Chat3, Chat4, Chat5, Chat6, Chat7,
    Chat8, Chat9, CloseWindow, QuitApp, OpenSettings, SearchChat, CopyTranscript, EmojiPalette, RevealChats, EscapeKey, ShortcutsHelp,
]);

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.chats[self.active].title.clone();
        window.set_window_title(&format!("{title} — Rixl Code"));
        window.set_window_edited(!self.composer.read(cx).value().is_empty());
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
            .on_action(move |_: &DeleteChat, window, cx| {
                ws_del.update(cx, |this, cx| this.delete_chat(this.active, window, cx));
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
            .on_action(|_: &CloseWindow, window, _cx| {
                window.remove_window();
            })
            .on_action({
                let ws = cx.entity();
                move |_: &OpenSettings, window, cx| {
                    ws.update(cx, |this, cx| this.open_settings(window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &SearchChat, window, cx| {
                    ws.update(cx, |this, cx| this.open_chat_search(window, cx));
                }
            })
            .on_action(|_: &QuitApp, _window, cx| {
                cx.quit();
            })
            .on_action(|_: &EmojiPalette, window, _cx| {
                window.show_character_palette();
            })
            .on_action(|_: &RevealChats, _window, cx| {
                cx.reveal_path(&crate::persist::chats_dir());
            })
            .on_action({
                let ws = cx.entity();
                move |_: &CopyTranscript, _window, cx| {
                    ws.update(cx, |this, cx| this.copy_transcript(cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &EscapeKey, window, cx| {
                    ws.update(cx, |this, cx| this.escape(window, cx));
                }
            })
            .on_action({
                let ws = cx.entity();
                move |_: &ShortcutsHelp, window, cx| {
                    ws.update(cx, |this, cx| this.shortcuts_help(window, cx));
                }
            })
            .h_full()
            .flex()
            .flex_col()
            .on_mouse_move(cx.listener(|this, ev: &gpui_kit::MouseMoveEvent, _, cx| {
                if this.resizing_sidebar && ev.dragging() {
                    this.sidebar_width = f32::from(ev.position.x).clamp(180.0, 480.0);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resizing_sidebar {
                        this.resizing_sidebar = false;
                        this.save_settings();
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(window, cx))
                    .child(self.render_chat(window, cx))
                    .when(self.agents_panel_open, |d| d.child(self.render_agents_panel(window, cx))),
            )
    }
}

/// Build an `on_action` handler that selects chat `A::IX`.
fn chat_switch<A: Action + ChatIx>(cx: &mut Context<Workspace>) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    let ws = cx.entity();
    move |_: &A, window, cx| {
        ws.update(cx, |this, cx| this.select_chat(A::IX, window, cx));
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
    gpui_kit::application().with_assets(gpui_kit::assets::Assets::new("")).run(|cx| {
        gpui_kit::init(cx);
        cx.set_menus([
            gpui_kit::Menu::new("Rixl Code").items([
                gpui_kit::MenuItem::action("About Rixl Code", gpui_kit::NoAction),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Settings…", OpenSettings),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Quit Rixl Code", QuitApp),
            ]),
            gpui_kit::Menu::new("File").items([
                gpui_kit::MenuItem::action("New Chat", NewChat),
                gpui_kit::MenuItem::action("Reveal Chats Folder", RevealChats),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Close Window", CloseWindow),
            ]),
            gpui_kit::Menu::new("Edit").items([
                gpui_kit::MenuItem::os_action("Cut", gpui_kit::NoAction, gpui_kit::OsAction::Cut),
                gpui_kit::MenuItem::os_action("Copy", gpui_kit::NoAction, gpui_kit::OsAction::Copy),
                gpui_kit::MenuItem::os_action("Paste", gpui_kit::NoAction, gpui_kit::OsAction::Paste),
                gpui_kit::MenuItem::os_action("Select All", gpui_kit::NoAction, gpui_kit::OsAction::SelectAll),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Emoji & Symbols", EmojiPalette),
                gpui_kit::MenuItem::action("Copy Transcript", CopyTranscript),
            ]),
            gpui_kit::Menu::new("View").items([
                gpui_kit::MenuItem::action("Toggle Sidebar", ToggleSidebar),
                gpui_kit::MenuItem::action("Toggle Agents", ToggleAgents),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Command Palette", OpenPalette),
            ]),
        ]);
        cx.bind_keys([
            KeyBinding::new("escape", EscapeKey, Some("workspace")),
            KeyBinding::new("cmd-n", NewChat, Some("workspace")),
            KeyBinding::new("cmd-,", OpenSettings, Some("workspace")),
            KeyBinding::new("cmd-/", ShortcutsHelp, Some("workspace")),
            KeyBinding::new("cmd-shift-backspace", DeleteChat, Some("workspace")),
            KeyBinding::new("cmd-b", ToggleSidebar, Some("workspace")),
            KeyBinding::new("cmd-j", ToggleAgents, Some("workspace")),
            KeyBinding::new("cmd-k", OpenPalette, Some("workspace")),
            KeyBinding::new("cmd-w", CloseWindow, Some("workspace")),
            KeyBinding::new("cmd-f", SearchChat, Some("workspace")),
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
            cx.open_window(
                WindowOptions {
                    window_min_size: Some(Size { width: px(800.), height: px(600.) }),
                    window_bounds: window::saved_window_bounds(),
                    window_background: gpui_kit::WindowBackgroundAppearance::Blurred,
                    // Traffic lights float over the sidebar; title hidden, top strip draggable.
                    titlebar: Some(gpui_kit::TitlebarOptions {
                        title: Some("Rixl Code".into()),
                        appears_transparent: true,
                        traffic_light_position: None,
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| Workspace::new(window, cx));
                    let ws = view.clone();
                    let handle = window.window_handle();
                    window.on_window_should_close(cx, move |window, cx| window::confirm_close(&ws, handle, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("failed to open window");
        })
        .detach();
    });
}
