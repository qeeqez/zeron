use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::*;

use crate::model::{ChatMessage, MessageKind, Role, ToolCall, ToolStatus};
use crate::workspace::Workspace;

/// Simulated agent reply: a tool call that runs, then a text answer.
/// Replaced by the real backend event stream later (docs/todo/backend.md).
pub fn simulate_reply(this: &mut Workspace, cx: &mut Context<Workspace>) {
    let chat_ix = this.active;
    this.chats[chat_ix].messages.push(ChatMessage {
        role: Role::Assistant,
        kind: MessageKind::Tool(ToolCall {
            name: "shell".into(),
            detail: "cargo build --locked".into(),
            status: ToolStatus::Running,
        }),
    });
    this.scroller.update(cx, |s, cx| {
        s.append(1, cx);
    });
    cx.notify();

    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(Duration::from_millis(900)).await;
        let _ = this.update(cx, |this, cx| this.finish_reply(chat_ix, cx));
    })
    .detach();
}

impl Workspace {
    pub(crate) fn finish_reply(&mut self, chat_ix: usize, cx: &mut Context<Self>) {
        let failed = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() % 4 == 0).unwrap_or(false);
        let chat = &mut self.chats[chat_ix];
        if let Some(last) = chat.messages.last_mut()
            && let MessageKind::Tool(tool) = &mut last.kind
        {
            tool.status = if failed { ToolStatus::Failed } else { ToolStatus::Done };
        }
        let text = if failed {
            "The command failed — see the tool output above."
        } else {
            "Done. The build is green — 0 warnings, all checks passed."
        };
        chat.messages.push(ChatMessage { role: Role::Assistant, kind: MessageKind::Text(text.into()) });
        chat.running = false;
        self.scroller.update(cx, |s, cx| {
            s.append(1, cx);
        });
        cx.notify();
    }
}
