//! `SettingsPanel` MCP operations: the add-form inputs, list mutations
//! (add/remove/enable) that persist to settings.json + `config.toml`, and
//! the codex `mcpServerStatus/list` refresh. Split from `settings_mcp.rs`
//! to stay under the SLOC cap.

use gpui_kit::component::input::InputState;
use gpui_kit::component::select::SelectState;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::mcp::{McpServer, McpStatus, McpTransport};
use crate::views::settings::SettingsPanel;
use crate::views::settings_mcp::{McpInputs, TRANSPORTS};

impl SettingsPanel {
    /// Create the add-form inputs; the transport picker defaults to stdio.
    pub(crate) fn new_mcp_inputs(window: &mut Window, cx: &mut Context<Self>) -> McpInputs {
        McpInputs {
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Server name")),
            transport: cx.new(|cx| {
                SelectState::new(TRANSPORTS.map(str::to_string).to_vec(), Some(gpui_kit::component::IndexPath::new(0)), window, cx)
            }),
            command: cx.new(|cx| InputState::new(window, cx).placeholder("npx -y @scope/mcp-server  ·  https://…")),
            env: cx.new(|cx| InputState::new(window, cx).placeholder("KEY=value, OTHER=value")),
        }
    }

    /// Whether the selected provider is codex — only it can report
    /// `mcpServerStatus/list`.
    fn codex_active(&self, cx: &App) -> bool {
        self.ws.upgrade().and_then(|ws| {
            let ws = ws.read(cx);
            ws.provider_instances()
                .iter()
                .find(|p| Some(p.id.as_str()) == ws.selected_provider())
                .map(|p| p.kind)
        }) == Some(crate::providers::ProviderKind::CodexCli)
    }

    /// Fetch live server status off the UI thread. No-op in tests (the
    /// fetch spawns `codex app-server`), while a fetch runs, or when the
    /// active provider isn't codex.
    pub(crate) fn refresh_mcp_status(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) || self.mcp_status_loading || !self.codex_active(cx) {
            return;
        }
        self.mcp_status_loading = true;
        let task = cx.background_executor().spawn(async move { crate::backend::fetch_mcp_status() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| this.land_mcp_status(result, cx));
        })
        .detach();
    }

    /// Publish a fetched status list; errors clear it — the dots fall back
    /// to "status unknown".
    pub(crate) fn land_mcp_status(&mut self, result: Result<Vec<McpStatus>, String>, cx: &mut Context<Self>) {
        self.mcp_status_loading = false;
        self.mcp_status = result.unwrap_or_default().into_iter().map(|s| (s.name.clone(), s)).collect();
        cx.notify();
    }

    /// Add `server` to the list and persist. Err on empty fields or a
    /// duplicate name — the caller shows the message in the form.
    pub(crate) fn add_mcp_server(&mut self, server: McpServer) -> Result<(), String> {
        if server.name.is_empty() {
            return Err("Name is required".into());
        }
        if server.command.is_empty() {
            return Err(match server.transport {
                McpTransport::Stdio => "Command is required".into(),
                McpTransport::Http => "URL is required".into(),
            });
        }
        if self.mcp_servers.iter().any(|s| s.name == server.name) {
            return Err(format!("`{}` is already configured", server.name));
        }
        self.mcp_servers.push(server);
        self.persist_mcp();
        Ok(())
    }

    /// Remove a server by name and persist; its status entry goes too.
    pub(crate) fn remove_mcp_server(&mut self, name: &str) {
        self.mcp_servers.retain(|s| s.name != name);
        self.mcp_status.remove(name);
        self.persist_mcp();
    }

    /// Flip a server's enabled flag and persist — disabled servers are
    /// written `enabled = false` for codex and omitted from ACP sessions.
    pub(crate) fn set_mcp_enabled(&mut self, name: &str, on: bool) {
        let Some(s) = self.mcp_servers.iter_mut().find(|s| s.name == name) else { return };
        if s.enabled == on {
            return;
        }
        s.enabled = on;
        self.persist_mcp();
    }

    /// Write the list to settings.json + `~/.codex/config.toml` and
    /// re-render.
    fn persist_mcp(&mut self) {
        crate::mcp_config::save_servers(&self.mcp_servers);
    }

    /// Read the add form, add the server, and on success clear the inputs
    /// and close the form; on failure show the error line.
    pub(crate) fn confirm_mcp_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let http = self.mcp_inputs.transport.read(cx).selected_index(cx).is_some_and(|i| i.row == 1);
        let draft = McpServer {
            name: self.mcp_inputs.name.read(cx).value().trim().to_string(),
            transport: if http { McpTransport::Http } else { McpTransport::Stdio },
            command: self.mcp_inputs.command.read(cx).value().trim().to_string(),
            env: crate::mcp::parse_env(&self.mcp_inputs.env.read(cx).value()),
            enabled: true,
        };
        match self.add_mcp_server(draft) {
            Ok(()) => {
                for input in [&self.mcp_inputs.name, &self.mcp_inputs.command, &self.mcp_inputs.env] {
                    input.update(cx, |s, cx| s.set_value("", window, cx));
                }
                self.mcp_adding = false;
                self.mcp_error = None;
            },
            Err(e) => self.mcp_error = Some(e),
        }
        cx.notify();
    }
}
