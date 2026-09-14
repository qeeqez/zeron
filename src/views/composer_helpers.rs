use std::collections::{HashMap, HashSet, VecDeque};

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::Chat;
use crate::workspace::Workspace;

/// A message committed while a reply was running — sends when the turn ends.
/// Keyed by chat id: the queue is composer UI state, not transcript data.
#[derive(Clone)]
pub struct Queued {
    pub id: u64,
    pub text: String,
}

thread_local! {
    static QUEUE: std::cell::RefCell<HashMap<u64, VecDeque<Queued>>> = std::cell::RefCell::new(HashMap::new());
    /// Chat ids with an in-flight drain task — dedupes drain spawns.
    static DRAINING: std::cell::RefCell<HashSet<u64>> = std::cell::RefCell::new(HashSet::new());
    static NEXT_QUEUED_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Queue `text` behind the running turn on `chat_id`; `live` prunes dead chats.
pub fn enqueue(chat_id: u64, text: String, live: impl Fn(u64) -> bool) {
    let id = NEXT_QUEUED_ID.with(|n| n.replace(n.get() + 1));
    QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        q.retain(|id, _| live(*id));
        q.entry(chat_id).or_default().push_back(Queued { id, text });
    });
}

/// Snapshot of a chat's queue for rendering.
pub fn queued(chat_id: u64) -> Vec<Queued> {
    QUEUE.with(|q| q.borrow().get(&chat_id).map(|d| d.iter().cloned().collect()).unwrap_or_default())
}

pub fn pop_queued(chat_id: u64) -> Option<Queued> {
    QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        let item = q.get_mut(&chat_id)?.pop_front();
        if q.get(&chat_id).is_some_and(|d| d.is_empty()) {
            q.remove(&chat_id);
        }
        item
    })
}

/// Drop one queued message (the ✕ on a queued row).
pub fn remove_queued(chat_id: u64, queued_id: u64) {
    QUEUE.with(|q| {
        if let Some(d) = q.borrow_mut().get_mut(&chat_id) {
            d.retain(|item| item.id != queued_id);
        }
    });
}

/// Mark a drain task in flight for `chat_id`; false when one already runs.
pub fn draining_begin(chat_id: u64) -> bool {
    DRAINING.with(|d| d.borrow_mut().insert(chat_id))
}

pub fn draining_end(chat_id: u64) {
    DRAINING.with(|d| d.borrow_mut().remove(&chat_id));
}

/// Forget a chat's queue entirely (the chat was deleted).
pub fn drop_queue(chat_id: u64) {
    QUEUE.with(|q| q.borrow_mut().remove(&chat_id));
}

/// Outcome of one queue-drain attempt for a chat.
pub enum Drain {
    /// A queued message was sent — the new turn is running.
    Sent,
    /// The chat is active but still busy — check again shortly.
    Wait,
    /// Queue empty, chat deleted, or chat backgrounded — stop draining.
    Done,
}

impl Workspace {
    /// Send the next queued message on `chat_id` once its turn ends. Only the
    /// active chat drains (`start_reply` targets `self.active`).
    pub(crate) fn drain_queued(&mut self, chat_id: u64, window: &mut Window, cx: &mut Context<Self>) -> Drain {
        let Some(chat) = self.chats.get(self.active) else {
            drop_queue(chat_id);
            return Drain::Done;
        };
        if chat.id != chat_id {
            return Drain::Done;
        }
        if chat.running {
            return Drain::Wait;
        }
        let Some(item) = pop_queued(chat_id) else { return Drain::Done };
        self.send_text(&item.text, window, cx);
        Drain::Sent
    }
}

pub fn slash_item(cmd: &str, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let ws = ws.clone();
    let cmd_str = cmd.to_string();
    div()
        .id(SharedString::from(format!("slash-{cmd}")))
        .test_support()
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

pub fn mention_item(file: &str, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let ws = ws.clone();
    let path = file.to_string();
    div()
        .id(SharedString::from(format!("mention-{file}")))
        .test_support()
        .cursor_pointer()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .hover(|d| d.bg(cx.theme().accent))
        .child(format!("@{file}"))
        .on_click(move |_, window, cx| {
            apply_mention(&ws, &path, window, cx);
        })
}

pub fn attachment_chips(chat: &Chat, ws: &Entity<Workspace>, cx: &mut App) -> Vec<AnyElement> {
    chat.attachments
        .iter()
        .enumerate()
        .map(|(ix, path)| {
            let ws = ws.clone();
            let full = path.to_string();
            let name = std::path::Path::new(&full)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(full.clone());
            div()
                .id(ix)
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_0p5()
                .rounded_md()
                .bg(cx.theme().secondary)
                .text_xs()
                .child(IconName::FileText)
                .child(
                    div()
                        .id(("reveal-attach", ix))
                        .cursor_pointer()
                        .child(name)
                        .on_click(move |_, _, cx| cx.reveal_path(std::path::Path::new(&full))),
                )
                .child(
                    div()
                        .id(("remove-attach", ix))
                        .cursor_pointer()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::X)
                        .on_click(move |_, _, cx| {
                            ws.update(cx, |this, cx| this.remove_attachment(ix, cx));
                        }),
                )
                .into_any_element()
        })
        .collect()
}

fn apply_slash(ws: &Entity<Workspace>, cmd: &str, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| this.run_command(cmd, window, cx));
}
fn apply_mention(ws: &Entity<Workspace>, path: &str, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.composer.update(cx, |s, cx| {
            let cur = s.value().to_string();
            let before = cur.rsplit_once('@').map(|(b, _)| b).unwrap_or("");
            s.set_value(format!("{before}@{path} "), window, cx);
            s.focus(window, cx);
        });
        // `set_value` suppresses Change — nudge the workspace so the menu closes.
        cx.notify();
    });
}

pub fn apply_pick(ws: &Entity<Workspace>, set: fn(&mut Workspace, &'static str), opt: &'static str, cx: &mut App) {
    ws.update(cx, |this, cx| {
        set(this, opt);
        cx.notify();
    });
}

/// One queued-message row: dimmed text plus an ✕ that drops it.
pub fn queued_item(item: &Queued, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let ws = ws.clone();
    let id = item.id;
    div()
        .id(SharedString::from(format!("queued-{}", item.id)))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .whitespace_nowrap()
                .text_ellipsis()
                .child(item.text.clone()),
        )
        .child(
            div()
                .id(SharedString::from(format!("dequeue-{}", item.id)))
                .test_support()
                .child(
                    Button::new(SharedString::from(format!("dequeue-btn-{}", item.id)))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .on_click(move |_, _, cx| {
                            ws.update(cx, |this, cx| {
                                remove_queued(this.chats[this.active].id, id);
                                cx.notify();
                            });
                        }),
                ),
        )
}
