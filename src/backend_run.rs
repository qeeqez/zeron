use std::time::Duration;

use gpui_kit::*;

use crate::backend::{AgentEvent, ReplyStream};
use crate::workspace::Workspace;

/// Drive a real `AgentBackend` reply: spawn the backend, pump its event
/// stream on a thread, and apply events on the UI thread via a channel.
pub fn run_backend(this: &mut Workspace, prompt: &str, cx: &mut Context<Workspace>) {
    let chat_id = this.chats[this.active].id;
    // A signed-out provider can't take a turn — surface the sign-in
    // prompt instead of the backend's opaque auth error.
    if let Some(reason) = this.auth_block_note() {
        this.push_note(format!("**Error:** {reason}"), cx);
        this.finish_reply(chat_id, cx);
        return;
    }
    let model = this.model.to_string();
    let mode = this.mode.to_string();
    // The thread's workdir (project root or its worktree) and access mode
    // travel with the turn — a mid-turn settings change can't alter them.
    let ctx = this.turn_context();
    // Snapshot the workdir before the backend can touch it — the turn's
    // "Undo" restores this checkpoint.
    this.record_turn_checkpoint(chat_id, &ctx.cwd);
    let mut stream = this.backend.send(prompt, &model, &mode, &ctx);
    this.spawn_run_agent(crate::agents::RunAgentSpec { chat_id, name: this.backend.name(), lane: &model }, cx);
    drive_stream(this, chat_id, &mut stream, cx);
    if let Some(chat) = this.chats.iter_mut().find(|c| c.id == chat_id) {
        chat.stream = Some(stream);
    }
}

/// Drive a backend compaction turn on the active chat: no user message,
/// no checkpoint — compaction only folds the thread's server-side history.
/// Returns false when the backend can't compact this chat (no bound
/// thread); the caller falls back to a summarization prompt.
pub fn run_compact(this: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    let chat_id = this.chats[this.active].id;
    if let Some(reason) = this.auth_block_note() {
        this.push_note(format!("**Error:** {reason}"), cx);
        this.finish_reply(chat_id, cx);
        return true;
    }
    let ctx = this.turn_context();
    let Some(mut stream) = this.backend.compact(&ctx) else { return false };
    this.spawn_run_agent(crate::agents::RunAgentSpec { chat_id, name: this.backend.name(), lane: "compact" }, cx);
    let chat = &mut this.chats[this.active];
    chat.running = true;
    chat.failed_flag = false;
    chat.started_at = Some(std::time::Instant::now());
    this.clear_recall();
    drive_stream(this, chat_id, &mut stream, cx);
    if let Some(chat) = this.chats.iter_mut().find(|c| c.id == chat_id) {
        chat.stream = Some(stream);
    }
    true
}

/// Pump a turn's event stream into the chat: forward whatever the backend
/// already queued, drain the rest on a blocking thread, and apply events
/// on the UI thread until `Done` or disconnect. The stream lands on the
/// chat so stop/delete drop it (killing the child).
fn drive_stream(this: &mut Workspace, chat_id: u64, stream: &mut ReplyStream, cx: &mut Context<Workspace>) {
    // The pump needs the receiver; the stream itself lands on the chat so
    // stop/delete drop it (killing the child, setting `cancelled`) and the
    // composer can steer into the turn. A dummy receiver stands in — the
    // chat never reads events.
    let (dead_tx, events) = std::sync::mpsc::channel();
    let events = std::mem::replace(&mut stream.events, events);
    drop(dead_tx);
    // The task's future owns this guard: dropping the task (stop, chat
    // delete, quit) drops the future and sets `cancelled` right away —
    // the pump thread's stream drop only fires once it wakes on an event.
    let cancel = stream.cancel_guard();
    let (tx, rx) = std::sync::mpsc::channel::<AgentEvent>();
    // Forward whatever the backend already queued before handing the
    // receiver to the pump thread: a stub that sends its whole stream
    // inside `send` must not depend on the OS scheduling the thread —
    // under parallel test load that scheduling is what flakes.
    let mut live = true;
    while let Ok(e) = events.try_recv() {
        if tx.send(e).is_err() {
            live = false;
            break;
        }
    }
    if live {
        std::thread::spawn(move || pump_stream(events, tx));
    }

    let task = cx.spawn(async move |this, cx| {
        let _cancel = cancel;

        'outer: loop {
            let e = match rx.try_recv() {
                Ok(e) => e,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    cx.background_executor().timer(Duration::from_millis(30)).await;
                    continue;
                },
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            };
            // Only Done ends the turn — item-level errors are non-terminal
            // (codex continues), and every other exit path closes the
            // channel, which surfaces as Disconnected.
            if matches!(e, AgentEvent::Done) {
                break 'outer;
            }
            let _ = this.update(cx, |this, cx| this.apply_event(chat_id, e, cx));
        }
        let _ = this.update_in(cx, |this, window, cx| {
            this.finish_reply(chat_id, cx);
            // A clean turn lifts the rate-limit banner — quota windows
            // stay on the usage popover. A failed turn keeps it.
            if let Some(chat) = this.chats.iter_mut().find(|c| c.id == chat_id && !c.failed_flag) {
                chat.usage.clear_limited();
            }
            this.notify_done(chat_id, window, cx);
        });
    });
    if let Some(chat) = this.chats.iter_mut().find(|c| c.id == chat_id) {
        chat.reply_task = Some(task);
        // A fresh turn starts the meter's per-turn counter over.
        chat.usage.begin_turn();
    }
}

/// Drain the backend event channel into `tx` on a blocking thread.
/// `recv` returns `Err` when the producer exits. The `ReplyStream` itself
/// lives on the chat — dropping it there kills the child.
fn pump_stream(events: std::sync::mpsc::Receiver<AgentEvent>, tx: std::sync::mpsc::Sender<AgentEvent>) {
    while let Ok(e) = events.recv() {
        if tx.send(e).is_err() {
            break;
        }
    }
}
