use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// In-app toast plus — when the window is inactive — a system
    /// notification and dock bounce. No-op unless `notify_on_done` is set.
    /// The platform layer is a safe no-op where notifications are
    /// unsupported or the app isn't bundled, so this never panics.
    pub(crate) fn notify_done(&mut self, chat_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        let title = chat.title.clone();
        if !self.notify_on_done {
            return;
        }
        window.push_notification(Notification::success(format!("{title} — reply complete")), cx);
        if let Some(note) = Self::done_notification(chat_id, &title, window.is_window_active()) {
            window.request_attention();
            cx.show_system_notification(note);
        }
    }

    /// The system notification for a finished reply — `None` while the
    /// window is focused, so a reply the user is already watching never
    /// pings the OS notification center.
    fn done_notification(chat_id: u64, title: &str, window_active: bool) -> Option<SystemNotification> {
        (!window_active).then(|| SystemNotification {
            tag: format!("reply-{chat_id}").into(),
            title: "Rixl Code".into(),
            body: format!("{title} — reply complete").into(),
            actions: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    // Narrow imports only: `use super::*` would pull `gpui_kit::test` into
    // scope and shadow the built-in `#[test]` attribute.
    use crate::workspace::Workspace;
    use gpui_kit::SystemNotification;

    #[test]
    fn silent_when_window_focused() {
        assert!(Workspace::done_notification(1, "chat", true).is_none());
    }

    #[test]
    fn posts_when_window_unfocused() {
        let note = Workspace::done_notification(7, "Build fix", false).expect("unfocused reply should notify");
        assert_eq!(
            note,
            SystemNotification {
                tag: "reply-7".into(),
                title: "Rixl Code".into(),
                body: "Build fix — reply complete".into(),
                actions: vec![],
            }
        );
    }
}
