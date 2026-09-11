use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

const MODELS: [&str; 3] = ["gpt-5-codex", "gpt-5", "gpt-5-mini"];
const MODES: [&str; 3] = ["Agent", "Plan", "Ask"];
const SLASH_COMMANDS: [&str; 6] = ["clear", "compact", "export", "help", "model", "rename"];

struct PickerSpec {
    id: &'static str,
    current: SharedString,
    options: &'static [&'static str],
    ws: Entity<Workspace>,
    set: fn(&mut Workspace, &'static str),
}

impl Workspace {
    pub fn render_composer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.chats[self.active].running;
        let ws = cx.entity();

        let send_button = if running {
            Button::new("stop").danger().icon(IconName::Pause).on_click(cx.listener(|this, _, _, cx| {
                this.stop_reply(cx);
            }))
        } else {
            let empty = self.composer.read(cx).value().trim().is_empty();
            let btn = Button::new("send")
                .primary()
                .icon(IconName::ArrowUp)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)));
            if empty { btn.disabled(true) } else { btn }
        };

        let model_picker = picker(PickerSpec {
            id: "model",
            current: self.model.clone(),
            options: &MODELS,
            ws: ws.clone(),
            set: |this, v| this.model = v.into(),
        });
        let mode_picker = picker(PickerSpec {
            id: "mode",
            current: self.mode.clone(),
            options: &MODES,
            ws: ws.clone(),
            set: |this, v| this.mode = v.into(),
        });
        let composer_text = self.composer.read(cx).value().to_string();
        let slash_open = composer_text.starts_with('/');
        let slash_query = composer_text.trim_start_matches('/').to_lowercase();
        let slash_items: Vec<AnyElement> = SLASH_COMMANDS
            .iter()
            .filter(|c| slash_query.is_empty() || c.to_lowercase().contains(&slash_query))
            .map(|cmd| slash_item(cmd, &ws, cx).into_any_element())
            .collect();

        div().p_3().border_t_1().border_color(cx.theme().border).child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().input)
                .when(slash_open && !slash_items.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .pb_1()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .children(slash_items),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap_2()
                        .child(div().flex_1().child(Textarea::new(&self.composer).appearance(false)))
                        .child(send_button),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(model_picker)
                        .child(mode_picker)
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} chars", self.composer.read(cx).value().len())),
                        ),
                ),
        )
    }
}

fn picker(spec: PickerSpec) -> impl IntoElement {
    let PickerSpec { id, current, options, ws, set } = spec;
    Button::new(id)
        .ghost()
        .label(current.clone())
        .icon(IconName::ChevronsUpDown)
        .dropdown_menu(move |menu, _window, _cx| {
            options.iter().fold(menu, |menu, opt| {
                let ws = ws.clone();
                let checked = *opt == current.as_str();
                menu.item(PopupMenuItem::new(*opt).checked(checked).on_click(move |_, _, cx| {
                    apply_pick(&ws, set, opt, cx);
                }))
            })
        })
}

fn apply_slash(ws: &Entity<Workspace>, cmd: &str, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.composer.update(cx, |s, cx| {
            s.set_value(format!("/{cmd} "), window, cx);
        });
    });
}

fn apply_pick(ws: &Entity<Workspace>, set: fn(&mut Workspace, &'static str), opt: &'static str, cx: &mut App) {
    ws.update(cx, |this, cx| {
        set(this, opt);
        cx.notify();
    });
}

fn slash_item(cmd: &str, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let ws = ws.clone();
    let cmd_str = cmd.to_string();
    div()
        .id(SharedString::from(format!("slash-{cmd}")))
        .cursor_pointer()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .hover(|d| d.bg(cx.theme().accent))
        .child(format!("/{cmd}"))
        .on_click(move |_, window, cx| {
            apply_slash(&ws, &cmd_str, window, cx);
        })
}
