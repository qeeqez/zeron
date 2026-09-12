use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::{apply_pick, attachment_chips, mention_item, slash_item};
use crate::workspace::Workspace;

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
            options: &crate::model::MODELS,
            ws: ws.clone(),
            set: |this, v| {
                this.model = v.into();
                this.save_settings();
            },
        });
        let mode_picker = picker(PickerSpec {
            id: "mode",
            current: self.mode.clone(),
            options: &MODES,
            ws: ws.clone(),
            set: |this, v| {
                this.mode = v.into();
                this.save_settings();
            },
        });
        let composer_text = self.composer.read(cx).value().to_string();
        let mention_open = composer_text.contains('@');
        let mention_query = composer_text.rsplit('@').next().unwrap_or("").to_lowercase();
        let mention_items: Vec<AnyElement> = self
            .project_files
            .iter()
            .filter(|f| mention_query.is_empty() || f.to_lowercase().contains(&mention_query))
            .take(8)
            .map(|f| mention_item(f, &ws, cx).into_any_element())
            .collect();
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
                .p_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().input)
                .when(mention_open && !mention_items.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .pb_1()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .children(mention_items),
                    )
                })
                .p_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().input)
                .when(!self.chats[self.active].attachments.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .pb_1()
                            .children(attachment_chips(&self.chats[self.active], &ws, cx)),
                    )
                })
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
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(f32::from(self.font_size)))
                                .child(Textarea::new(&self.composer).appearance(false)),
                        )
                        .child(Button::new("attach").ghost().icon(IconName::Paperclip).on_click(cx.listener(|this, _, _, cx| {
                            this.attach_file(cx);
                        })))
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
                                .child(format!("~{} tok", self.token_estimate())),
                        )
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
