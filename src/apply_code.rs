//! Apply-to-file for code blocks: the Apply button on a non-shell fence
//! (shell blocks get Run instead — see `crate::run_cmd`) writes the block's
//! contents to a project file. The target resolves from a first-line path
//! hint (`// path: <file>`, `# <file>`, …) or a path in the language tag;
//! without one, a small picker over `project_files` asks, ranking files
//! whose extension matches the block's language first (see `resolve`).
//!
//! Overwriting an existing file always asks first — the block is agent
//! output, not a reviewed diff — and read-only threads (`!access.writes()`)
//! confirm every write. Both prompts reuse the transcript's approval card,
//! like a backend `fileChange` request. The write itself sits behind
//! `FileWriter` so tests inject a fake instead of touching the disk.

mod resolve;

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{IndexPath, WindowExt};
use gpui_kit::*;

use crate::backend::{ApprovalCard, ApprovalDecision, ApprovalKind, ApprovalResponder};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

pub(crate) use resolve::{apply_ranked, lang_extension, lang_path, path_hint, relative_path};

/// The Apply button on a non-shell code block — carries the block's code
/// and language tag. Dispatched on the window so the chat pane's
/// `on_action` runs it; `no_json` — it is never built from a keymap.
#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(no_json)]
pub struct ApplyCodeBlock {
    /// The code block's contents, written verbatim.
    pub code: String,
    /// The fence's language tag — a hint for the target file, never the
    /// target itself unless it parses as a path.
    pub lang: Option<String>,
}

/// A code block waiting on a target or an approval answer.
#[derive(Clone)]
struct PendingApply {
    /// The chat the Apply click came from — the result note lands there.
    chat_id: u64,
    code: String,
    /// The block's language tag — ranks the picker's file list.
    lang: Option<String>,
}

/// How the write actually happens — the real impl touches the disk; tests
/// substitute a fake via `set_file_writer`.
pub(crate) trait FileWriter: Send + Sync {
    fn exists(&self, root: &Path, rel: &str) -> bool;
    fn write(&self, root: &Path, rel: &str, content: &str) -> Result<(), String>;
}

/// Writes under `root`, creating parent directories for new files.
struct FsWriter;

impl FileWriter for FsWriter {
    fn exists(&self, root: &Path, rel: &str) -> bool {
        root.join(rel).is_file()
    }

    fn write(&self, root: &Path, rel: &str, content: &str) -> Result<(), String> {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, content).map_err(|e| e.to_string())
    }
}

/// The process-wide writer — swapped for a fake in tests.
static WRITER: std::sync::LazyLock<parking_lot::RwLock<Arc<dyn FileWriter>>> =
    std::sync::LazyLock::new(|| parking_lot::RwLock::new(Arc::new(FsWriter)));

/// The active writer — a fake in tests, `FsWriter` otherwise.
fn file_writer() -> Arc<dyn FileWriter> {
    WRITER.read().clone()
}

/// Install the writer used by every subsequent apply — tests only.
#[cfg(test)]
pub(crate) fn set_file_writer(writer: Arc<dyn FileWriter>) {
    *WRITER.write() = writer;
}

/// `request_ix` space for apply approval cards — counts down from
/// `usize::MAX / 2` so it can't collide with the backends' hashed item ids
/// or `run_cmd`'s local command-run cards.
static NEXT_APPLY_IX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX / 2);

/// A transcript message carrying an assistant text note.
fn note_message(text: String) -> ChatMessage {
    ChatMessage {
        role: Role::Assistant,
        kind: MessageKind::Text(text.into()),
        rating: None,
        usage: None,
        attachments: vec![],
        at: SystemTime::now(),
    }
}

/// A transcript message carrying the apply approval card.
fn approval_message(detail: String, respond: ApprovalResponder) -> ChatMessage {
    ChatMessage {
        role: Role::Assistant,
        kind: MessageKind::Approval(ApprovalCard {
            request_ix: NEXT_APPLY_IX.fetch_sub(1, std::sync::atomic::Ordering::Relaxed),
            kind: ApprovalKind::Patch,
            detail: detail.into(),
            decision: None,
            respond: Some(respond),
        }),
        rating: None,
        usage: None,
        attachments: vec![],
        at: SystemTime::now(),
    }
}

