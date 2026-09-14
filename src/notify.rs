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
    /// "Reply complete", or the failure's first line.
    body: String,
    failed: bool,
    /// Post to the OS notification center — only while unfocused.
    system: bool,
}

/// Marker type for the per-chat notification id — a repeat push for the
/// same chat replaces the previous toast and system notification instead
/// of stacking.
struct ReplyDone;

impl Workspace {
    /// In-app toast plus — when the window is inactive — a system
    /// notification and dock bounce. No-op unless `notify_on_done` is set.
    /// Clicking either surface activates the window and opens the chat.
    /// The platform layer is a safe no-op where notifications are
    /// unsupported or the app isn't bundled, so this never panics.
    pub(crate) fn notify_done(&mut self, chat_id: u64, window: &mut Window, cx: &mut Context<Self>) {
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
    fn open_notified_chat(&mut self, chat_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.chat_index(chat_id) {
            self.select_chat(ix, window, cx);
        }
    }

    /// The notice for a finished reply — `system` stays false while the
    /// window is focused, so a reply the user is already watching never
    /// pings the OS notification center.
    fn done_notice(chat: &Chat, window_active: bool) -> DoneNotice {
        let body = if chat.failed_flag {
            Self::error_detail(chat).map_or_else(|| "Reply failed".to_string(), |line| format!("Reply failed — {line}"))
        } else {
            "Reply complete".to_string()
        };
        DoneNotice {
            title: chat.title.clone(),
            body,
            failed: chat.failed_flag,
            system: !window_active,
        }
    }

    /// First line of the last assistant text — the backend writes failures
    /// as `**Error:** …`, so strip the marker for a clean headline. A
    /// failed turn whose last text is a partial reply still reads better
    /// than a bare "Reply failed".
    fn error_detail(chat: &Chat) -> Option<String> {
        chat.messages.iter().rev().filter(|m| m.role == Role::Assistant).find_map(|m| match &m.kind {
            MessageKind::Text(t) => {
                let line = t.trim_start_matches("**Error:**").lines().next()?.trim();
                (!line.is_empty()).then(|| line.to_string())
            },
            _ => None,
        })
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
}
