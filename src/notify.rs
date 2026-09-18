use gpui_kit::component::WindowExt;
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::{Notification, NotificationDelivery};
use gpui_kit::*;

use crate::model::{Chat, MessageKind, Role};
use crate::workspace::Workspace;

/// What a finished reply surfaces: an in-app toast always, plus a system
/// notification and dock bounce while the reply isn't on screen.
#[derive(Debug, PartialEq)]
pub(crate) struct DoneNotice {
    /// Chat title — the toast/system headline.
    title: SharedString,
    /// A preview of the reply's first line, or the failure's first line.
    body: String,
    failed: bool,
    /// Post to the OS notification center — only while the user isn't
    /// watching the chat (background chat or unfocused window).
    system: bool,
}

/// Times the done sound played — the test platform's `play_system_bell` is
/// a silent no-op, so tests count calls here instead of listening for audio.
#[cfg(test)]
pub(crate) static SOUND_PLAYS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The turn-finished chime: gpui's system bell (NSBeep on macOS) — a
/// synchronous platform call that never blocks the UI thread.
fn play_done_sound(window: &Window) {
    #[cfg(test)]
    SOUND_PLAYS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    window.play_system_bell();
}

/// Marker type for the per-chat notification id — a repeat push for the
/// same chat replaces the previous toast and system notification instead
/// of stacking.
struct ReplyDone;

/// The failed toast's Retry button: dismisses its own toast, then surfaces
/// the chat and re-sends the last prompt.
fn retry_button(ws: WeakEntity<Workspace>, chat_id: u64, toast: Entity<Notification>) -> Button {
    Button::new(("reply-retry", chat_id)).label("Retry").on_click(move |_, window, cx| {
        toast.update(cx, |n, cx| n.dismiss(window, cx));
        let _ = ws.update(cx, |ws, cx| ws.retry_notified_chat(chat_id, window, cx));
    })
}

/// Marker type for the per-chat approval-waiting notification id — a new
/// request replaces the previous toast instead of stacking.
struct ApprovalNeeded;

/// First non-empty line of `text`, capped at 100 chars + ellipsis — the
/// shared body-preview shape for toast and system notices.
fn preview_line(text: &str) -> Option<String> {
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    let mut preview: String = line.chars().take(101).collect();
    if preview.chars().count() > 100 {
        preview.truncate(preview.char_indices().nth(100).map_or(preview.len(), |(i, _)| i));
        preview.push('…');
    }
    Some(preview)
}

