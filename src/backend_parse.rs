//! Parse `codex exec --json` JSONL lines into `AgentEvent`s.

use crate::backend::AgentEvent;
use crate::model::{PlanStatus, PlanStep};

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
        // `todo_list` items carry the whole checklist on every event —
        // started, updated and completed all map to a Plan snapshot.
        "item.started" | "item.updated" | "item.completed" if item["type"].as_str() == Some("todo_list") => {
            vec![AgentEvent::Plan {
                ix: item_ix(item),
                steps: plan_steps(&item["items"], "text", "completed"),
            }]
        },

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

/// Item id as a String — app-server ids are already globally unique.
pub(crate) fn item_id(item: &serde_json::Value) -> String {
    item["id"].as_str().unwrap_or("").to_string()
}

/// Reasoning text from a completed app-server item: summary lines plus
/// content.
pub(crate) fn reasoning_text(item: &serde_json::Value) -> String {
    let mut parts: Vec<&str> = item["summary"]
        .as_array()
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    if let Some(content) = item["content"].as_array() {
        parts.extend(content.iter().filter_map(serde_json::Value::as_str));
    }
    parts.join("\n")
}

/// Human-readable result of a completed MCP/dynamic tool call. A
/// structured-only result (`structuredContent` object/array with empty
/// `content`) still renders — serialized, not dropped as blank output.
pub(crate) fn mcp_result_text(item: &serde_json::Value) -> String {
    if let Some(err) = item["error"]["message"].as_str() {
        return format!("error: {err}");
    }
    let result = &item["result"];
    let structured = &result["structuredContent"];
    if let Some(text) = structured.as_str() {
        return text.to_string();
    }
    if !structured.is_null() {
        return serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string());
    }
    result["content"]
        .as_array()
        .map(|c| {
            c.iter()
                .filter_map(|b| b["text"].as_str().map(str::to_string).or_else(|| Some(b.to_string())))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
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

/// Map a backend's step-status word to `PlanStatus`. Covers codex's
/// camelCase (`inProgress`), ACP's snake_case (`in_progress`) and claude's
/// `TodoWrite` names; unknown values read as pending.
pub(crate) fn plan_status(status: Option<&str>) -> PlanStatus {
    match status {
        Some("completed") | Some("done") => PlanStatus::Done,
        Some("inProgress") | Some("in_progress") => PlanStatus::InProgress,
        _ => PlanStatus::Pending,
    }
}

/// Steps from a JSON array — `label`/`status` name the per-entry fields.
/// `status` may be a status word or a `completed: bool` flag.
pub(crate) fn plan_steps(items: &serde_json::Value, label: &str, status: &str) -> Vec<PlanStep> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(id, e)| PlanStep {
            id,
            label: e[label].as_str().unwrap_or("").into(),
            status: match e[status].as_bool() {
                Some(true) => PlanStatus::Done,
                _ => plan_status(e[status].as_str()),
            },
        })
        .collect()
}

/// Steps parsed from a markdown checklist (`- [ ]`, `- [x]`, `* [ ]`,
/// `1. [x]`). Returns `None` when no line carries a checkbox — prose plans
/// aren't checklists.
pub(crate) fn plan_steps_from_text(text: &str) -> Option<Vec<PlanStep>> {
    let mut steps = Vec::new();
    for line in text.lines() {
        // Strip the bullet: `-`, `*`, or an ordered `N.` marker.
        let t = line.trim_start();
        let t = t
            .strip_prefix(['-', '*'])
            .or_else(|| {
                t.split_once(". ")
                    .filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
                    .map(|(_, r)| r)
            })
            .map(str::trim_start)
            .unwrap_or(t);
        let (done, rest) = if let Some(r) = t.strip_prefix("[ ]") {
            (false, r)
        } else if let Some(r) = t.strip_prefix("[x]").or_else(|| t.strip_prefix("[X]")) {
            (true, r)
        } else {
            continue;
        };
        let label = rest.trim();
        if !label.is_empty() {
            steps.push(PlanStep {
                id: steps.len(),
                label: label.into(),
                status: if done { PlanStatus::Done } else { PlanStatus::Pending },
            });
        }
    }
    (!steps.is_empty()).then_some(steps)
}

/// `tool_result.content` is a string or an array of content blocks —
/// flatten to displayable text.
pub(crate) fn result_text(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    let Some(blocks) = content.as_array() else { return String::new() };
    blocks
        .iter()
        .filter_map(|b| match b["type"].as_str() {
            Some("text") => Some(b["text"].as_str().unwrap_or("").to_string()),
            Some("image") => Some("[image]".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `TodoWrite` input → plan steps. `activeForm` is the present-tense label
/// claude shows while a step runs — prefer it for in-progress rows.
pub(crate) fn todo_steps(input: &serde_json::Value) -> Vec<PlanStep> {
    let mut steps = plan_steps(&input["todos"], "content", "status");
    for (step, todo) in steps.iter_mut().zip(input["todos"].as_array().into_iter().flatten()) {
        if step.status == PlanStatus::InProgress
            && let Some(active) = todo["activeForm"].as_str().filter(|a| !a.is_empty())
        {
            step.label = active.into();
        }
    }
    steps
}
