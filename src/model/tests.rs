use super::*;

#[test]
fn complete_turn_records_duration() {
    let mut chat = Chat::new(1, "t");
    chat.started_at = Some(Instant::now() - std::time::Duration::from_secs(3));
    chat.complete_turn();
    assert!(chat.started_at.is_none());
    assert!(chat.last_turn.is_some_and(|d| d.as_secs() >= 3));
}

#[test]
fn complete_turn_without_start_records_nothing() {
    let mut chat = Chat::new(1, "t");
    chat.complete_turn();
    assert!(chat.last_turn.is_none());
}
