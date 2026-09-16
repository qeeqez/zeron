//! Headless tests for the image lightbox: clicking an image thumbnail — a
//! composer attachment chip or an image on a sent message — opens the
//! full-size overlay; Esc, a backdrop click, and the ✕ button close it; a
//! missing file shows a placeholder.
//!
//! Imports stay narrow on purpose: `use gpui_kit::*` would pull the
//! `#[gpui_kit::test]` attribute into scope under its plain name `test`,
//! shadowing the built-in `#[test]` the expansion relies on.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, point, px};

use crate::composer_testutil::open_workspace;
use crate::model::{ChatMessage, MessageKind, Role};
use crate::workspace::Workspace;

/// Push a user message carrying `attachments` without starting a reply.
fn seed_user_with_attachments(ws: &Entity<Workspace>, attachments: &[&str], cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            role: Role::User,
            kind: MessageKind::Text("with image".into()),
            rating: None,
            bookmarked: false,
            usage: None,
            attachments: attachments.iter().map(|a| (*a).into()).collect(),
            at: std::time::SystemTime::now(),
        });
        this.scroller.update(cx, |s, cx| s.append(1, cx));
        cx.notify();
    });
}

/// The lightbox's open path, or `None` when closed.
fn lightbox_path(ws: &Entity<Workspace>, cx: &VisualTestContext) -> Option<String> {
    ws.read_with(cx, |ws, _| ws.image_view.as_ref().map(|p| p.to_string()))
}

/// Clicking a composer attachment thumbnail opens the lightbox on that path.
#[test]
fn composer_thumbnail_click_opens_lightbox() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let dir = std::env::temp_dir().join(format!("rixlcode-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shot.png");
    std::fs::write(&path, b"png").unwrap();
    ws.update(cx, |this, cx| {
        this.add_attachments(vec![path.clone(), std::path::PathBuf::from("/tmp/notes.txt")], cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.try_find("image-view-overlay").is_none(), "lightbox starts closed");
        window.click("attach-thumb-0", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("image-view-overlay").visible(), "thumbnail click should open the lightbox");
        assert_eq!(window.find("image-view-img").label(), Some(path.to_string_lossy().as_ref()));
    });
    assert_eq!(lightbox_path(&ws, cx).as_deref(), Some(path.to_string_lossy().as_ref()));
}

/// Clicking an image thumbnail on a sent message opens the lightbox.
#[test]
fn message_thumbnail_click_opens_lightbox() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let dir = std::env::temp_dir().join(format!("rixlcode-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("photo.jpg");
    std::fs::write(&path, b"jpg").unwrap();
    seed_user_with_attachments(&ws, &[path.to_str().unwrap(), "/tmp/notes.txt"], cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // The .jpg previews as a thumbnail; the .txt attachment does not.
        assert!(window.try_find("msg-thumb-0-0").is_some());
        assert!(window.try_find("msg-thumb-0-1").is_none());
        window.click("msg-thumb-0-0", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("image-view-overlay").visible());
    });
    assert_eq!(lightbox_path(&ws, cx).as_deref(), Some(path.to_string_lossy().as_ref()));
}

/// Esc, a backdrop click, and the ✕ button all dismiss the lightbox.
#[test]
fn lightbox_closes_on_escape_backdrop_and_button() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let dir = std::env::temp_dir().join(format!("rixlcode-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shot.png");
    std::fs::write(&path, b"png").unwrap();
    ws.update(cx, |this, cx| {
        this.add_attachments(vec![path.clone()], cx);
    });
    cx.update(|window, cx| {
        cx.bind_keys(crate::workspace_keys());
        window.draw(cx).clear(cx);

        // Esc closes.
        window.click("attach-thumb-0", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("image-view-overlay").visible());
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("image-view-overlay").is_none(), "esc should close the lightbox");

        // A press on the dimmed backdrop (outside the centered image) closes.
        window.click("attach-thumb-0", cx);
        window.draw(cx).clear(cx);
        window.click_at("image-view-backdrop", point(px(8.), px(8.)), cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("image-view-overlay").is_none(), "backdrop click should close the lightbox");

        // The ✕ button closes.
        window.click("attach-thumb-0", cx);
        window.draw(cx).clear(cx);
        window.click("image-view-close", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("image-view-overlay").is_none(), "close button should dismiss the lightbox");
    });
    assert_eq!(lightbox_path(&ws, cx), None);
}

/// An attachment whose file is gone opens the lightbox on a placeholder
/// instead of an empty frame.
#[test]
fn missing_file_shows_placeholder() {
    let mut app = TestAppContext::single();
    let (ws, cx) = open_workspace(&mut app);
    let gone = std::env::temp_dir().join(format!("rixlcode-img-{}/gone.png", std::process::id()));
    ws.update(cx, |this, cx| {
        this.add_attachments(vec![gone.clone()], cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("attach-thumb-0", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("image-view-overlay").visible());
        assert!(window.find("image-view-missing").visible(), "missing file should show the placeholder");
        assert!(window.try_find("image-view-img").is_none(), "no image element for a missing file");
    });
    assert_eq!(lightbox_path(&ws, cx).as_deref(), Some(gone.to_string_lossy().as_ref()));
}
