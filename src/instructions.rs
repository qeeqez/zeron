//! Custom instructions: the global instructions field (Settings → Custom
//! Instructions, persisted to settings.json) merged with the project's own
//! instructions file, then handed to every backend turn via `TurnContext`.
//!
//! Merge order is global-then-project, matching real Codex: user
//! instructions first, then the project file (`AGENTS.md`, `CLAUDE.md`, or
//! `.rixl/instructions.md` — first present wins) appended after it.

/// Project instruction files probed at the project root, in precedence
/// order — the first one that exists and reads non-empty wins (a repo
/// rarely carries more than one, and concatenating would double-ship the
/// same guidance when files are symlinked together).
pub(crate) const PROJECT_FILES: [&str; 3] = ["AGENTS.md", "CLAUDE.md", ".rixl/instructions.md"];

/// The project's instructions file as `(name, trimmed text)`, or `None`
/// when no candidate exists or every candidate is empty/unreadable.
pub(crate) fn project_file(root: &std::path::Path) -> Option<(String, String)> {
    for name in PROJECT_FILES {
        if let Ok(text) = std::fs::read_to_string(root.join(name)) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some((name.to_string(), trimmed.to_string()));
            }
        }
    }
    None
}

/// Merge the global setting with the project's instructions file —
/// global first, project second, blank line between. `None` when both
/// are empty so backends can omit their instructions field entirely.
pub(crate) fn merge(global: &str, project: Option<&str>) -> Option<String> {
    let mut parts = Vec::new();
    let global = global.trim();
    if !global.is_empty() {
        parts.push(global);
    }
    if let Some(project) = project.map(str::trim).filter(|p| !p.is_empty()) {
        parts.push(project);
    }
    if parts.is_empty() { None } else { Some(parts.join("\n\n")) }
}

/// The merged instructions for one turn — the workspace's global setting
/// plus the project file under `root`.
pub(crate) fn for_turn(global: &str, root: &std::path::Path) -> Option<String> {
    merge(global, project_file(root).map(|(_, text)| text).as_deref())
}

/// `prompt` with `instructions` prepended as a `<system_instructions>`
/// block — for transports with no dedicated system channel (ACP's
/// `session/prompt`, the HTTP endpoint's `prompt` field). Returns the
/// prompt untouched when there are no instructions.
pub(crate) fn prefixed(prompt: &str, instructions: Option<&str>) -> String {
    match instructions.map(str::trim).filter(|i| !i.is_empty()) {
        Some(instructions) => format!("<system_instructions>\n{instructions}\n</system_instructions>\n\n{prompt}"),
        None => prompt.to_string(),
    }
}
