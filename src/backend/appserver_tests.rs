//! Tests for the `codex app-server` NDJSON-RPC decoder — pure parsing,
//! no subprocess or network. Line shapes mirror a captured 0.154.0 stream.

use super::appserver::TurnDecoder;
use crate::backend::{AgentEvent, ApprovalRoute};

fn events(d: &mut TurnDecoder, line: &str) -> Vec<AgentEvent> {
    d.line(line).events
}

#[test]
fn agent_message_streams_deltas() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"item/started","params":{"item":{"type":"agentMessage","id":"m1","text":"","phase":"final_answer"},"threadId":"t","turnId":"u","startedAtMs":1}}"#,
    );
    assert_eq!(evs.len(), 1);
    assert!(matches!(evs[0], AgentEvent::TextStart));

    let evs =
        events(&mut d, r#"{"method":"item/agentMessage/delta","params":{"threadId":"t","turnId":"u","itemId":"m1","delta":"hello"}}"#);
    assert!(matches!(&evs[0], AgentEvent::TextDelta(t) if t == "hello"));
    let evs =
        events(&mut d, r#"{"method":"item/agentMessage/delta","params":{"threadId":"t","turnId":"u","itemId":"m1","delta":" world"}}"#);
    assert!(matches!(&evs[0], AgentEvent::TextDelta(t) if t == " world"));

    // Completed carries the full text again — must not re-emit.
    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"m1","text":"hello world"},"threadId":"t","turnId":"u","completedAtMs":2}}"#,
    );
    assert!(evs.is_empty());
}

#[test]
fn agent_message_completed_without_deltas() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"m1","text":"hi"},"threadId":"t","turnId":"u","completedAtMs":1}}"#,
    );
    assert_eq!(evs.len(), 1);
    assert!(matches!(&evs[0], AgentEvent::TextDelta(t) if t == "hi"));
}

#[test]
fn command_execution_streams_output() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"item/started","params":{"item":{"type":"commandExecution","id":"c1","command":"/bin/zsh -lc 'ls'","status":"inProgress","aggregatedOutput":null,"exitCode":null},"threadId":"t","turnId":"u","startedAtMs":1}}"#,
    );
    assert_eq!(evs.len(), 1);
    assert!(matches!(&evs[0], AgentEvent::ToolCallStart { name, detail, .. } if name == "shell" && detail.contains("ls")));

    let evs = events(
        &mut d,
        r#"{"method":"item/commandExecution/outputDelta","params":{"threadId":"t","turnId":"u","itemId":"c1","delta":"a.rs\n"}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallDelta { output, .. } if output == "a.rs\n"));

    // Completed repeats aggregatedOutput — only the End may be emitted.
    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"c1","command":"ls","status":"completed","aggregatedOutput":"a.rs\n","exitCode":0},"threadId":"t","turnId":"u","completedAtMs":2}}"#,
    );
    assert_eq!(evs.len(), 1);
    assert!(matches!(&evs[0], AgentEvent::ToolCallEnd { ok: true, .. }));
}

#[test]
fn command_execution_completed_without_deltas() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"c1","command":"ls","status":"completed","aggregatedOutput":"out","exitCode":0},"threadId":"t","turnId":"u","completedAtMs":1}}"#,
    );
    // Start (defensive) + Delta + End.
    assert_eq!(evs.len(), 3);
    assert!(matches!(&evs[0], AgentEvent::ToolCallStart { .. }));
    assert!(matches!(&evs[1], AgentEvent::ToolCallDelta { output, .. } if output == "out"));
    assert!(matches!(&evs[2], AgentEvent::ToolCallEnd { ok: true, .. }));
}

#[test]
fn failed_command_marks_not_ok() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"c1","command":"false","status":"failed","aggregatedOutput":"","exitCode":1},"threadId":"t","turnId":"u","completedAtMs":1}}"#,
    );
    assert!(evs.iter().any(|e| matches!(e, AgentEvent::ToolCallEnd { ok: false, .. })));
}

#[test]
fn reasoning_streams_into_thinking_card() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"item/started","params":{"item":{"type":"reasoning","id":"r1","summary":[],"content":[]},"threadId":"t","turnId":"u","startedAtMs":1}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallStart { name, .. } if name == "thinking"));

    let evs = events(
        &mut d,
        r#"{"method":"item/reasoning/summaryTextDelta","params":{"threadId":"t","turnId":"u","itemId":"r1","delta":"thinking hard","summaryIndex":0}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallDelta { output, .. } if output == "thinking hard"));

    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"reasoning","id":"r1","summary":["thinking hard"],"content":[]},"threadId":"t","turnId":"u","completedAtMs":2}}"#,
    );
    assert_eq!(evs.len(), 1);
    assert!(matches!(&evs[0], AgentEvent::ToolCallEnd { ok: true, .. }));
}

#[test]
fn plan_updates_replace_checklist() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"turn/plan/updated","params":{"threadId":"t","turnId":"u","explanation":null,"plan":[{"step":"scan","status":"inProgress"},{"step":"edit","status":"pending"}]}}"#,
    );
    assert_eq!(evs.len(), 1);
    let AgentEvent::Plan { steps, .. } = &evs[0] else { panic!("expected Plan") };
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].label.as_str(), "scan");
    assert_eq!(steps[0].status, crate::model::PlanStatus::InProgress);
    assert_eq!(steps[1].status, crate::model::PlanStatus::Pending);

    // Second update: same card ix, steps replaced wholesale.
    let evs = events(
        &mut d,
        r#"{"method":"turn/plan/updated","params":{"threadId":"t","turnId":"u","explanation":null,"plan":[{"step":"scan","status":"completed"},{"step":"edit","status":"inProgress"}]}}"#,
    );
    assert_eq!(evs.len(), 1);
    let AgentEvent::Plan { steps, .. } = &evs[0] else { panic!("expected Plan") };
    assert_eq!(steps[0].status, crate::model::PlanStatus::Done);
    assert_eq!(steps[1].status, crate::model::PlanStatus::InProgress);
}

