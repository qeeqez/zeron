//! Headless tests for slash-command dispatch: each command's effect on the
//! workspace, the unknown-command note, and `/init`'s canned prompt reaching
//! the backend (via a recording fake — no real subprocess).
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::backend::{AgentBackend, AgentEvent, ReplyStream, TurnContext};
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// A backend that records each prompt instead of spawning — the `/init`
/// assertion without a real subprocess.
struct PromptBackend {
    prompts: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
}

impl AgentBackend for PromptBackend {
    fn name(&self) -> &'static str {
        "rec"
    }

    fn send(&self, prompt: &str, _model: &str, _mode: &str, _ctx: &TurnContext) -> ReplyStream {
        self.prompts.lock().push(prompt.to_string());
        let (tx, events) = std::sync::mpsc::channel();
        let _ = tx.send(AgentEvent::Done);
        ReplyStream {
            events,
            child: None,
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a temp
/// dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-send-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

/// Type `text` into the composer and send it through the real `send` path.
fn submit(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.composer.update(cx, |s, cx| s.set_value(text, window, cx));
            ws.send(window, cx);
        });
    });
}

/// One seeded text message — `i` alternates the role and names the text.
fn seeded(i: usize) -> ChatMessage {
    ChatMessage {
        role: if i.is_multiple_of(2) { Role::User } else { Role::Assistant },
        kind: MessageKind::Text(format!("message {i}").into()),
        rating: None,
        bookmarked: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

/// Seed the active chat with `n` alternating user/assistant text messages.
fn seed(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, n: usize) {
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            let chat = &mut ws.chats[ws.active];
            for i in 0..n {
                std::rc::Rc::make_mut(&mut chat.messages).push(seeded(i));
            }
        });
    });
}

/// Text of every `Text` message in the active chat, in order.
fn texts(ws: &Workspace) -> Vec<String> {
    ws.chats[ws.active]
        .messages
        .iter()
        .filter_map(|m| match &m.kind {
            MessageKind::Text(t) => Some(t.to_string()),
            _ => None,
        })
        .collect()
}

#[gpui_kit::test]
fn clear_empties_active_chat(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    seed(&workspace, cx, 6);
    let chat_id = workspace.read_with(cx, |ws, _| ws.chats[ws.active].id);
    submit(&workspace, cx, "/clear");
    workspace.read_with(cx, |ws, _| {
        assert!(ws.chats[ws.active].messages.is_empty(), "/clear must empty the transcript");
        assert_eq!(ws.chats[ws.active].id, chat_id, "/clear must not delete the chat itself");
        assert_eq!(ws.chats.len(), 1);
    });
}

#[gpui_kit::test]
fn compact_folds_prefix_into_summary(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    seed(&workspace, cx, 8);
    submit(&workspace, cx, "/compact");
    workspace.read_with(cx, |ws, _| {
        let msgs = texts(ws);
        // Summary + 4 kept verbatim + the "Compacted" note.
        assert_eq!(msgs.len(), 6);
        assert!(msgs[0].contains("Compacted context"), "dropped prefix must become a digest");
        assert!(msgs[0].contains("message 0"), "digest must cover the dropped messages");
        assert!(msgs[0].contains("message 3"));
        assert!(!msgs[0].contains("message 4"), "kept messages stay out of the digest");
        assert_eq!(msgs[1], "message 4");
        assert_eq!(msgs[4], "message 7");
        assert!(msgs[5].contains("Compacted"));
    });
}

#[gpui_kit::test]
fn compact_short_transcript_notes_nothing_to_do(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    seed(&workspace, cx, 3);
    submit(&workspace, cx, "/compact");
    workspace.read_with(cx, |ws, _| {
        let msgs = texts(ws);
        assert_eq!(msgs.len(), 4, "short transcript keeps its messages plus the note");
        assert!(msgs[3].contains("Nothing to compact"));
    });
}

#[gpui_kit::test]
fn init_sends_codebase_prompt(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    let prompts = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(PromptBackend { prompts: prompts.clone() });
            ws.model = "test-model".into();
        });
    });
    submit(&workspace, cx, "/init");
    cx.run_until_parked();
    let sent = prompts.lock().clone();
    assert_eq!(sent.len(), 1, "/init must send exactly one backend turn");
    assert!(sent[0].contains("AGENTS.md"), "prompt must ask for AGENTS.md: {}", sent[0]);
    assert!(sent[0].contains("Analyze"), "prompt must ask for codebase analysis: {}", sent[0]);
    workspace.read_with(cx, |ws, _| {
        let msgs = texts(ws);
        assert!(msgs.iter().any(|t| t.contains("AGENTS.md")), "the prompt should appear as a user message");
    });
}

#[gpui_kit::test]
fn status_reports_provider_model_access(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.model = "test-model".into();
        });
    });
    let (provider, backend, access) =
        workspace.read_with(cx, |ws, _| (ws.selected_provider.clone(), ws.backend.name(), ws.access.name().to_string()));
    submit(&workspace, cx, "/status");
    workspace.read_with(cx, |ws, _| {
        let note = texts(ws).pop().unwrap_or_default();
        assert!(note.contains(&format!("`{provider}`")), "status must name the provider: {note}");
        assert!(note.contains(backend), "status must name the backend: {note}");
        assert!(note.contains("`test-model`"), "status must name the model: {note}");
        assert!(note.contains(&access), "status must name the access mode: {note}");
        assert!(note.contains("workspace:"), "status must show the workspace: {note}");
    });
}

#[gpui_kit::test]
fn help_lists_every_command(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    submit(&workspace, cx, "/help");
    workspace.read_with(cx, |ws, _| {
        let note = texts(ws).pop().unwrap_or_default();
        for (cmd, _) in crate::slash::SLASH_COMMANDS {
            assert!(note.contains(&format!("`/{cmd}`")), "/help must list /{cmd}: {note}");
        }
    });
}

#[gpui_kit::test]
fn unknown_command_notes_instead_of_sending(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    let prompts = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(PromptBackend { prompts: prompts.clone() });
            ws.model = "test-model".into();
        });
    });
    submit(&workspace, cx, "/bogus");
    cx.run_until_parked();
    assert!(prompts.lock().is_empty(), "unknown command must not reach the backend");
    workspace.read_with(cx, |ws, _| {
        let msgs = texts(ws);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("Unknown command `/bogus`"), "expected a helpful note: {}", msgs[0]);
        assert!(msgs[0].contains("/help"));
        // No user message — the command was consumed, not sent.
        assert!(!ws.chats[ws.active].messages.iter().any(|m| m.role == Role::User));
    });
}

#[gpui_kit::test]
fn slash_looking_path_still_sends(cx: &mut TestAppContext) {
    let (workspace, cx) = mount(cx);
    let prompts = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.backend = std::sync::Arc::new(PromptBackend { prompts: prompts.clone() });
            ws.model = "test-model".into();
        });
    });
    submit(&workspace, cx, "/tmp/file.rs");
    cx.run_until_parked();
    assert_eq!(prompts.lock().as_slice(), &["/tmp/file.rs".to_string()], "a path is text, not a command");
}
