//! Background tasks started at workspace creation: the 1s elapsed-time
//! ticker and the project-file scan for the @-mention picker.

use std::time::Duration;

use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// Spawn the ticker and the file scan. Called once from `Workspace::new`.
    pub(crate) fn start_background(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let _ = this.update(cx, Self::tick);
            }
        })
        .detach();
        // Scan project files off the UI thread — a large tree would block
        // launch; the @-mention picker just stays empty until it lands.
        cx.spawn(async move |this, cx| {
            let files = cx.background_executor().spawn(async { crate::files::scan_project_files() }).await;
            let _ = this.update(cx, |this, cx| {
                this.project_files = files;
                cx.notify();
            });
        })
        .detach();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.chats.iter().any(|c| c.running) {
            cx.notify();
        }
    }
}