/// The picker's `Command` element — same shape as the Cmd-P palette, but a
/// pick applies the block instead of mentioning the file.
fn apply_command(
    state: &Entity<CommandState>, files: &[SharedString], apply: PendingApply, ws: &Entity<Workspace>, cx: &mut App,
) -> Command {
    let ws_confirm = ws.clone();
    let ws_query = ws.clone();
    let ext = lang_extension(apply.lang.as_deref());
    let ranked = apply_ranked(files, ext, &state.read(cx).query(cx));
    let group = CommandGroup::new()
        .label("Apply to file")
        .items(ranked.into_iter().map(|f| CommandItem::new().label(f)));
    Command::new(state)
        .placeholder("Apply code block to file…")
        .filterable(false)
        .group(group)
        .empty(|state, _, cx| {
            let hint = if state.query(cx).trim().is_empty() { "No files in this project" } else { "No matching files" };
            div().py_6().w_full().text_center().text_sm().text_color(cx.theme().muted_foreground).child(hint)
        })
        .footer(|_, _, cx| crate::palette::command_footer("↵ apply", cx))
        .on_query(move |_, _, cx| {
            ws_query.update(cx, |_, cx| cx.notify());
        })
        .on_confirm(move |path, window, cx| {
            ws_confirm.update(cx, |this, cx| this.confirm_apply_pick(path, apply.clone(), window, cx));
        })
        .on_cancel(|window, cx| window.close_dialog(cx))
}

impl Workspace {
    /// The Apply button on a non-shell code block: resolve the target file
    /// and write the block's contents. A path hint goes straight to the
    /// write; without one the file picker asks.
    pub(crate) fn apply_code_block(&mut self, code: String, lang: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let apply = PendingApply { chat_id: self.chats[self.active].id, code, lang };
        if let Some(rel) = path_hint(&apply.code).or_else(|| lang_path(apply.lang.as_deref())).and_then(|p| relative_path(&p)) {
            self.begin_apply(apply, rel, cx);
            return;
        }
        self.open_apply_picker(apply, window, cx);
    }

