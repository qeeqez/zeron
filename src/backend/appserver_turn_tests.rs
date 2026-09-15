//! Turn-level decoder tests: `turn/completed` variants, token usage, and
//! the synthetic plan card's lifecycle. Pure parsing — no subprocess.

use super::appserver::TurnDecoder;
use crate::backend::{AgentEvent, ApprovalRoute};

fn events(d: &mut TurnDecoder, line: &str) -> Vec<AgentEvent> {
    d.line(line).events
}

#[test]
fn turn_completed_variants() {
    // Clean completion.
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let dec =
        d.line(r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed","error":null,"items":[]}}}"#);
    assert!(dec.turn_over);
    assert!(matches!(&dec.events[0], AgentEvent::Done));

    // Failed turn surfaces the error, then Done.
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let dec = d.line(
        r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"failed","error":{"message":"boom","additionalDetails":null},"items":[]}}}"#,
    );
    assert!(dec.turn_over);
    assert!(matches!(&dec.events[0], AgentEvent::Error(e) if e == "boom"));
    assert!(matches!(&dec.events[1], AgentEvent::Done));

    // Interrupted (user cancel) is a clean stop — no error bubble.
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let dec =
        d.line(r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"interrupted","error":null,"items":[]}}}"#);
    assert!(dec.turn_over);
    assert_eq!(dec.events.len(), 1);
    assert!(matches!(&dec.events[0], AgentEvent::Done));

    // A turn error already emitted isn't duplicated at completion.
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
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
fn plan_card_keeps_last_snapshot_when_turn_completes() {
    // `turn/plan/updated` emits a Plan snapshot; the turn ends without an
    // item/completed for it. The checklist has no spinner, so completion
    // emits only Done — no ToolCallEnd.
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"turn/plan/updated","params":{"threadId":"t","turnId":"u","explanation":null,"plan":[{"step":"scan","status":"inProgress"}]}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::Plan { .. }));

    let dec =
        d.line(r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed","error":null,"items":[]}}}"#);
    assert!(dec.turn_over);
    assert_eq!(dec.events.len(), 1);
    assert!(matches!(&dec.events[0], AgentEvent::Done));

    // Interrupted turn: same — the card just keeps its last snapshot.
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    events(
        &mut d,
        r#"{"method":"turn/plan/updated","params":{"threadId":"t","turnId":"u","explanation":null,"plan":[{"step":"scan","status":"inProgress"}]}}"#,
    );
    let dec =
        d.line(r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"interrupted","error":null,"items":[]}}}"#);
    assert_eq!(dec.events.len(), 1);
    assert!(matches!(&dec.events[0], AgentEvent::Done));

    // A turn that never showed a plan still emits only Done.
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let dec =
        d.line(r#"{"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed","error":null,"items":[]}}}"#);
    assert_eq!(dec.events.len(), 1);
    assert!(matches!(&dec.events[0], AgentEvent::Done));
}

#[test]
fn usage_reads_last_turn_slice() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"thread/tokenUsage/updated","params":{"threadId":"t","turnId":"u","tokenUsage":{"total":{"inputTokens":100,"outputTokens":50},"last":{"inputTokens":40,"outputTokens":10},"modelContextWindow":200000}}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::Usage { input: 40, output: 10 }));
}

