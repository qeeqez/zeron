//! Lazy transcript loading: `load_chats` restores metadata only, the first
//! open hydrates the real messages, a pending save never writes the empty
//! placeholder over history, and the aggregate surfaces (bookmarks, usage,
//! search) still see unopened chats.

use std::rc::Rc;

use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{Chat, ChatMessage, MessageKind, Role, ToolCall, ToolStatus};
use crate::persist::{load_chats, save_chats};
use crate::workspace::Workspace;

/// A text message with the field set `ChatMessage` literals carry.
pub(super) fn msg(text: &str) -> ChatMessage {
    ChatMessage {
        alternatives: vec![],
        role: Role::Assistant,
        kind: MessageKind::Text(text.into()),
        rating: None,
        bookmarked: false,
        pinned: false,
        usage: None,
        attachments: vec![],
        at: std::time::SystemTime::now(),
    }
}

pub(super) fn seeded_chat(id: u64, title: &str, messages: Vec<ChatMessage>) -> Chat {
    let mut chat = Chat::new(id, title);
    chat.messages = Rc::new(messages);
    chat
}

pub(super) fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-persist-load-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn sandbox_home() {
    let dir = std::env::temp_dir().join(format!("rixlcode-lazy-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread
    // can observe HOME mid-write.
    unsafe { std::env::set_var("HOME", &dir) };
}

/// Mount a `Workspace` on the seeded store — the "launch" half of a
/// restart; chats restore metadata-only until opened.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let window = cx.open_window(gpui_kit::size(gpui_kit::px(1024.), gpui_kit::px(768.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    (workspace.unwrap(), cx)
}

/// Run the debounce + background scan behind `refresh_pending_bookmarks`.
fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
}

#[test]
fn load_leaves_transcripts_pending() {
    let dir = temp_dir("pending");
    save_chats(&dir, &[seeded_chat(0, "alpha", vec![msg("one")]), seeded_chat(1, "beta", vec![msg("two")])]);
    let mut next_id = 0;
    let loaded = load_chats(&dir, &mut next_id, false);
    assert_eq!(loaded.len(), 2);
    for chat in &loaded {
        assert!(chat.pending_load.is_some(), "transcript stays on disk until first open");
        assert!(chat.messages.is_empty(), "metadata-only load carries no messages");
    }
    assert_eq!(loaded[0].title, "alpha");
    assert_eq!(loaded[1].title, "beta");
}

#[test]
fn hydrate_restores_the_transcript() {
    let dir = temp_dir("hydrate");
    save_chats(&dir, &[seeded_chat(0, "alpha", vec![msg("one"), msg("two")])]);
    let mut next_id = 0;
    let mut loaded = load_chats(&dir, &mut next_id, false);
    assert!(crate::persist::hydrate_chat(&mut loaded[0], &dir));
    assert!(loaded[0].pending_load.is_none());
    assert_eq!(loaded[0].messages.len(), 2);
}

#[test]
fn cold_start_marks_saved_running_tools_failed() {
    let dir = temp_dir("recover");
    let mut tool = msg("call");
    tool.kind = MessageKind::Tool(ToolCall {
        tool_ix: 0,
        name: "shell".into(),
        detail: "make".into(),
        output: "".into(),
        status: ToolStatus::Running,
        expanded: false,
    });
    save_chats(&dir, &[seeded_chat(0, "alpha", vec![tool])]);
    let mut next_id = 0;
    // Cold start: no live window can own the turn — recovery marks it.
    let mut loaded = load_chats(&dir, &mut next_id, true);
    crate::persist::hydrate_all(&mut loaded, &dir);
    assert!(matches!(&loaded[0].messages[0].kind, MessageKind::Tool(t) if t.status == ToolStatus::Failed));
}

#[test]
fn warm_load_keeps_running_tools_running() {
    let dir = temp_dir("warm");
    let mut tool = msg("call");
    tool.kind = MessageKind::Tool(ToolCall {
        tool_ix: 0,
        name: "shell".into(),
        detail: "make".into(),
        output: "".into(),
        status: ToolStatus::Running,
        expanded: false,
    });
    save_chats(&dir, &[seeded_chat(0, "alpha", vec![tool])]);
    let mut next_id = 0;
    let mut loaded = load_chats(&dir, &mut next_id, false);
    crate::persist::hydrate_all(&mut loaded, &dir);
    assert!(matches!(&loaded[0].messages[0].kind, MessageKind::Tool(t) if t.status == ToolStatus::Running));
}

#[test]
fn slot_shift_still_hydrates() {
    let dir = temp_dir("shift");
    save_chats(&dir, &[seeded_chat(0, "alpha", vec![msg("one")]), seeded_chat(1, "beta", vec![msg("two")])]);
    let mut next_id = 0;
    let mut loaded = load_chats(&dir, &mut next_id, false);
    // Another window rewrote the slots: `alpha` and `beta` traded file
    // positions. The slot hint now points at the wrong transcript, so the
    // created_at rescan must find `alpha` at slot 1.
    std::fs::rename(dir.join("0.json"), dir.join("tmp.json")).unwrap();
    std::fs::rename(dir.join("1.json"), dir.join("0.json")).unwrap();
    std::fs::rename(dir.join("tmp.json"), dir.join("1.json")).unwrap();
    assert!(crate::persist::hydrate_chat(&mut loaded[0], &dir));
    assert_eq!(loaded[0].messages.len(), 1);
    assert!(matches!(&loaded[0].messages[0].kind, MessageKind::Text(t) if t.as_str() == "one"));
}

