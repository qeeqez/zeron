//! Tests for `date_separator` — the day divider above the first visible
//! message of each new day. Unit tests pin the label rules and boundary
//! detection; headless runs mount the workspace and read the separators'
//! a11y labels, plus the scroll-target mapping that must keep landing on
//! message indices (separators live inside message rows, not between them).

use std::time::SystemTime;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::model::{ChatMessage, MessageKind, Role};
use crate::views::date_separator::{day_label, separator_label};
use crate::workspace::Workspace;

/// Local `h:m` on `y`-`m`-`d` — midday keeps the local date stable across
/// DST edges.
fn at_hms(y: i32, m: u32, d: u32, h: u32, min: u32) -> SystemTime {
    use chrono::TimeZone;
    let dt = chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap();
    chrono::Local.from_local_datetime(&dt).unwrap().into()
}

fn at(y: i32, m: u32, d: u32) -> SystemTime {
    at_hms(y, m, d, 12, 0)
}

#[test]
fn separator_label_marks_day_boundaries() {
    // First visible message — no divider above the transcript's head.
    assert_eq!(separator_label(None, at(2026, 3, 3)), None);
    // Same local day — no divider.
    assert_eq!(separator_label(Some(at(2026, 3, 3)), at(2026, 3, 3)), None);
    // A new day gets a divider; the exact text is `day_label`'s job.
    assert!(separator_label(Some(at(2026, 3, 3)), at(2026, 3, 4)).is_some());
    // Midnight crossing: 23:59 → 00:01 is a new day even minutes apart.
    let late = at_hms(2026, 3, 3, 23, 59);
    let early = at_hms(2026, 3, 4, 0, 1);
    assert!(separator_label(Some(late), early).is_some());
}

#[test]
fn day_label_names_recent_days_and_dates() {
    // Fixed "now" keeps the assertions independent of the run date.
    let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 16).unwrap();
    assert_eq!(day_label(today, today), "Today");
    assert_eq!(day_label(today.pred_opt().unwrap(), today), "Yesterday");
    // Same year → weekday + month + day, no year.
    assert_eq!(day_label(chrono::NaiveDate::from_ymd_opt(2026, 3, 3).unwrap(), today), "Tue, Mar 3");
    // Other year → year appended.
    assert_eq!(day_label(chrono::NaiveDate::from_ymd_opt(2020, 3, 3).unwrap(), today), "Tue, Mar 3, 2020");
}

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats reads+writes stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-date-sep-test-{}", std::process::id()));
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

/// Append a text message stamped `at` to the active chat and grow the
/// scroller — same shape as the other transcript test helpers.
fn push(ws: &Entity<Workspace>, cx: &mut VisualTestContext, s: &str, at: SystemTime) {
    ws.update(cx, |this, cx| {
        std::rc::Rc::make_mut(&mut this.chats[this.active].messages).push(ChatMessage {
            role: Role::User,
            kind: MessageKind::Text(s.into()),
            rating: None,
            bookmarked: false,
            usage: None,
            attachments: vec![],
            at,
        });
        let count = this.chats[this.active].messages.len();
        this.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    });
}

#[test]
fn separators_render_between_days_with_labels() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // Three days: Mar 3 2020 ×2, Mar 5 2020, today ×2.
    push(&ws, cx, "day one a", at(2020, 3, 3));
    push(&ws, cx, "day one b", at(2020, 3, 3));
    push(&ws, cx, "day two", at(2020, 3, 5));
    push(&ws, cx, "today a", SystemTime::now());
    push(&ws, cx, "today b", SystemTime::now());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        // No divider on the transcript's first message or within a day.
        assert!(window.try_find(("date-separator", 0usize)).is_none(), "no header above the first message");
        assert!(window.try_find(("date-separator", 1usize)).is_none(), "same day → none");
        assert!(window.try_find(("date-separator", 4usize)).is_none(), "same day → none");
        // One divider where each new day starts, carrying its label.
        assert_eq!(window.find(("date-separator", 2usize)).label(), Some("Thu, Mar 5, 2020"));
        assert_eq!(window.find(("date-separator", 3usize)).label(), Some("Today"));
    });
}

#[test]
fn scroll_targets_still_land_on_message_indices() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    push(&ws, cx, "old", at(2020, 3, 3));
    push(&ws, cx, "middle", at(2020, 3, 5));
    push(&ws, cx, "latest", SystemTime::now());
    cx.update(|window, cx| {
        // Separators don't occupy rows: scroller positions still equal
        // message indices, so a jump to message 2 puts row ("msg", 2) —
        // and its "Today" divider — in view.
        ws.update(cx, |this, cx| this.scroll_to_message(2, cx));
        window.draw(cx).clear(cx);
        assert!(window.find(("msg", 2usize)).visible(), "scroll_to_item landed on message 2");
        assert_eq!(window.find(("date-separator", 2usize)).label(), Some("Today"));
        // Nav cursor indices are message indices too — G targets the last.
        ws.update(cx, |this, _| {
            this.nav = Some(crate::msg_nav::MsgNav { chat_id: this.chats[this.active].id, ix: 1 });
        });
        window.draw(cx).clear(cx);
        assert!(window.find(("msg", 1usize)).visible(), "nav cursor row renders");
    });
}