#[test]
fn plan_item_completed_decodes_checklist_text() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    // A `plan` item whose text is a markdown checklist becomes Plan steps.
    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"plan","id":"p1","text":"- [x] scan repo\n- [ ] edit files"},"threadId":"t","turnId":"u"}}"#,
    );
    assert_eq!(evs.len(), 1);
    let AgentEvent::Plan { steps, .. } = &evs[0] else { panic!("expected Plan") };
    assert_eq!(steps[0].status, crate::model::PlanStatus::Done);
    assert_eq!(steps[1].status, crate::model::PlanStatus::Pending);

    // Prose plans keep the old text card — opened and closed in one shot.
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"plan","id":"p2","text":"Approach:\nDo the thing."},"threadId":"t","turnId":"u"}}"#,
    );
    assert_eq!(evs.len(), 3);
    assert!(matches!(&evs[0], AgentEvent::ToolCallStart { name, .. } if name == "plan"));
    assert!(matches!(&evs[1], AgentEvent::ToolCallSet { output, .. } if output.contains("Approach")));
    assert!(matches!(&evs[2], AgentEvent::ToolCallEnd { ok: true, .. }));
}

#[test]
fn mcp_tool_call_lifecycle() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"item/started","params":{"item":{"type":"mcpToolCall","id":"p1","server":"docs","tool":"search","status":"inProgress","arguments":{}},"threadId":"t","turnId":"u","startedAtMs":1}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallStart { name, detail, .. } if name == "docs" && detail == "search"));

    let evs = events(
        &mut d,
        r#"{"method":"item/mcpToolCall/progress","params":{"threadId":"t","turnId":"u","itemId":"p1","message":"querying"}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallDelta { output, .. } if output.contains("querying")));

    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"mcpToolCall","id":"p1","server":"docs","tool":"search","status":"completed","result":{"content":[{"type":"text","text":"found it"}],"structuredContent":null},"error":null},"threadId":"t","turnId":"u","completedAtMs":2}}"#,
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::ToolCallDelta { output, .. } if output.contains("found it")))
    );
    assert!(evs.iter().any(|e| matches!(e, AgentEvent::ToolCallEnd { ok: true, .. })));
}

#[test]
fn web_search_card() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"item/started","params":{"item":{"type":"webSearch","id":"w1","query":"rust async","action":null},"threadId":"t","turnId":"u","startedAtMs":1}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallStart { name, detail, .. } if name == "web_search" && detail == "rust async"));
    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"webSearch","id":"w1","query":"rust async","action":null},"threadId":"t","turnId":"u","completedAtMs":2}}"#,
    );
    assert!(matches!(&evs[0], AgentEvent::ToolCallEnd { ok: true, .. }));
}

#[test]
fn error_will_retry_is_transient() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    let evs = events(
        &mut d,
        r#"{"method":"error","params":{"error":{"message":"Reconnecting... 1/5","additionalDetails":null},"willRetry":true,"threadId":"t","turnId":"u"}}"#,
    );
    assert!(evs.is_empty());
    let evs = events(
        &mut d,
        r#"{"method":"error","params":{"error":{"message":"rate limited","additionalDetails":"try again later"},"willRetry":false,"threadId":"t","turnId":"u"}}"#,
    );
    assert_eq!(evs.len(), 1);
    assert!(matches!(&evs[0], AgentEvent::Error(e) if e.contains("rate limited") && e.contains("try again later")));
}

#[test]
fn mcp_structured_only_result_renders() {
    // `structuredContent` as an object with an empty `content` array must
    // still produce output — `as_str()` alone would render it blank.
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    events(
        &mut d,
        r#"{"method":"item/started","params":{"item":{"type":"mcpToolCall","id":"m1","server":"db","tool":"query","status":"inProgress","arguments":{}},"threadId":"t","turnId":"u","startedAtMs":1}}"#,
    );
    let evs = events(
        &mut d,
        r#"{"method":"item/completed","params":{"item":{"type":"mcpToolCall","id":"m1","server":"db","tool":"query","status":"completed","result":{"content":[],"structuredContent":{"rows":3,"ok":true}},"error":null},"threadId":"t","turnId":"u","completedAtMs":2}}"#,
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::ToolCallDelta { output, .. } if output.contains("\"rows\": 3")))
    );
    assert!(evs.iter().any(|e| matches!(e, AgentEvent::ToolCallEnd { ok: true, .. })));
}

#[test]
fn malformed_and_unrelated_lines_ignored() {
    let mut d = TurnDecoder::new(ApprovalRoute::Ask);
    assert!(events(&mut d, "not json").is_empty());
    assert!(events(&mut d, r#"{"method":"mcpServer/startupStatus/updated","params":{"name":"x","status":"ready"}}"#).is_empty());
    assert!(events(&mut d, r#"{"method":"thread/started","params":{"thread":{"id":"t"}}}"#).is_empty());
    // Our own request responses carry no method — ignored by the decoder.
    assert!(events(&mut d, r#"{"id":1,"result":{"userAgent":"x"}}"#).is_empty());
}