#[test]
fn saving_a_pending_chat_keeps_its_history() {
    let dir = temp_dir("pending-save");
    save_chats(&dir, &[seeded_chat(0, "alpha", vec![msg("one"), msg("two")])]);
    let mut next_id = 0;
    let mut loaded = load_chats(&dir, &mut next_id, false);
    // A metadata-only edit must not write the empty placeholder over the
    // on-disk transcript.
    loaded[0].draft = "wip".to_string();
    save_chats(&dir, &loaded);
    let stored = crate::persist::read_stored(&dir.join("0.json")).unwrap();
    assert_eq!(stored.messages.len(), 2, "the deferred save preserved history");
    assert_eq!(stored.draft, "wip", "the metadata edit still persisted");
}

#[test]
fn selecting_a_chat_hydrates_it() {
    sandbox_home();
    let project = crate::project::Project::current();
    save_chats(&project.chats_dir(), &[seeded_chat(0, "first", vec![msg("earlier question")]), seeded_chat(1, "second", vec![])]);
    project.save_state(&crate::project::ProjectState { active_chat: 1, ..Default::default() });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            assert!(this.chats[0].pending_load.is_some(), "the inactive chat stays pending");
            this.select_chat(0, window, cx);
            assert_eq!(this.chats[0].messages.len(), 1, "selecting hydrated the transcript");
            assert!(this.chats[0].pending_load.is_none());
        });
    });
}

#[test]
fn bookmark_count_includes_unopened_chats() {
    sandbox_home();
    let project = crate::project::Project::current();
    let mut starred = msg("starred in an unopened chat");
    starred.bookmarked = true;
    save_chats(&project.chats_dir(), &[seeded_chat(0, "starred chat", vec![starred]), seeded_chat(1, "plain", vec![])]);
    project.save_state(&crate::project::ProjectState { active_chat: 1, ..Default::default() });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    settle(cx);
    ws.read_with(cx, |this, _| {
        assert!(this.chats[0].pending_load.is_some());
        assert_eq!(this.bookmark_count(), 1, "the badge counts stars still on disk");
    });
    // Opening the chat moves its star to the live half — the total is
    // unchanged, never double-counted.
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.select_chat(0, window, cx);
            assert_eq!(this.bookmark_count(), 1, "hydration moved the star, not doubled it");
        });
    });
}

#[test]
fn bookmarks_panel_lists_unopened_chats() {
    sandbox_home();
    let project = crate::project::Project::current();
    let mut starred = msg("starred in an unopened chat");
    starred.bookmarked = true;
    save_chats(&project.chats_dir(), &[seeded_chat(0, "starred chat", vec![starred]), seeded_chat(1, "plain", vec![])]);
    project.save_state(&crate::project::ProjectState { active_chat: 1, ..Default::default() });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.toggle_bookmarks_panel(cx);
            let groups = this.bookmark_groups();
            assert_eq!(groups.len(), 1, "opening the panel surfaces unopened chats' stars");
            assert_eq!(groups[0].title, "starred chat");
            assert_eq!(groups[0].rows.len(), 1);
        });
    });
}

#[test]
fn usage_totals_include_unopened_chats() {
    sandbox_home();
    let project = crate::project::Project::current();
    let mut spent = msg("turn with recorded tokens");
    spent.usage = Some(crate::model::Usage { input: 10, output: 20 });
    save_chats(&project.chats_dir(), &[seeded_chat(0, "spent chat", vec![spent]), seeded_chat(1, "plain", vec![])]);
    project.save_state(&crate::project::ProjectState { active_chat: 1, ..Default::default() });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            let before = this.usage_totals().by_day.iter().map(|d| d.tokens).sum::<u64>();
            assert_eq!(before, 0, "a pending transcript contributes nothing until opened");
            this.toggle_usage_panel(window, cx);
            let after = this.usage_totals().by_day.iter().map(|d| d.tokens).sum::<u64>();
            assert_eq!(after, 30, "opening the panel totals the unopened chat's stamps");
        });
    });
}

#[test]
fn search_docs_skip_a_hydrated_chats_drifted_file() {
    sandbox_home();
    let project = crate::project::Project::current();
    save_chats(&project.chats_dir(), &[seeded_chat(0, "opened chat", vec![msg("needle")]), seeded_chat(1, "plain", vec![])]);
    project.save_state(&crate::project::ProjectState { active_chat: 0, ..Default::default() });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // Another window's longer slot map left a copy of an already-open
    // chat's file past the loaded set — it must not become a disk-only
    // doc whose click loads a duplicate chat.
    std::fs::copy(project.chats_dir().join("0.json"), project.chats_dir().join("7.json")).unwrap();
    ws.read_with(cx, |this, _| {
        let docs = this.search_docs();
        assert!(!docs.iter().any(|d| d.chat_id.is_none() && d.title.as_ref() == "opened chat"));
    });
}

#[test]
fn search_docs_find_pending_chats() {
    sandbox_home();
    let project = crate::project::Project::current();
    save_chats(&project.chats_dir(), &[seeded_chat(0, "needle chat", vec![msg("the needle is here")]), seeded_chat(1, "plain", vec![])]);
    project.save_state(&crate::project::ProjectState { active_chat: 1, ..Default::default() });
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    ws.read_with(cx, |this, _| {
        let pending_id = this.chats[0].id;
        let docs = this.search_docs();
        let doc = docs.iter().find(|d| d.chat_id == Some(pending_id)).expect("pending chat searched via its file");
        assert!(doc.messages.iter().any(|m| matches!(&m.kind, MessageKind::Text(t) if t.contains("needle"))));
    });
}
