//! Snapshots-panel actions on `Workspace`: toggle/refresh/land, the
//! expanded row's lazy file list, and restore/delete. Split from
//! `snapshots.rs` for the SLOC cap — the list types and pure
//! collect/prune live there.

use gpui_kit::*;

use crate::snapshots::{SnapshotFiles, SnapshotInfo, cap_bytes, collect, prune, retention_age, seeds};
use crate::workspace::Workspace;

impl Workspace {
    /// Toggle the Snapshots panel; opening refreshes the list so the first
    /// render never shows stale rows.
    pub fn toggle_snapshots_panel(&mut self, cx: &mut Context<Self>) {
        self.snapshots.open = !self.snapshots.open;
        if self.snapshots.open {
            self.refresh_snapshots(cx);
        }
        cx.notify();
    }

    /// Re-collect snapshot metadata on the background executor — each entry
    /// shells out to git or walks a copy dir, so it can't run on the UI
    /// thread. Lands through `land_snapshots`, which applies retention.
    pub fn refresh_snapshots(&mut self, cx: &mut Context<Self>) {
        self.snapshots.generation += 1;
        let generation = self.snapshots.generation;
        let seeds = seeds(&self.chats, self.project.root());
        cx.spawn(async move |this, cx| {
            let list = cx.background_executor().spawn(async move { collect(&seeds) }).await;
            let _ = this.update(cx, |this, cx| this.land_snapshots(generation, list, cx));
        })
        .detach();
    }

    /// Publish a collected list — skipped when a newer refresh was requested
    /// while this one ran. Retention prunes before the list lands so stale
    /// snapshots never render; a prune that removed anything persists the
    /// shrunken checkpoint lists.
    pub(crate) fn land_snapshots(&mut self, generation: u64, list: Vec<SnapshotInfo>, cx: &mut Context<Self>) {
        if generation != self.snapshots.generation {
            return;
        }
        let victims = prune(&list, retention_age(self.snapshots.retention_days), cap_bytes(self.snapshots.cap_mb));
        let mut removed = false;
        let kept: Vec<SnapshotInfo> = list
            .into_iter()
            .filter(|s| {
                if !victims.contains(s) {
                    return true;
                }
                let dir = if s.workdir.is_dir() { s.workdir.clone() } else { self.project.root().to_path_buf() };
                if crate::snapshot_store::delete(&dir, &s.checkpoint).is_ok() {
                    removed |= self.unpin_checkpoint(s);
                    return false;
                }
                true
            })
            .collect();
        self.snapshots.list = kept;
        if removed {
            self.save();
        }
        cx.notify();
    }

    /// Expand/collapse a row's file list. Expanding computes the paths
    /// restore would touch on the background executor — the same diff
    /// `describe` counted — and caches them on the row; collapsing keeps
    /// the cache so re-expanding is instant.
    pub fn expand_snapshot(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.snapshots.list.get_mut(ix) else { return };
        row.expanded = !row.expanded;
        let expanded = row.expanded;
        if expanded {
            self.start_files_load(ix, cx);
        }
        cx.notify();
    }

