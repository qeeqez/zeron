//! `backend_for` — build the `AgentBackend` for one provider instance —
//! plus the shared env injection every subprocess spawn uses. Split from
//! `backend.rs` to stay under the SLOC cap.

use super::{AcpBackend, AgentBackend, ClaudeCliBackend, CodexCliBackend, HttpBackend, McpBackend, OllamaBackend, SimBackend};

/// Build the backend for one provider instance — `command`/`key_env` carry
/// the kind-specific connection fields (acp spawn command, http endpoint)
/// and `env` lands on every subprocess the backend spawns.
pub fn backend_for(p: &crate::providers::ProviderInstance) -> std::sync::Arc<dyn AgentBackend> {
    use crate::providers::ProviderKind;
    let env = p.env.clone();
    match p.kind {
        ProviderKind::CodexCli => std::sync::Arc::new(CodexCliBackend::new(env)),
        ProviderKind::ClaudeCli => std::sync::Arc::new(ClaudeCliBackend::new(env)),
        ProviderKind::Acp => std::sync::Arc::new(AcpBackend::new(p.command.clone(), env)),
        ProviderKind::Mcp => std::sync::Arc::new(McpBackend::new(p.command.clone(), env)),
        ProviderKind::Http => std::sync::Arc::new(HttpBackend::new(p.command.clone(), p.key_env.clone(), env)),
        ProviderKind::Ollama => std::sync::Arc::new(OllamaBackend::new(p.command.clone())),
        ProviderKind::Sim => std::sync::Arc::new(SimBackend),
    }
}

/// Apply an instance's Variables to a spawned `Command`. Blank keys are
/// half-edited rows — skipped; keys are trimmed so stray whitespace can't
/// silently set a differently-named variable.
pub(crate) fn apply_env(cmd: &mut std::process::Command, env: &[(String, String)]) {
    for (k, v) in env {
        let k = k.trim();
        if !k.is_empty() {
            cmd.env(k, v);
        }
    }
}
