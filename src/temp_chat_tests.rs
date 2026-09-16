//! Temporary chats: `Chat.ephemeral` keeps a thread off disk — `save_chats`
//! skips it, the stale sweep removes any file in its slot, and it stays out
//! of search docs, snapshot seeds, the send queue and the activity feed.
//! Deleting the last chat is normally a no-op, but a last *temporary* chat
//! is replaced by a fresh normal one.

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Role as A11yRole, TestAppContext, VisualTestContext};

use crate::model::Chat;
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount<'a>(cx: &'a mut TestAppContext, name: &str) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-tempchat-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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

/// A fresh throwaway chats dir — `save_chats`/`load_chats` take the dir
/// explicitly, so these tests never touch the real profile.
fn temp_chats_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-tempchat-persist-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn temp_chat(id: u64) -> Chat {
    let mut chat = Chat::new(id, "temp");
    chat.ephemeral = true;
    chat
}

/// The `Workspace` entities of every open window except `known`.
fn other_workspaces(app: &mut TestAppContext, known: &Entity<Workspace>) -> Vec<Entity<Workspace>> {
    let windows = app.read(|cx| cx.windows());
    let mut found = Vec::new();
    for handle in windows {
        let _ = handle.update(app, |root, _window, cx| {
            if let Some(ws) = root.downcast::<Root>().ok().and_then(|r| r.read(cx).view().clone().downcast::<Workspace>().ok())
                && ws != *known
            {
                found.push(ws);
            }
        });
    }
    found
}

#[test]
fn temp_chat_never_reaches_disk() {
    let dir = temp_chats_dir("skip");
    crate::persist::save_chats(&dir, &[Chat::new(0, "normal"), temp_chat(1)]);
    assert!(dir.join("0.json").exists(), "normal chat persists in its slot");
    assert!(!dir.join("1.json").exists(), "temporary chat writes no file");
    let mut next_id = 0;
    let loaded = crate::persist::load_chats(&dir, &mut next_id, true);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].title, "normal");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn temp_chat_slot_cant_resurrect() {
    let dir = temp_chats_dir("sweep");
    // A file in the temporary chat's slot — a stale write from before the
    // chat went ephemeral — is swept like a deleted chat's file.
    std::fs::write(dir.join("1.json"), r#"{"v":1,"title":"ghost","messages":[]}"#).unwrap();
    crate::persist::save_chats(&dir, &[Chat::new(0, "normal"), temp_chat(1)]);
    assert!(!dir.join("1.json").exists(), "the stale file in a temp slot is removed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_temp_chat_saves_nothing() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "save");
    let chats_dir = cx.update(|_, cx| ws.read(cx).project.chats_dir());
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_temp_chat(cx);
            assert!(this.chats[this.active].ephemeral, "new_temp_chat marks the chat");
            assert_eq!(this.chats[this.active].title, "Temporary chat");
        });
    });
    // The initial normal chat owns slot 0; the temporary chat's slot stays
    // empty even though `save` ran.
    assert!(chats_dir.join("0.json").exists(), "the normal chat persists");
    assert!(!chats_dir.join("1.json").exists(), "the temporary chat writes no file");
    let _ = std::fs::remove_dir_all(&chats_dir);
}

#[test]
fn last_normal_chat_cant_be_deleted() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "lastnormal");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            assert_eq!(this.chats.len(), 1);
            this.delete_chat_now(0, window, cx);
            assert_eq!(this.chats.len(), 1, "the last normal chat survives");
        });
    });
}

#[test]
fn last_temp_chat_delete_spawns_normal_chat() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "lasttemp");
    cx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.chats[0].ephemeral = true;
            this.delete_chat_now(0, window, cx);
            assert_eq!(this.chats.len(), 1, "a replacement chat opens");
            assert!(!this.chats[0].ephemeral, "the replacement is a normal chat");
            assert_eq!(this.active, 0);
        });
    });
}

#[test]
fn temp_chat_excluded_from_search_docs() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "search");
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_temp_chat(cx);
            let docs = this.search_docs();
            assert_eq!(docs.len(), 1, "only the normal chat is searchable");
            assert_eq!(docs[0].title.as_ref(), "New chat");
        });
    });
}