    /// Issue the background path-diff for row `ix` — skipped when the list
    /// is already loaded or in flight (a double-expand can't double-spawn).
    fn start_files_load(&mut self, ix: usize, cx: &mut Context<Self>) {
        let generation = self.snapshots.generation;
        let Some(row) = self.snapshots.list.get_mut(ix) else { return };
        if !matches!(row.files, SnapshotFiles::Idle | SnapshotFiles::Failed) {
            return;
        }
        row.files = SnapshotFiles::Loading;
        let key = row.key();
        let workdir = row.workdir.clone();
        let checkpoint = row.checkpoint.clone();
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_executor()
                .spawn(async move { crate::snapshot_store::describe_files(&workdir, &checkpoint).1 })
                .await;
            let _ = this.update(cx, |this, cx| this.land_snapshot_files(generation, key, files, cx));
        })
        .detach();
    }

    /// Store a loaded file list on the row `key` names — skipped when a
    /// refresh landed while it ran (the generation moved on) or the row is
    /// gone. A fresh list also refreshes `changed` so the count and the
    /// expanded rows never disagree.
    pub(crate) fn land_snapshot_files(
        &mut self, generation: u64, key: (u64, usize, std::time::SystemTime), files: Option<Vec<crate::snapshot_store::SnapshotFile>>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.snapshots.generation {
            return;
        }
        let Some(row) = self.snapshots.list.iter_mut().find(|s| s.key() == key) else { return };
        match files {
            Some(files) => {
                row.changed = Some(files.len());
                row.files = SnapshotFiles::Loaded(files);
            },
            None => row.files = SnapshotFiles::Failed,
        }
        cx.notify();
    }

    /// Revert the snapshot's chat workdir to it — the same restore the
    /// per-message "Undo turn" runs, reachable from the panel. A clean
    /// snapshot (0 changed files) restores on one click; anything else
    /// confirms first, naming the count and the first few paths. The entry
    /// stays listed: snapshots are a history, not a queue.
    pub fn restore_snapshot(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snap) = self.snapshots.list.get_mut(ix) else { return };
        if matches!(snap.files, SnapshotFiles::Idle | SnapshotFiles::Failed) {
            // The confirm names paths — compute them now (a `git diff` or
            // dir walk, the same cost a restore pays) and fill the cache
            // so a later expand doesn't recompute.
            let files = crate::snapshot_store::describe_files(&snap.workdir, &snap.checkpoint).1;
            snap.files = match files {
                Some(files) => {
                    snap.changed = Some(files.len());
                    SnapshotFiles::Loaded(files)
                },
                None => SnapshotFiles::Failed,
            };
        }
        let snap = snap.clone();
        let files = match &snap.files {
            SnapshotFiles::Loaded(files) => Some(files.as_slice()),
            _ => None,
        };
        if files.is_some_and(|f| f.is_empty()) {
            self.run_snapshot_restore(&snap, cx);
            return;
        }
        let title = match files {
            Some(files) => format!("Restore {} {}?", files.len(), if files.len() == 1 { "file" } else { "files" }),
            None => "Restore this snapshot?".to_string(),
        };
        let detail = files.map(|files| {
            let mut names: Vec<&str> = files.iter().take(5).map(|f| f.path.as_str()).collect();
            if files.len() > 5 {
                names.push("…");
            }
            names.join(", ")
        });
        let rx = window.prompt(
            PromptLevel::Warning,
            &title,
            detail.as_deref(),
            &[PromptButton::ok("Restore"), PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| this.run_snapshot_restore(&snap, cx));
        })
        .detach();
    }

    /// The restore itself, once any confirm is answered — the workdir and
    /// checkpoint come from `snap`, so a list refresh can't redirect it.
    fn run_snapshot_restore(&mut self, snap: &SnapshotInfo, cx: &mut Context<Self>) {
        match crate::checkpoints::restore(&snap.workdir, &snap.checkpoint) {
            Ok(()) => {
                if self.changes_panel_open {
                    self.refresh_changes(cx);
                }
                self.refresh_snapshots(cx);
            },
            Err(e) => self.push_note(format!("**Snapshot restore failed:** {e}"), cx),
        }
    }

    /// Drop the snapshot at row `ix`: free its storage, unpin the
    /// checkpoint from its chat (persisted), and refresh the list.
    pub fn delete_snapshot(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(snap) = self.snapshots.list.get(ix) else { return };
        let snap = snap.clone();
        let dir = if snap.workdir.is_dir() { snap.workdir.clone() } else { self.project.root().to_path_buf() };
        match crate::snapshot_store::delete(&dir, &snap.checkpoint) {
            Ok(()) => {
                if self.unpin_checkpoint(&snap) {
                    self.save();
                }
                self.refresh_snapshots(cx);
            },
            Err(e) => self.push_note(format!("**Snapshot delete failed:** {e}"), cx),
        }
    }

    /// Remove `snap`'s `TurnCheckpoint` from its chat — the entry the
    /// per-message "Undo turn" looks up. `false` when nothing matched.
    fn unpin_checkpoint(&mut self, snap: &SnapshotInfo) -> bool {
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == snap.chat_id) else { return false };
        let before = chat.checkpoints.len();
        chat.checkpoints.retain(|c| !(c.ix == snap.message_ix && c.at == snap.at));
        chat.checkpoints.len() != before
    }

    /// Set the age half of the retention policy (days; `0` = forever),
    /// persist it, and re-run collection so the new rule applies now.
    pub fn set_snapshot_retention(&mut self, days: u32, cx: &mut Context<Self>) {
        self.snapshots.retention_days = days;
        self.save_settings();
        self.refresh_snapshots(cx);
    }

    /// Set the size half of the retention policy (MiB; `0` = no cap),
    /// persist it, and re-run collection so the new rule applies now.
    pub fn set_snapshot_cap(&mut self, mb: u32, cx: &mut Context<Self>) {
        self.snapshots.cap_mb = mb;
        self.save_settings();
        self.refresh_snapshots(cx);
    }
}
