//! Parse `codex exec --json` JSONL lines into `AgentEvent`s.

use crate::backend::AgentEvent;

/// Map one `codex exec --json` JSONL line to zero or more `AgentEvent`s.
pub fn parse_codex_line(line: &str) -> Vec<AgentEvent> {
    let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) else { return vec![] };
    let item = &ev["item"];
    let Some(kind) = ev["type"].as_str() else { return vec![] };
    match kind {
        "item.started" if item["type"].as_str() == Some("command_execution") => vec![AgentEvent::ToolCallStart {
            ix: item_ix(item),
            name: "shell".into(),
            detail: item["command"].as_str().unwrap_or("").into(),
        }],
        "item.started" if item["type"].as_str() == Some("agent_message") => vec![AgentEvent::TextStart],
        "item.completed" => match item["type"].as_str() {
            Some("command_execution") => {
                let mut out = Vec::with_capacity(2);
                let output = item["aggregated_output"].as_str().unwrap_or("");
                if !output.is_empty() {
                    out.push(AgentEvent::ToolCallDelta { ix: item_ix(item), output: output.into() });
                }
                out.push(AgentEvent::ToolCallEnd { ix: item_ix(item), ok: item["exit_code"].as_i64() == Some(0) });
                out
            },
            Some("agent_message") => vec![AgentEvent::TextDelta(item["text"].as_str().unwrap_or("").into())],
            Some("reasoning") => reasoning_events(item),
            Some("file_change") => file_change_events(item),
            Some("error") => vec![AgentEvent::Error(item["message"].as_str().unwrap_or("codex error").into())],
            _ => vec![],
        },
        "error" => vec![AgentEvent::Error(ev["message"].as_str().unwrap_or("codex error").into())],
        "turn.failed" => vec![AgentEvent::Error(ev["error"]["message"].as_str().unwrap_or("turn failed").into())],
        "turn.completed" => {
            let usage = &ev["usage"];
            let input = usage["input_tokens"].as_u64().unwrap_or(0);
            let output = usage["output_tokens"].as_u64().unwrap_or(0);
            vec![AgentEvent::Usage { input, output }, AgentEvent::Done]
        },
        _ => vec![],
    }
}

/// Stable per-item key — codex's `item.id` string hashed so parallel
/// tool calls route deltas to the right card. Process-local only.
fn item_ix(item: &serde_json::Value) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    item["id"].as_str().unwrap_or("").hash(&mut h);
    h.finish() as usize
}

/// Turn a completed `reasoning` item into a collapsible "thinking" card.
/// `text` is a string on some codex versions, an array of {text:…} on others.
fn reasoning_events(item: &serde_json::Value) -> Vec<AgentEvent> {
    let text = item["text"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            item["text"]
                .as_array()
                .map(|a| a.iter().filter_map(|t| t["text"].as_str()).collect::<Vec<_>>().join("\n"))
        })
        .unwrap_or_default();
    if text.is_empty() {
        vec![]
    } else {
        vec![
            AgentEvent::ToolCallStart {
                ix: item_ix(item),
                name: "thinking".into(),
                detail: "".into(),
            },
            AgentEvent::ToolCallDelta { ix: item_ix(item), output: text.into() },
            AgentEvent::ToolCallEnd { ix: item_ix(item), ok: true },
        ]
    }
}

/// Turn a completed `file_change` item into `Diff` cards by asking git for
/// the working-tree diff of each touched path.
fn file_change_events(item: &serde_json::Value) -> Vec<AgentEvent> {
    if item["status"].as_str() != Some("completed") {
        return vec![];
    }
    let Some(changes) = item["changes"].as_array() else { return vec![] };
    changes.iter().filter_map(|c| c["path"].as_str()).filter_map(diff_for_path).collect()
}