    /// No path hint — ask which file via a picker over `project_files`,
    /// extension-matching files first. An empty project can't offer a
    /// picker, so the note explains the hint instead.
    fn open_apply_picker(&mut self, apply: PendingApply, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            window.close_dialog(cx);
            return;
        }
        if self.project_files.is_empty() {
            self.push_note("**Apply:** no project files to pick from — add a `// path: <file>` hint to the code block.".into(), cx);
            return;
        }
        self.apply_palette.update(cx, |state, cx| state.set_query("", window, cx));
        let files = self.project_files.clone();
        let state = self.apply_palette.clone();
        let ws = cx.entity();
        window.open_dialog(cx, move |dialog, _window, cx| {
            dialog
                .close_button(false)
                .overlay_closable(true)
                .child(apply_command(&state, &files, apply.clone(), &ws, cx))
        });
        self.apply_palette.update(cx, |state, cx| state.focus(window, cx));
    }

    /// A picker row was confirmed — the chosen file becomes the target.
    /// Runs after the dialog closes so focus lands back on the transcript.
    /// The list re-ranks exactly as the picker showed it so `path.row`
    /// resolves to the row the user saw.
    fn confirm_apply_pick(&mut self, path: IndexPath, apply: PendingApply, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        let query = self.apply_palette.read(cx).query(cx);
        let files = self.project_files.clone();
        let ext = lang_extension(apply.lang.as_deref());
        let Some(rel) = apply_ranked(&files, ext, &query).get(path.row).cloned() else { return };
        self.begin_apply(apply, rel.to_string(), cx);
    }

    /// Write the block to `rel` under the chat's working directory. An
    /// existing file or a read-only thread gates the write behind an
    /// approval card; a new file in a writing mode lands directly.
    fn begin_apply(&mut self, apply: PendingApply, rel: String, cx: &mut Context<Self>) {
        let chat = &self.chats[self.active];
        let root = crate::worktree::workdir_for(chat, self.project.root());
        let access = self.effective_access(chat.access.unwrap_or(self.access));
        let exists = file_writer().exists(&root, &rel);
        if !exists && access.writes() || self.apply_approved {
            let result = file_writer().write(&root, &rel, &apply.code);
            self.finish_apply(apply.chat_id, &rel, result, cx);
            return;
        }
        let (respond, rx) = std::sync::mpsc::channel::<ApprovalDecision>();
        let detail = if exists { format!("Overwrite {rel}") } else { format!("Write {rel}") };
        Rc::make_mut(&mut self.chats[self.active].messages).push(approval_message(detail.clone(), respond));
        if self.push_visible(cx) {
            self.scroller.update(cx, |s, cx| s.append(1, cx));
        }
        self.record_approval(apply.chat_id, ApprovalKind::Patch, &detail);
        cx.notify();
        self.save();
        // Poll like `run_cmd` — a blocking recv would park the test
        // executor, which is the same clock the approval click needs.
        cx.spawn(async move |this, cx| {
            let decision = poll_apply_decision(&rx, cx).await;
            let _ = this.update(cx, |this, cx| this.answer_apply(apply, rel, decision, cx));
        })
        .detach();
    }

    /// Apply the approval card's answer: `Approve` writes, `ApproveForSession`
    /// also blesses later applies this session; `Deny` or a dropped
    /// responder leaves the card's recorded outcome as the only trace.
    fn answer_apply(&mut self, apply: PendingApply, rel: String, decision: Option<ApprovalDecision>, cx: &mut Context<Self>) {
        match decision {
            Some(ApprovalDecision::ApproveForSession) => self.apply_approved = true,
            Some(ApprovalDecision::Approve) => {},
            _ => return,
        }
        let root = self
            .chats
            .iter()
            .find(|c| c.id == apply.chat_id)
            .map_or_else(|| self.project.root().to_path_buf(), |c| crate::worktree::workdir_for(c, self.project.root()));
        let result = file_writer().write(&root, &rel, &apply.code);
        self.finish_apply(apply.chat_id, &rel, result, cx);
    }

    /// Land the write's outcome as a note on the chat the click came from —
    /// "Wrote `src/foo.rs`" or the failure. A chat that isn't active gets
    /// the unread dot instead of stealing focus.
    fn finish_apply(&mut self, chat_id: u64, rel: &str, result: Result<(), String>, cx: &mut Context<Self>) {
        let text = match result {
            Ok(()) => format!("Wrote `{rel}`"),
            Err(e) => format!("**Apply failed:** couldn't write `{rel}` — {e}"),
        };
        self.note_in(chat_id, text, cx);
    }

    /// `push_note` for a chat that may not be active — the apply can land
    /// after the user switched threads.
    fn note_in(&mut self, chat_id: u64, text: String, cx: &mut Context<Self>) {
        let ix = self.chat_index(chat_id).unwrap_or(self.active);
        let is_active = ix == self.active;
        let chat = &mut self.chats[ix];
        chat.last_turn = None;
        Rc::make_mut(&mut chat.messages).push(note_message(text));
        if is_active {
            if self.push_visible(cx) {
                self.scroller.update(cx, |s, cx| s.append(1, cx));
            }
        } else {
            chat.unread = true;
        }
        crate::dock_badge::update(cx);
        cx.notify();
        self.save();
    }
}

/// Poll the approval channel until a decision lands or the responder drops.
/// `Some(d)` = answered; `None` = responder dropped (turn stopped, chat
/// reloaded).
async fn poll_apply_decision(rx: &std::sync::mpsc::Receiver<ApprovalDecision>, cx: &mut gpui_kit::AsyncApp) -> Option<ApprovalDecision> {
    use std::sync::mpsc::TryRecvError::{Disconnected, Empty};
    loop {
        match rx.try_recv() {
            Ok(d) => return Some(d),
            Err(Disconnected) => return None,
            Err(Empty) => cx.background_executor().timer(Duration::from_millis(30)).await,
        }
    }
}