impl Workspace {
    /// System bell (gated by `notify_sound`), an in-app toast, plus — when
    /// the user isn't watching the chat — a system notification and dock
    /// bounce. "Not watching" means the finished chat isn't the active one
    /// or the window is unfocused; the OS surface also needs
    /// `notify_background`, and the toast/system surfaces need
    /// `notify_on_done`. The sound is its own toggle so a reply can chime
    /// without a popup. Clicking either surface activates the window and
    /// opens the chat; a failed turn's toast also carries a Retry button
    /// that re-sends the last prompt. The platform layer is a safe no-op where
    /// notifications are unsupported or the app isn't bundled, so this
    /// never panics.
    pub(crate) fn notify_done(&mut self, chat_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.notify_sound {
            play_done_sound(window);
        }
        if !self.notify_on_done {
            return;
        }
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == chat_id);
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        let notice = Self::done_notice(chat, is_active && window.is_window_active(), self.notify_background);
        let ws = cx.entity().downgrade();
        let ws_retry = cx.entity().downgrade();
        let note = if notice.failed { Notification::error(notice.body) } else { Notification::success(notice.body) }
            .title(notice.title)
            .id1::<ReplyDone>(("reply-done", chat_id))
            .delivery(if notice.system { NotificationDelivery::InAppAndSystem } else { NotificationDelivery::InApp })
            .on_click(move |_, window, cx| {
                let _ = ws.update(cx, |ws, cx| ws.open_notified_chat(chat_id, window, cx));
            });
        // A dead turn shouldn't need the chat opened before it can be
        // re-sent: failed toasts carry a Retry button. An action also pins
        // the toast (no autohide), which is right for a failure that wants
        // eyes — the click dismisses it either way.
        let note = if notice.failed {
            note.action(move |_, _, cx| retry_button(ws_retry.clone(), chat_id, cx.entity()))
        } else {
            note
        };
        window.push_notification(note, cx);
        if notice.system {
            window.request_attention();
        }
    }

    /// Select the chat a notification points at — the chat may have been
    /// deleted since the notice was posted, so resolve by id at click time.
    /// Overlay state is cleared too: a click while settings is open must
    /// reveal the chat, not leave the overlay covering it.
    fn open_notified_chat(&mut self, chat_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        if let Some(ix) = self.chat_index(chat_id) {
            self.mark_chat_activity_read(self.chats[ix].created_at);
            self.select_chat(ix, window, cx);
            // The click means "show me the new thing" — pin the view to
            // the tail so the fresh reply is on screen even when the chat
            // was already selected mid-history (select_chat no-ops then).
            self.scroller.update(cx, |s, cx| s.scroll_to_end(cx));
        }
        cx.notify();
    }

    /// An unanswered approval request is a turn blocked on a click — unlike
    /// a finished reply it never arrives by itself, so it earns the same
    /// surfaces: an in-app toast always, a system notification and dock
    /// bounce while unwatched (the same `notify_on_done`/`notify_background`
    /// gates). No action button: the command must be read in the chat
    /// before approving, so click-to-open is the only honest affordance.
    /// And no chime — `notify_sound` is specifically the *finish* bell.
    /// `summary` is the `"<kind>: <what will run>"` line from the caller.
    pub(crate) fn notify_approval(&mut self, chat_id: u64, summary: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.notify_on_done {
            return;
        }
        let is_active = self.chats.get(self.active).is_some_and(|c| c.id == chat_id);
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        let system = self.notify_background && !(is_active && window.is_window_active());
        let title = chat.title.clone();
        let body = format!("Needs approval — {}", preview_line(summary).unwrap_or_else(|| summary.trim().to_string()));
        let ws = cx.entity().downgrade();
        let note = Notification::warning(body)
            .title(title)
            .id1::<ApprovalNeeded>(("approval-needed", chat_id))
            .delivery(if system { NotificationDelivery::InAppAndSystem } else { NotificationDelivery::InApp })
            .on_click(move |_, window, cx| {
                let _ = ws.update(cx, |ws, cx| {
                    ws.open_notified_chat(chat_id, window, cx);
                    ws.scroll_to_pending_approval(cx);
                });
            });
        window.push_notification(note, cx);
        if system {
            window.request_attention();
        }
    }

    /// The toast's Retry button: surface the failed chat, then re-send its
    /// last prompt. `retry_last` is active-scoped, so only retry when the
    /// requested chat actually became active — a chat deleted since the
    /// toast posted must not resend whatever happens to be selected.
    fn retry_notified_chat(&mut self, chat_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.open_notified_chat(chat_id, window, cx);
        if self.chats.get(self.active).is_some_and(|c| c.id == chat_id) {
            self.retry_last(cx);
        }
    }

    /// The notice for a finished reply. `watched` is "the user can see the
    /// reply land" — the chat is active AND the window focused — so
    /// `system` stays false only then; `allow_system` is the
    /// `notify_background` toggle, off = never ping the OS.
    fn done_notice(chat: &Chat, watched: bool, allow_system: bool) -> DoneNotice {
        let body = if chat.failed_flag {
            Self::error_detail(chat).map_or_else(|| "Reply failed".to_string(), |line| format!("Reply failed — {line}"))
        } else {
            Self::reply_preview(chat).unwrap_or_else(|| "Reply complete".to_string())
        };
        DoneNotice {
            title: chat.title.clone(),
            body,
            failed: chat.failed_flag,
            system: allow_system && !watched,
        }
    }

    /// First non-empty line of the last assistant text, capped at 100
    /// chars — the notification body doubles as a reply preview so the
    /// user can tell what finished without opening the chat.
    pub(crate) fn reply_preview(chat: &Chat) -> Option<String> {
        preview_line(Self::last_assistant_text(chat)?)
    }

    /// The last assistant Text message's content, if any.
    fn last_assistant_text(chat: &Chat) -> Option<&str> {
        chat.messages.iter().rev().filter(|m| m.role == Role::Assistant).find_map(|m| match &m.kind {
            MessageKind::Text(t) => Some(t.as_ref()),
            _ => None,
        })
    }

    /// First line of the last assistant text — the backend writes failures
    /// as `**Error:** …`, so strip the marker for a clean headline. A
    /// failed turn whose last text is a partial reply still reads better
    /// than a bare "Reply failed".
    pub(crate) fn error_detail(chat: &Chat) -> Option<String> {
        let line = Self::last_assistant_text(chat)?.trim_start_matches("**Error:**").lines().next()?.trim();
        (!line.is_empty()).then(|| line.to_string())
    }
}

