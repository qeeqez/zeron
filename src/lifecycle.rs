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
        let root = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let files = cx.background_executor().spawn(async move { crate::files::scan_project_files(&root) }).await;
            let _ = this.update(cx, |this, cx| {
                this.project_files = files;
                cx.notify();
            });
        })
        .detach();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let mut dirty = self.chats.iter().any(|c| c.running);
        for agent in &mut self.agents {
            if agent.status == crate::model::AgentStatus::Running {
                agent.elapsed_secs += 1;
                dirty = true;
            }
        }
        if dirty {
            cx.notify();
        }
    }
}