/// `git diff` for `path` (or `--no-index` for untracked files), capped at
/// 200 lines so a huge generated file can't flood the chat.
fn diff_for_path(path: &str) -> Option<AgentEvent> {
    let tracked = std::process::Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", path])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let output = if tracked {
        std::process::Command::new("git").args(["diff", "--", path]).output().ok()?
    } else {
        std::process::Command::new("git")
            .args(["diff", "--no-index", "--", "/dev/null", path])
            .output()
            .ok()?
    };
    let text = String::from_utf8_lossy(&output.stdout);
    if text.trim().is_empty() {
        return None;
    }
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut hunks = String::new();
    let mut kept = 0usize;
    for line in text.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed += 1;
        }
        if kept < 200 {
            hunks.push_str(line);
            hunks.push('\n');
            kept += 1;
        }
    }
    Some(AgentEvent::Diff { path: path.into(), added, removed, hunks: hunks.into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_json_yields_nothing() {
        assert!(parse_codex_line("not json").is_empty());
        assert!(parse_codex_line("{}").is_empty());
        assert!(parse_codex_line(r#"{"type":"item.started"}"#).is_empty());
    }

    #[test]
    fn command_execution_start_and_end() {
        let start = parse_codex_line(r#"{"type":"item.started","item":{"id":"a","type":"command_execution","command":"ls"}}"#);
        assert!(matches!(&start[0], AgentEvent::ToolCallStart { name, detail, .. } if name == "shell" && detail == "ls"));
        let done = parse_codex_line(
            r#"{"type":"item.completed","item":{"id":"a","type":"command_execution","aggregated_output":"hi\n","exit_code":0}}"#,
        );
        assert_eq!(done.len(), 2);
        assert!(matches!(&done[0], AgentEvent::ToolCallDelta { output, .. } if output == "hi\n"));
        assert!(matches!(&done[1], AgentEvent::ToolCallEnd { ok: true, .. }));
    }

    #[test]
    fn empty_output_skips_delta() {
        let done = parse_codex_line(
            r#"{"type":"item.completed","item":{"id":"a","type":"command_execution","aggregated_output":"","exit_code":1}}"#,
        );
        assert_eq!(done.len(), 1);
        assert!(matches!(&done[0], AgentEvent::ToolCallEnd { ok: false, .. }));
    }

    #[test]
    fn agent_message_starts_new_bubble() {
        let start = parse_codex_line(r#"{"type":"item.started","item":{"id":"m1","type":"agent_message"}}"#);
        assert!(matches!(&start[0], AgentEvent::TextStart));
        let done = parse_codex_line(r#"{"type":"item.completed","item":{"id":"m1","type":"agent_message","text":"hi"}}"#);
        assert!(matches!(&done[0], AgentEvent::TextDelta(t) if t == "hi"));
    }

    #[test]
    fn turn_completed_emits_usage_and_done() {
        let evs = parse_codex_line(r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":5}}"#);
        assert_eq!(evs.len(), 2);
        assert!(matches!(&evs[0], AgentEvent::Usage { input: 10, output: 5 }));
        assert!(matches!(&evs[1], AgentEvent::Done));
    }

    #[test]
    fn error_variants() {
        let e = parse_codex_line(r#"{"type":"error","message":"boom"}"#);
        assert!(matches!(&e[0], AgentEvent::Error(m) if m == "boom"));
        let f = parse_codex_line(r#"{"type":"turn.failed","error":{"message":"nope"}}"#);
        assert!(matches!(&f[0], AgentEvent::Error(m) if m == "nope"));
    }

    #[test]
    fn same_item_id_routes_to_same_ix() {
        let a = parse_codex_line(r#"{"type":"item.started","item":{"id":"x","type":"command_execution","command":"a"}}"#);
        let b = parse_codex_line(
            r#"{"type":"item.completed","item":{"id":"x","type":"command_execution","aggregated_output":"","exit_code":0}}"#,
        );
        let ix_a = match &a[0] {
            AgentEvent::ToolCallStart { ix, .. } => *ix,
            _ => panic!(),
        };
        let ix_b = match &b[0] {
            AgentEvent::ToolCallEnd { ix, .. } => *ix,
            _ => panic!(),
        };
        assert_eq!(ix_a, ix_b);
    }
}

#[cfg(test)]
mod reasoning_tests {
    use super::*;

    #[test]
    fn reasoning_string_text() {
        let evs = parse_codex_line(r#"{"type":"item.completed","item":{"id":"r","type":"reasoning","text":"thinking…"}}"#);
        assert_eq!(evs.len(), 3);
        assert!(matches!(&evs[0], AgentEvent::ToolCallStart { name, .. } if name == "thinking"));
        assert!(matches!(&evs[1], AgentEvent::ToolCallDelta { output, .. } if output == "thinking…"));
        assert!(matches!(&evs[2], AgentEvent::ToolCallEnd { ok: true, .. }));
    }

    #[test]
    fn reasoning_array_text() {
        let evs = parse_codex_line(r#"{"type":"item.completed","item":{"id":"r","type":"reasoning","text":[{"text":"a"},{"text":"b"}]}}"#);
        assert_eq!(evs.len(), 3);
        assert!(matches!(&evs[1], AgentEvent::ToolCallDelta { output, .. } if output == "a\nb"));
    }

    #[test]
    fn reasoning_empty_yields_nothing() {
        let evs = parse_codex_line(r#"{"type":"item.completed","item":{"id":"r","type":"reasoning","text":""}}"#);
        assert!(evs.is_empty());
    }
}
