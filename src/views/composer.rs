use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{MoveDown, MoveUp, Paste, Textarea};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::slash::SLASH_COMMANDS;
use crate::views::{
    EffortPickerSpec, ModelPickerSpec, PickerProvider, PickerSpec, SavedPromptsSpec, attachment_chips, effort_picker, mention_item,
    model_picker, picker, queued_item, saved_prompts_popover, slash_item, usage_popover,
};
use crate::workspace::Workspace;

const MODES: [&str; 3] = ["Agent", "Plan", "Ask"];

impl Workspace {
    pub fn render_composer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Adopt persisted queues once per window — a restart restores what
        // was pending when the app closed (see `SendQueue::hydrate`).
        self.send_queue.hydrate(&self.project.chats_dir(), &self.chats);
        let running = self.chats[self.active].running;
        // The running turn's own handle decides steerability — a provider
        // switch mid-turn leaves the old turn's channel live.
        let steerable = running && self.backend.supports_steer() && self.chats[self.active].stream.is_some();
        let empty = self.composer.read(cx).value().trim().is_empty();
        let ws = cx.entity();

        // While a steerable turn runs, Enter still queues — the explicit
        // Steer button injects into the turn instead.
        let send_controls = if steerable {
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div().id("steer").test_support().child(
                        Button::new("steer-btn")
                            .primary()
                            .xsmall()
                            .icon(IconName::Send)
                            .label("Steer")
                            .disabled(empty)
                            .on_click(cx.listener(|this, _, window, cx| this.send_steer(window, cx))),
                    ),
                )
                .child(
                    div().id("queue").test_support().child(
                        Button::new("queue-btn")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .label("Queue")
                            .disabled(empty)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
                )
                .child(Button::new("stop").danger().icon(IconName::Pause).on_click(cx.listener(|this, _, _, cx| {
                    this.stop_reply(cx);
                })))
        } else if running {
            div().child(Button::new("stop").danger().icon(IconName::Pause).on_click(cx.listener(|this, _, _, cx| {
                this.stop_reply(cx);
            })))
        } else {
            let btn = Button::new("send")
                .primary()
                .icon(IconName::ArrowUp)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)));
            div().child(if empty { btn.disabled(true) } else { btn })
        };

        let model_picker = model_picker(ModelPickerSpec {
            current_provider: self.selected_provider.clone(),
            current_model: self.model.clone(),
            providers: self
                .enabled_providers()
                .into_iter()
                .map(|p| PickerProvider {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    icon: p.kind.info().icon,
                    models: self.models_for(&p.id),
                })
                .collect(),
            ws: ws.clone(),
            on_pick: None,
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
        // The effort picker rides the selected model's advertised efforts —
        // hidden when the catalog entry carries none (non-codex providers,
        // unfetched catalogs).
        let effort_options = self.effort_options();
        let effort = (!effort_options.is_empty()).then(|| {
            effort_picker(EffortPickerSpec {
                current: self.effort.clone(),
                default_effort: self.selected_model_info().map(|m| m.default_effort.to_string()).unwrap_or_default(),
                options: effort_options,
                ws: ws.clone(),
            })
        });
        // The ★ popover lists saved prompts; "Save current…" is armed by a
        // non-empty composer.
        let prompts_popover = saved_prompts_popover(SavedPromptsSpec {
            prompts: self.prompts.prompts.clone(),
            can_save: !empty,
            ws: ws.clone(),
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
                    .filter(|(c, _)| q.is_empty() || c.starts_with(q.as_str()))
                    .map(|&(cmd, desc)| slash_item(cmd, desc, &ws, cx).into_any_element())
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
            // Capture-phase so clipboard images/files attach before the
            // input's own paste handler can insert their paths as text.
            .capture_action::<Paste>(cx.listener(|this, _, window, cx| this.paste_attachments(window, cx)))
            // Up/Down recall the chat's prompt history — capture-phase so a
            // consumed keystroke never reaches the textarea's own cursor
            // move. `history_recall` propagates when recall doesn't apply
            // (mid-text cursor, no history), keeping multi-line moves intact.
            .capture_action::<MoveUp>(cx.listener(|this, _, window, cx| this.history_recall(true, window, cx)))
            .capture_action::<MoveDown>(cx.listener(|this, _, window, cx| this.history_recall(false, window, cx)))
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
                            div().flex().flex_col().gap_0p5().pb_1().border_b_1().border_color(cx.theme().border).children(
                                queued
                                    .iter()
                                    .enumerate()
                                    .map(|(ix, item)| queued_item(item, ix == 0, ix + 1 == queued.len(), &ws, cx).into_any_element())
                                    .collect::<Vec<_>>(),
                            ),
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
                            .child(crate::views::composer_voice::dictate_button(self.voice.phase, cx))
                            .child(send_controls),
                    )
                    .when_some(self.voice.note.clone(), |d, note| {
                        d.child(crate::views::composer_voice::dictate_note(note, self.voice.note_is_error, cx))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(model_picker)
                            .child(mode_picker)
                            .when_some(effort, |d, e| d.child(e))
                            .child(prompts_popover)
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
                            )
                            .when_some(usage_popover(&self.chats[self.active].usage, &ws, cx), |d, meter| d.child(meter)),
                    ),
            )
    }
}
