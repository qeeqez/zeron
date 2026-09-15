//! Thread defaults: the `default_model`/`default_permissions`/
//! `default_workspace` settings applied to every NEW chat, plus the
//! per-thread provider/model/access/workdir stamps that keep existing
//! threads on their own configuration when the defaults change.

use gpui_kit::*;

use crate::workspace::Workspace;

impl Workspace {
    /// The access mode new threads start on — `None` follows the current
    /// workspace access at `new_chat` time.
    pub fn default_permissions(&self) -> Option<crate::backend::AccessMode> {
        self.default_permissions
    }

    /// Set the access mode new threads start on and persist it —
    /// `Settings.default_permissions`. `None` = follow the current access.
    /// Existing threads keep their own stamped mode.
    pub fn set_default_permissions(&mut self, mode: Option<crate::backend::AccessMode>, cx: &mut Context<Self>) {
        self.default_permissions = mode;
        self.save_settings();
        cx.notify();
    }

    /// Where new threads run: the project checkout or a per-thread worktree.
    pub fn default_workspace(&self) -> crate::worktree::WorkspaceMode {
        self.default_workspace
    }

    /// Set where new threads run and persist it — `Settings.default_workspace`.
    /// Existing threads keep their own stamped workdir.
    pub fn set_default_workspace(&mut self, mode: crate::worktree::WorkspaceMode, cx: &mut Context<Self>) {
        self.default_workspace = mode;
        self.save_settings();
        cx.notify();
    }

    /// Stamp the active chat's provider/model/access from the live
    /// workspace selection — called before switching away so the outgoing
    /// thread keeps its own configuration.
    pub(crate) fn stamp_thread(&mut self) {
        let Some(chat) = self.chats.get_mut(self.active) else { return };
        chat.provider = self.selected_provider.clone();
        chat.model = self.model.to_string();
        chat.access = Some(self.access);
    }

    /// Apply the persisted thread defaults to the just-created active
    /// chat: provider+model, access mode, and — for `Worktree` — a fresh
    /// git worktree the thread's backend turns run in.
    pub(crate) fn apply_thread_defaults(&mut self, cx: &mut Context<Self>) {
        let dm = self.default_model.clone();
        if !dm.provider_instance_id.is_empty()
            && self.providers.iter().any(|p| p.id == dm.provider_instance_id && p.enabled)
            && !self.select_model(&dm.provider_instance_id, &dm.model_id, cx)
        {
            // The configured model is gone from the catalog — still switch
            // to the instance so the thread lands on its first model.
            if let Some(first) = self.models_for(&dm.provider_instance_id).first().map(|m| m.id.to_string()) {
                self.select_model(&dm.provider_instance_id, &first, cx);
            }
        }
        let access = self.default_permissions().unwrap_or(self.access);
        self.access = access;
        let workspace_mode = self.default_workspace();
        let chat_id = self.chats[self.active].id;
        let chat = &mut self.chats[self.active];
        chat.access = Some(access);
        match workspace_mode {
            crate::worktree::WorkspaceMode::Checkout => {
                chat.workdir = self.project.root().to_string_lossy().into_owned();
                chat.worktree = false;
            },
            crate::worktree::WorkspaceMode::Worktree => match crate::worktree::create(&self.project, chat_id) {
                Ok(dir) => {
                    chat.workdir = dir.to_string_lossy().into_owned();
                    chat.worktree = true;
                },
                Err(e) => {
                    chat.workdir = self.project.root().to_string_lossy().into_owned();
                    chat.worktree = false;
                    self.push_note(format!("**Worktree unavailable** — running in the project checkout.\n\n```\n{e}\n```"), cx);
                },
            },
        }
        self.stamp_thread();
    }

    /// Restore the selected chat's stamped provider/model/access into the
    /// workspace — the picker shows the active thread's own configuration.
    /// Legacy chats (empty stamps) leave the current selection alone.
    pub(crate) fn restore_thread_selection(&mut self, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(self.active) else { return };
        let provider = chat.provider.clone();
        let model = chat.model.clone();
        let access = chat.access;
        if !provider.is_empty()
            && self.providers.iter().any(|p| p.id == provider && p.enabled)
            && !self.select_model(&provider, &model, cx)
            && let Some(first) = self.models_for(&provider).first().map(|m| m.id.to_string())
        {
            self.select_model(&provider, &first, cx);
        }
        if let Some(access) = access {
            self.access = access;
        }
    }

    /// The `TurnContext` for the active chat's next backend turn — its
    /// worktree or the project root, plus its access mode. A chat bound to
    /// a past session carries its backend thread id so the turn resumes it.
    pub(crate) fn turn_context(&self) -> crate::backend::TurnContext {
        let chat = &self.chats[self.active];
        let mut ctx =
            crate::backend::TurnContext::at(crate::worktree::workdir_for(chat, self.project.root()), chat.access.unwrap_or(self.access));
        ctx.thread_id = if chat.thread_id.is_empty() { None } else { Some(chat.thread_id.clone()) };
        ctx
    }
}