#[test]
fn approval_requests_surface_as_cards() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let dec = d.line(
        r#"{"method":"item/commandExecution/requestApproval","id":9,"params":{"threadId":"t","turnId":"u","itemId":"c1","command":"rm -rf /"}}"#,
    );
    // No canned reply — the card's event carries the responder and the
    // pending request waits for the UI's answer.
    assert!(dec.response.is_none());
    let pending = dec.pending.expect("approval must wait on the UI");
    let AgentEvent::ApprovalRequest { kind, detail, respond, .. } = &dec.events[0] else {
        panic!("expected ApprovalRequest, got {:?}", dec.events);
    };
    assert_eq!(*kind, crate::backend::ApprovalKind::Command);
    assert_eq!(detail.as_str(), "rm -rf /");

    // Approve round-trips to the wire as the codex ReviewDecision.
    let mut wire = Vec::new();
    respond.send(crate::backend::ApprovalDecision::Approve).unwrap();
    pending.answer(&mut wire).unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&wire).unwrap();
    assert_eq!(reply["id"], serde_json::json!(9));
    assert_eq!(reply["result"]["decision"], serde_json::json!("approved"));

    // Deny aborts the tool call — "denied" on the wire.
    let dec = d.line(r#"{"method":"applyPatchApproval","id":11,"params":{"conversationId":"t","callId":"c","fileChanges":{"src/a.rs":{}}}}"#);
    let AgentEvent::ApprovalRequest { kind, detail, respond, .. } = &dec.events[0] else {
        panic!("expected ApprovalRequest");
    };
    assert_eq!(*kind, crate::backend::ApprovalKind::Patch);
    assert_eq!(detail.as_str(), "src/a.rs");
    let mut wire = Vec::new();
    respond.send(crate::backend::ApprovalDecision::Deny).unwrap();
    dec.pending.unwrap().answer(&mut wire).unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&wire).unwrap();
    assert_eq!(reply["result"]["decision"], serde_json::json!("denied"));

    // A dropped responder (stop/delete) answers Deny without a click.
    let dec = d.line(r#"{"method":"execCommandApproval","id":12,"params":{"conversationId":"t","callId":"c2","command":"ls"}}"#);
    let mut wire = Vec::new();
    drop(dec.events);
    dec.pending.unwrap().answer(&mut wire).unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&wire).unwrap();
    assert_eq!(reply["result"]["decision"], serde_json::json!("denied"));
}

#[test]
fn auto_route_answers_without_a_card() {
    // Auto/FullAccess: approvals are approved on the spot — no event.
    let mut d = TurnDecoder::new(ApprovalRoute::Auto(crate::backend::ApprovalDecision::Approve));
    let dec = d.line(
        r#"{"method":"item/commandExecution/requestApproval","id":9,"params":{"threadId":"t","turnId":"u","itemId":"c1","command":"rm -rf /"}}"#,
    );
    assert!(dec.events.is_empty() && dec.pending.is_none());
    assert_eq!(dec.response.unwrap()["result"]["decision"], serde_json::json!("approved"));

    // Read-only modes (Plan/Ask) deny instead of prompting.
    let mut d = TurnDecoder::new(ApprovalRoute::Auto(crate::backend::ApprovalDecision::Deny));
    let dec = d.line(r#"{"method":"item/fileChange/requestApproval","id":10,"params":{"threadId":"t","turnId":"u","itemId":"f1"}}"#);
    assert!(dec.events.is_empty());
    assert_eq!(dec.response.unwrap()["result"]["decision"], serde_json::json!("denied"));

    // Always-allow maps to approved_for_session.
    let mut d = TurnDecoder::new(ApprovalRoute::Auto(crate::backend::ApprovalDecision::ApproveForSession));
    let dec = d.line(r#"{"method":"execCommandApproval","id":11,"params":{"conversationId":"t","callId":"c","command":"ls"}}"#);
    assert_eq!(dec.response.unwrap()["result"]["decision"], serde_json::json!("approved_for_session"));
}

#[test]
fn non_approval_requests_get_canned_replies() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let dec = d.line(r#"{"method":"mcpServer/elicitation/request","id":12,"params":{"threadId":"t","message":"need input"}}"#);
    assert_eq!(dec.response.unwrap()["result"]["action"], serde_json::json!("decline"));

    // Unknown requests get a JSON-RPC error so the server can't hang.
    let dec = d.line(r#"{"method":"item/tool/requestUserInput","id":13,"params":{"threadId":"t","turnId":"u","itemId":"x","questions":[],"isBlocking":true}}"#);
    let resp = dec.response.unwrap();
    assert!(resp.get("error").is_some());
}
