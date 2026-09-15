use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::{Notification, NotificationDelivery};
use gpui_kit::*;

use crate::model::{Chat, MessageKind, Role};
use crate::workspace::Workspace;

/// What a finished reply surfaces: an in-app toast always, plus a system
/// notification and dock bounce while the window is unfocused.
#[derive(Debug, PartialEq)]
pub(crate) struct DoneNotice {
    /// Chat title — the toast/system headline.
    title: SharedString,
    /// A preview of the reply's first line, or the failure's first line.
    body: String,
    failed: bool,
    /// Post to the OS notification center — only while unfocused.
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

impl Workspace {
    /// System bell (gated by `notify_sound`), an in-app toast, plus — when
    /// the window is inactive — a system notification and dock bounce. The
    /// toast/system surfaces need `notify_on_done`; the sound is its own
    /// toggle so a reply can chime without a popup. Clicking either surface
    /// activates the window and opens the chat. The platform layer is a
    /// safe no-op where notifications are unsupported or the app isn't
    /// bundled, so this never panics.
    pub(crate) fn notify_done(&mut self, chat_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.notify_sound {
            play_done_sound(window);
        }
        if !self.notify_on_done {
            return;
        }
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        let notice = Self::done_notice(chat, window.is_window_active());
        let ws = cx.entity().downgrade();
        let note = if notice.failed { Notification::error(notice.body) } else { Notification::success(notice.body) }
            .title(notice.title)
            .id1::<ReplyDone>(("reply-done", chat_id))
            .delivery(if notice.system { NotificationDelivery::InAppAndSystem } else { NotificationDelivery::InApp })
            .on_click(move |_, window, cx| {
                let _ = ws.update(cx, |ws, cx| ws.open_notified_chat(chat_id, window, cx));
            });
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
            self.select_chat(ix, window, cx);
        }
        cx.notify();
    }

    /// The notice for a finished reply — `system` stays false while the
    /// window is focused, so a reply the user is already watching never
    /// pings the OS notification center.
    fn done_notice(chat: &Chat, window_active: bool) -> DoneNotice {
        let body = if chat.failed_flag {
            Self::error_detail(chat).map_or_else(|| "Reply failed".to_string(), |line| format!("Reply failed — {line}"))
        } else {
            Self::reply_preview(chat).unwrap_or_else(|| "Reply complete".to_string())
        };
        DoneNotice {
            title: chat.title.clone(),
            body,
            failed: chat.failed_flag,
            system: !window_active,
        }
    }

    /// First non-empty line of the last assistant text, capped at 80 chars —
    /// the notification body doubles as a reply preview so the user can tell
    /// what finished without opening the chat.
    pub(crate) fn reply_preview(chat: &Chat) -> Option<String> {
        let line = Self::last_assistant_text(chat)?.lines().find(|l| !l.trim().is_empty())?.trim();
        let mut preview: String = line.chars().take(81).collect();
        if preview.chars().count() > 80 {
            preview.truncate(preview.char_indices().nth(80).map_or(preview.len(), |(i, _)| i));
            preview.push('…');
        }
        Some(preview)
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
            role: Role::Assistant,
            kind: MessageKind::Text(text.into()),
            rating: None,
            usage: None,
            attachments: vec![],
            at: SystemTime::now(),
        });
    }

    #[test]
    fn success_notice_is_in_app_only_when_focused() {
        let notice = Workspace::done_notice(&chat("Build fix"), true);
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
    fn success_notice_goes_system_when_unfocused() {
        let notice = Workspace::done_notice(&chat("Build fix"), false);
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
    fn failed_notice_strips_error_markdown() {
        let mut chat = chat("Build fix");
        chat.failed_flag = true;
        assistant_text(&mut chat, "**Error:** codex exited 1\nmore detail");
        let notice = Workspace::done_notice(&chat, false);
        assert!(notice.failed);
        assert_eq!(notice.body, "Reply failed — codex exited 1");
    }

    #[test]
    fn failed_notice_falls_back_without_error_text() {
        let mut chat = chat("Build fix");
        chat.failed_flag = true;
        let notice = Workspace::done_notice(&chat, false);
        assert_eq!(notice.body, "Reply failed");
    }

    #[test]
    fn success_notice_previews_the_reply() {
        let mut chat = chat("Build fix");
        assistant_text(&mut chat, "Fixed the borrow error\nand two more lines");
        let notice = Workspace::done_notice(&chat, false);
        assert_eq!(notice.body, "Fixed the borrow error");
    }

    #[test]
    fn preview_skips_blank_lines_and_truncates() {
        let mut chat = chat("Build fix");
        assistant_text(&mut chat, &format!("\n  \n{}", "x".repeat(120)));
        let notice = Workspace::done_notice(&chat, false);
        assert_eq!(notice.body.chars().count(), 81, "80 chars + ellipsis");
        assert!(notice.body.ends_with('…'));
    }
}