#[test]
fn temp_chat_excluded_from_snapshot_seeds() {
    let mut temp = temp_chat(1);
    temp.checkpoints.push(crate::checkpoints::TurnCheckpoint {
        ix: 0,
        at: std::time::SystemTime::now(),
        checkpoint: crate::checkpoints::Checkpoint::Git("abc".into()),
    });
    let mut normal = Chat::new(0, "normal");
    normal.checkpoints.push(crate::checkpoints::TurnCheckpoint {
        ix: 0,
        at: std::time::SystemTime::now(),
        checkpoint: crate::checkpoints::Checkpoint::Git("def".into()),
    });
    let seeds = crate::snapshots::seeds(&[normal, temp], std::path::Path::new("/tmp"));
    assert_eq!(seeds.len(), 1, "temporary chats contribute no snapshot rows");
    assert_eq!(seeds[0].title, "normal");
}

#[test]
fn temp_chat_queue_not_persisted() {
    let dir = temp_chats_dir("queue");
    let mut queue = crate::send_queue::SendQueue::default();
    let normal = Chat::new(0, "normal");
    let temp = temp_chat(1);
    queue.enqueue(0, crate::send_queue::Queued::new("keep".into(), vec![]), |_| true);
    queue.enqueue(1, crate::send_queue::Queued::new("drop".into(), vec![]), |_| true);
    queue.persist(&dir, &[normal, temp]);
    let json = std::fs::read_to_string(dir.join("queue.json")).unwrap();
    assert!(json.contains("keep"), "the normal chat's queue persists");
    assert!(!json.contains("drop"), "the temporary chat's queue never persists");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn temp_chat_leaves_no_activity() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "activity");
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_temp_chat(cx);
            let temp_id = this.chats[this.active].id;
            this.record_turn_finished(temp_id);
            assert!(this.activity.entries.is_empty(), "temporary chats leave no feed entries");
            this.record_turn_finished(this.chats[0].id);
            assert_eq!(this.activity.entries.len(), 1, "normal chats still record");
        });
    });
}

#[test]
fn temp_indicators_render() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "indicators");
    let (normal_id, temp_id) = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            let normal = this.chats[0].id;
            this.new_temp_chat(cx);
            (normal, this.chats[this.active].id)
        })
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find(("temp-glyph", temp_id)).visible(), "temp row carries the ghost glyph");
        assert!(window.try_find(("temp-glyph", normal_id)).is_none(), "normal rows have no glyph");
        assert!(window.find("temp-badge").visible(), "header shows the Temporary chip");
    });
}

#[test]
fn chat_menu_offers_new_temp_chat() {
    let mut app = TestAppContext::single();
    let (_ws, cx) = mount(&mut app, "menu");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("chat-menu", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("popup-menu").visible(), "⋯ should open the chat menu");
        assert!(
            snapshots(window)
                .iter()
                .any(|s| s.role() == Some(A11yRole::MenuItem) && s.label() == Some("New Temporary Chat")),
            "chat menu should offer New Temporary Chat"
        );
    });
}

/// Export refuses a temporary chat with a note instead of a save dialog —
/// nothing about it may be written out.
#[test]
fn temp_chat_export_is_refused() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "export");
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_temp_chat(cx);
            let ix = this.active;
            this.export_chat(ix, cx);
            let chat = &this.chats[ix];
            assert!(
                chat.messages
                    .iter()
                    .any(|m| matches!(&m.kind, crate::model::MessageKind::Text(t) if t.contains("can't be exported"))),
                "export leaves a refusal note"
            );
        });
    });
}

/// "Open in New Window" can't work for a chat that never reaches disk —
/// the call is a no-op and no second window appears.
#[test]
fn temp_chat_open_in_new_window_is_noop() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app, "window");
    let temp_id = cx.update(|_, cx| {
        ws.update(cx, |this, cx| {
            this.new_temp_chat(cx);
            this.chats[this.active].id
        })
    });
    cx.update(|_, cx| {
        ws.update(cx, |this, cx| this.open_chat_in_new_window(temp_id, cx));
    });
    app.run_until_parked();
    assert!(other_workspaces(&mut app, &ws).is_empty(), "no window spawns for a temporary chat");
}
