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
pub(crate) fn item_ix(item: &serde_json::Value) -> usize {
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
pub(crate) fn file_change_events(item: &serde_json::Value) -> Vec<AgentEvent> {
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
