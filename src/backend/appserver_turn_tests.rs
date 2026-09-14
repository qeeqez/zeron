//! Turn-level decoder tests: `turn/completed` variants, token usage, and
//! the synthetic plan card's lifecycle. Pure parsing — no subprocess.

use super::appserver::TurnDecoder;
use crate::backend::AgentEvent;

fn events(d: &mut TurnDecoder, line: &str) -> Vec<AgentEvent> {
    d.line(line).events
}

#[test]
fn turn_completed_variants() {
    // Clean completion.
    let mut d = TurnDecoder::new();
    let dec =
        d.line(r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed","error":null,"items":[]}}}"#);
    assert!(dec.turn_over);
    assert!(matches!(&dec.events[0], AgentEvent::Done));

    // Failed turn surfaces the error, then Done.
    let mut d = TurnDecoder::new();
    let dec = d.line(
        r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"failed","error":{"message":"boom","additionalDetails":null},"items":[]}}}"#,
    );
    assert!(dec.turn_over);
    assert!(matches!(&dec.events[0], AgentEvent::Error(e) if e == "boom"));
    assert!(matches!(&dec.events[1], AgentEvent::Done));

    // Interrupted (user cancel) is a clean stop — no error bubble.
    let mut d = TurnDecoder::new();
    let dec =
        d.line(r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"interrupted","error":null,"items":[]}}}"#);
    assert!(dec.turn_over);
    assert_eq!(dec.events.len(), 1);
    assert!(matches!(&dec.events[0], AgentEvent::Done));

    // A turn error already emitted isn't duplicated at completion.
    let mut d = TurnDecoder::new();
    events(
        &mut d,
        r#"{"method":"error","params":{"error":{"message":"boom","additionalDetails":null},"willRetry":false,"threadId":"t","turnId":"u"}}"#,
    );
    let dec = d.line(
        r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"failed","error":{"message":"boom","additionalDetails":null},"items":[]}}}"#,
    );
    assert_eq!(dec.events.len(), 1);
    assert!(matches!(&dec.events[0], AgentEvent::Done));
}

#[test]
fn plan_card_closes_when_turn_completes() {
    // `turn/plan/updated` opens a synthetic Running card; the turn ends
    // without an item/completed for it, so `turn/completed` must emit the
    // card's ToolCallEnd or it spins forever.
    let mut d = TurnDecoder::new();
    let evs = events(
        &mut d,
        r#"{"method":"turn/plan/updated","params":{"threadId":"t","turnId":"u","explanation":null,"plan":[{"step":"scan","status":"inProgress"}]}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallStart { name, .. } if name == "plan"));

    let dec =
        d.line(r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed","error":null,"items":[]}}}"#);
    assert!(dec.turn_over);
    assert!(matches!(&dec.events[0], AgentEvent::ToolCallEnd { ok: true, .. }));
    assert!(matches!(&dec.events[1], AgentEvent::Done));

    // Interrupted turn: the card still closes — as failed, not spinning.
    let mut d = TurnDecoder::new();
    events(
        &mut d,
        r#"{"method":"turn/plan/updated","params":{"threadId":"t","turnId":"u","explanation":null,"plan":[{"step":"scan","status":"inProgress"}]}}"#,
    );
    let dec =
        d.line(r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"interrupted","error":null,"items":[]}}}"#);
    assert!(matches!(&dec.events[0], AgentEvent::ToolCallEnd { ok: false, .. }));
    assert!(matches!(&dec.events[1], AgentEvent::Done));

    // A turn that never showed a plan emits no ToolCallEnd.
    let mut d = TurnDecoder::new();
    let dec =
        d.line(r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed","error":null,"items":[]}}}"#);
    assert_eq!(dec.events.len(), 1);
    assert!(matches!(&dec.events[0], AgentEvent::Done));
}

#[test]
fn usage_reads_last_turn_slice() {
    let mut d = TurnDecoder::new();
    let evs = events(
        &mut d,
        r#"{"method":"thread/tokenUsage/updated","params":{"threadId":"t","turnId":"u","tokenUsage":{"total":{"inputTokens":100,"outputTokens":50},"last":{"inputTokens":40,"outputTokens":10},"modelContextWindow":200000}}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::Usage { input: 40, output: 10 }));
}
