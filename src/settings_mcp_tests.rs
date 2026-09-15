//! Headless tests for the MCP Servers settings section: the server list,
//! the add form (stdio + http), enable/remove, validation errors, and the
//! codex status dots.

use gpui_kit::component::Root;
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::mcp::{McpServer, McpStatus, McpTransport};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME`/`CODEX_HOME`
/// redirected to a temp dir so settings and config.toml writes stay off
/// the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-mcp-settings-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other
    // thread can observe HOME/CODEX_HOME mid-write.
    unsafe {
        std::env::set_var("HOME", &dir);
        std::env::set_var("CODEX_HOME", dir.join("codex"));
    }
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

/// Open settings and switch to the MCP Servers section.
fn open_mcp(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.click("settings-btn", cx);
        window.draw(cx).clear(cx);
        window.click("settings-nav-mcp", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("settings-section-mcp").visible(), "mcp section should show");
    });
}

/// Type `value` into an `InputState` — the add form's fields.
fn fill(input: &Entity<InputState>, value: &str, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    input.update(cx, |s, cx| s.set_value(value, window, cx));
}

/// The panel's configured server names.
fn server_names(ws: &Entity<Workspace>, cx: &VisualTestContext) -> Vec<String> {
    ws.read_with(cx, |w, app| w.settings_panel.read(app).mcp_servers.iter().map(|s| s.name.clone()).collect())
}

#[test]
fn section_lists_servers_and_add_flow_works() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_mcp(cx);
    // Empty state, then the add form opens.
    cx.update(|window, cx| {
        assert!(window.find("mcp-list").visible());
        assert!(window.try_find("mcp-row-fs").is_none(), "no rows before adding");
        window.click("mcp-add", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("mcp-add-form").visible(), "add form should open");

        let inputs = ws.read(cx).settings_panel.read(cx).mcp_inputs.clone();
        fill(&inputs.name, "fs", window, cx);
        fill(&inputs.command, "mcp-fs --root /tmp", window, cx);
        fill(&inputs.env, "DEBUG=1", window, cx);
        window.click("mcp-confirm", cx);
        window.draw(cx).clear(cx);
    });
    assert_eq!(server_names(&ws, cx), ["fs"]);
    cx.update(|window, _cx| {
        assert!(window.find("mcp-row-fs").visible(), "new server row should render");
        assert!(window.try_find("mcp-add-form").is_none(), "form closes after add");
        // Codex is the default provider — the status dot renders.
        assert!(window.find("mcp-status-fs").visible(), "status dot should show");
    });
    // Persisted to settings.json and mirrored into config.toml.
    let loaded = crate::persist::load_settings();
    assert_eq!(loaded.mcp_servers.len(), 1);
    assert_eq!(loaded.mcp_servers[0].command, "mcp-fs --root /tmp");
    assert_eq!(loaded.mcp_servers[0].env["DEBUG"], "1");
    let dir = std::env::temp_dir().join(format!("rixlcode-mcp-settings-test-{}", std::process::id()));
    let toml = std::fs::read_to_string(dir.join("codex/config.toml")).unwrap();
    assert!(toml.contains("[mcp_servers.fs]"), "{toml}");
    assert!(toml.contains("args = [\"--root\", \"/tmp\"]"), "{toml}");
}

#[test]
fn add_form_supports_http_and_validates() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    open_mcp(cx);
    cx.update(|window, cx| {
        window.click("mcp-add", cx);
        window.draw(cx).clear(cx);
        // Empty fields → the error line shows, nothing is added.
        window.click("mcp-confirm", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("mcp-error").visible(), "empty add should error");
        assert!(ws.read(cx).settings_panel.read(cx).mcp_servers.is_empty());

        // Switch the transport picker to http and add a url server.
        let inputs = ws.read(cx).settings_panel.read(cx).mcp_inputs.clone();
        inputs.transport.update(cx, |s, cx| {
            s.set_selected_index(Some(gpui_kit::component::IndexPath::new(1)), window, cx);
        });
        fill(&inputs.name, "web", window, cx);
        fill(&inputs.command, "https://mcp.example.com/mcp", window, cx);
        fill(&inputs.env, "X-Key=k", window, cx);
        window.click("mcp-confirm", cx);
        window.draw(cx).clear(cx);
    });
    let servers = ws.read_with(cx, |w, app| w.settings_panel.read(app).mcp_servers.clone());
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].transport, McpTransport::Http);
    assert_eq!(servers[0].command, "https://mcp.example.com/mcp");
    // http servers write `url` + `http_headers`, not `command`/`env`.
    let dir = std::env::temp_dir().join(format!("rixlcode-mcp-settings-test-{}", std::process::id()));
    let toml = std::fs::read_to_string(dir.join("codex/config.toml")).unwrap();
    assert!(toml.contains("url = \"https://mcp.example.com/mcp\""), "{toml}");
    assert!(toml.contains("[mcp_servers.web.http_headers]"), "{toml}");

    // A duplicate name is rejected with the error line.
    cx.update(|window, cx| {
        window.click("mcp-add", cx);
        window.draw(cx).clear(cx);
        let inputs = ws.read(cx).settings_panel.read(cx).mcp_inputs.clone();
        fill(&inputs.name, "web", window, cx);
        fill(&inputs.command, "other-bin", window, cx);
        window.click("mcp-confirm", cx);
        window.draw(cx).clear(cx);
        assert!(window.find("mcp-error").visible(), "duplicate name should error");
    });
    assert_eq!(server_names(&ws, cx), ["web"]);
}

#[test]
fn enable_disable_and_remove_persist() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    // Seed one server directly on the panel.
    cx.update(|_, cx| {
        let panel = ws.read(cx).settings_panel.clone();
        panel.update(cx, |panel, _| {
            panel
                .add_mcp_server(McpServer {
                    name: "fs".into(),
                    command: "mcp-fs".into(),
                    ..Default::default()
                })
                .unwrap();
        });
    });
    open_mcp(cx);
    cx.update(|window, cx| {
        assert!(window.find("mcp-row-fs").visible());
        window.click("mcp-enable-fs", cx);
        window.draw(cx).clear(cx);
        assert!(!ws.read(cx).settings_panel.read(cx).mcp_servers[0].enabled);
        window.click("mcp-enable-fs", cx);
        window.draw(cx).clear(cx);
        assert!(ws.read(cx).settings_panel.read(cx).mcp_servers[0].enabled);
        window.click("mcp-remove-fs", cx);
        window.draw(cx).clear(cx);
        assert!(window.try_find("mcp-row-fs").is_none(), "row should unmount");
        assert!(ws.read(cx).settings_panel.read(cx).mcp_servers.is_empty());
    });
    assert!(crate::persist::load_settings().mcp_servers.is_empty());
}

#[test]
fn status_lands_on_rows() {
    let mut app = TestAppContext::single();
    let (ws, cx) = mount(&mut app);
    cx.update(|_, cx| {
        let panel = ws.read(cx).settings_panel.clone();
        panel.update(cx, |panel, _| {
            panel
                .add_mcp_server(McpServer {
                    name: "fs".into(),
                    command: "mcp-fs".into(),
                    ..Default::default()
                })
                .unwrap();
        });
    });
    open_mcp(cx);
    cx.update(|_, cx| {
        let panel = ws.read(cx).settings_panel.clone();
        panel.update(cx, |panel, cx| {
            panel.land_mcp_status(
                Ok(vec![McpStatus {
                    name: "fs".into(),
                    runtime: "connected".into(),
                    auth: "unsupported".into(),
                    tools: 3,
                    tools_error: None,
                }]),
                cx,
            );
        });
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.find("mcp-status-fs").visible(), "status dot should render");
        assert_eq!(ws.read(cx).settings_panel.read(cx).mcp_status["fs"].tools, 3);
    });
}
