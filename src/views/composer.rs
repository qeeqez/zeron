use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::send::SLASH_COMMANDS;
use crate::views::{ModelPickerSpec, apply_pick, attachment_chips, mention_item, model_picker, queued_item, slash_item};
use crate::workspace::Workspace;

const MODES: [&str; 3] = ["Agent", "Plan", "Ask"];

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

        let model_picker = model_picker(ModelPickerSpec {
            current_provider: self.provider,
            current_model: self.model.clone(),
            providers: self.enabled_providers().into_iter().map(|p| (*p, self.picker_options(p.id))).collect(),
            ws: ws.clone(),
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
        // The mention menu tracks the LAST `@` token: it must sit at a word
        // boundary ("user@host" stays quiet) and its query may not contain
        // whitespace, so a completed "@path next-word" closes the menu.
        let mention_query = composer_text
            .rsplit_once('@')
            .map(|(before, q)| (before, q.to_lowercase()))
            .filter(|(before, q)| before.chars().next_back().is_none_or(|p| p.is_whitespace()) && !q.chars().any(|c| c.is_whitespace()));
        let mention_items: Vec<AnyElement> = mention_query
            .map(|(_, q)| {
                self.project_files
                    .iter()
                    .filter(|f| q.is_empty() || f.to_lowercase().contains(q.as_str()))
                    .take(8)
                    .map(|f| mention_item(f, &ws, cx).into_any_element())
                    .collect()
            })
            .unwrap_or_default();
        // `/` only at position 0; once an argument (whitespace) follows the
        // command word the menu steps aside.
        let slash_query = composer_text
            .strip_prefix('/')
            .map(str::to_lowercase)
            .filter(|q| !q.chars().any(|c| c.is_whitespace()));
        let slash_items: Vec<AnyElement> = slash_query
            .map(|q| {
                SLASH_COMMANDS
                    .iter()
                    .filter(|c| q.is_empty() || c.contains(q.as_str()))
                    .map(|cmd| slash_item(cmd, &ws, cx).into_any_element())
                    .collect()
            })
            .unwrap_or_default();
        let queued = self.send_queue.queued(self.chats[self.active].id);
        // Re-selecting a chat with a pending queue re-arms its drain (the
        // enqueue-time waiter exits when the chat backgrounds).
        if !queued.is_empty() {
            self.spawn_queue_drain(self.chats[self.active].id, cx);
        }

        div()
            .p_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .on_drop::<ExternalPaths>(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_attachments(paths.0.to_vec(), cx);
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .p_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().input)
                    .when(!mention_items.is_empty(), |d| {
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
                    .when(!slash_items.is_empty(), |d| {
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
                    .when(!queued.is_empty(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .pb_1()
                                .border_b_1()
                                .border_color(cx.theme().border)
                                .children(queued.iter().map(|item| queued_item(item, &ws, cx).into_any_element()).collect::<Vec<_>>()),
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