#[cfg(test)]
mod tests {
    // Narrow imports only: `use super::*` would pull `gpui_kit::test` into
    // scope and shadow the built-in `#[test]` attribute.
    use super::DoneNotice;
    use crate::model::{Chat, ChatMessage, MessageKind, Role};
    use crate::workspace::Workspace;
    use std::time::SystemTime;

    fn chat(title: &str) -> Chat {
        Chat::new(7, title)
    }

    fn assistant_text(chat: &mut Chat, text: &str) {
        std::rc::Rc::make_mut(&mut chat.messages).push(ChatMessage {
            alternatives: vec![],
            role: Role::Assistant,
            kind: MessageKind::Text(text.into()),
            rating: None,
            bookmarked: false,
            pinned: false,
            usage: None,
            attachments: vec![],
            at: SystemTime::now(),
        });
    }

    #[test]
    fn success_notice_is_in_app_only_when_watched() {
        let notice = Workspace::done_notice(&chat("Build fix"), true, true);
        assert_eq!(
            notice,
            DoneNotice {
                title: "Build fix".into(),
                body: "Reply complete".into(),
                failed: false,
                system: false
            }
        );
    }

    #[test]
    fn success_notice_goes_system_when_unwatched() {
        // Unwatched covers both cases the caller ORs in: a background chat
        // finishing while focused, and any chat while the window is
        // unfocused.
        let notice = Workspace::done_notice(&chat("Build fix"), false, true);
        assert_eq!(
            notice,
            DoneNotice {
                title: "Build fix".into(),
                body: "Reply complete".into(),
                failed: false,
                system: true
            }
        );
    }

    #[test]
    fn background_toggle_off_keeps_notice_in_app() {
        let notice = Workspace::done_notice(&chat("Build fix"), false, false);
        assert!(!notice.system, "notify_background off must never post to the OS");
    }

    #[test]
    fn failed_notice_strips_error_markdown() {
        let mut chat = chat("Build fix");
        chat.failed_flag = true;
        assistant_text(&mut chat, "**Error:** codex exited 1\nmore detail");
        let notice = Workspace::done_notice(&chat, false, true);
        assert!(notice.failed);
        assert_eq!(notice.body, "Reply failed — codex exited 1");
    }

    #[test]
    fn failed_notice_falls_back_without_error_text() {
        let mut chat = chat("Build fix");
        chat.failed_flag = true;
        let notice = Workspace::done_notice(&chat, false, true);
        assert_eq!(notice.body, "Reply failed");
    }

    #[test]
    fn success_notice_previews_the_reply() {
        let mut chat = chat("Build fix");
        assistant_text(&mut chat, "Fixed the borrow error\nand two more lines");
        let notice = Workspace::done_notice(&chat, false, true);
        assert_eq!(notice.body, "Fixed the borrow error");
    }

    #[test]
    fn preview_skips_blank_lines_and_truncates() {
        let mut chat = chat("Build fix");
        assistant_text(&mut chat, &format!("\n  \n{}", "x".repeat(140)));
        let notice = Workspace::done_notice(&chat, false, true);
        assert_eq!(notice.body.chars().count(), 101, "100 chars + ellipsis");
        assert!(notice.body.ends_with('…'));
    }
}
