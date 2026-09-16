//! MCP Servers settings section: the configured server list (status dot,
//! command/url, enable switch, remove), the inline add form (name +
//! stdio command or streamable-http url + env/headers), and the codex
//! `mcpServerStatus/list` refresh behind the status dots. State lives on
//! `SettingsPanel` — the workspace doesn't own MCP config.

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::mcp::{McpServer, McpStatus};
use crate::views::settings_provider_detail::field;
use crate::views::settings_sections::{SettingsView, group_label};

/// The add form's transport picker labels — index 0 = stdio, 1 = http.
pub(crate) const TRANSPORTS: [&str; 2] = ["Stdio (command)", "Streamable HTTP (url)"];

/// The add form's owned inputs — created once in `SettingsPanel::new` so
/// typed text survives re-renders.
#[derive(Clone)]
pub(crate) struct McpInputs {
    pub name: Entity<InputState>,
    pub transport: Entity<SelectState<Vec<String>>>,
    pub command: Entity<InputState>,
    pub env: Entity<InputState>,
}

/// The MCP Servers content pane: header + add button, the add form when
/// open, then one row per configured server.
pub(crate) fn mcp_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let codex = s.ws.read(cx).selected_provider().is_some_and(|id| {
        s.ws.read(cx).provider_instances().iter().find(|p| p.id.as_str() == id).map(|p| p.kind)
            == Some(crate::providers::ProviderKind::CodexCli)
    });
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .child(group_label("Configured servers", &s.search, cx))
                .child(div().flex_1())
                .when(codex, |d| {
                    d.child(
                        Button::new("mcp-refresh")
                            .icon(IconName::RotateCw)
                            .small()
                            .ghost()
                            .accessibility_label("Refresh server status")
                            .on_click({
                                let panel = s.panel.clone();
                                move |_, _, cx| panel.update(cx, |this, cx| this.refresh_mcp_status(cx))
                            }),
                    )
                })
                .child(Button::new("mcp-add").label("Add server").icon(IconName::Plus).small().outline().on_click({
                    let panel = s.panel.clone();
                    move |_, _, cx| {
                        panel.update(cx, |this, cx| {
                            this.mcp_adding = !this.mcp_adding;
                            this.mcp_error = None;
                            cx.notify();
                        });
                    }
                })),
        )
        .when(s.mcp_adding, |d| d.child(add_form(s, cx)))
        .child(server_list(s, codex, cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Saved to ~/.codex/config.toml for codex and sent to ACP agents on session start."),
        )
}

/// The inline add form: name, transport picker, command/url, env/headers,
/// then Add/Cancel. Labels follow the selected transport.
fn add_form(s: &SettingsView, cx: &App) -> impl IntoElement {
    let http = s.mcp_inputs.transport.read(cx).selected_index(cx).is_some_and(|i| i.row == 1);
    let panel = s.panel.clone();
    div()
        .id("mcp-add-form")
        .test_support()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .child(field("Name", Input::new(&s.mcp_inputs.name).id("mcp-name").appearance(true).into_any_element()))
        .child(field(
            "Transport",
            div()
                .w(px(240.))
                .child(Select::new(&s.mcp_inputs.transport).id("mcp-transport").small().appearance(true))
                .into_any_element(),
        ))
        .child(field(
            if http { "URL" } else { "Command" },
            Input::new(&s.mcp_inputs.command).id("mcp-command").appearance(true).into_any_element(),
        ))
        .child(field(
            if http { "Headers" } else { "Environment" },
            Input::new(&s.mcp_inputs.env).id("mcp-env").appearance(true).into_any_element(),
        ))
        .when_some(s.mcp_error.clone(), |d, e| {
            d.child(div().id("mcp-error").test_support().text_xs().text_color(cx.theme().danger).child(e))
        })
        .child(
            div()
                .flex()
                .gap_2()
                .child(Button::new("mcp-confirm").label("Add server").small().primary().on_click(move |_, window, cx| {
                    panel.update(cx, |this, cx| this.confirm_mcp_add(window, cx));
                }))
                .child(Button::new("mcp-cancel").label("Cancel").small().ghost().on_click({
                    let panel = s.panel.clone();
                    move |_, _, cx| {
                        panel.update(cx, |this, cx| {
                            this.mcp_adding = false;
                            this.mcp_error = None;
                            cx.notify();
                        });
                    }
                })),
        )
}

/// The server list — one row per configured server, or the empty state.
fn server_list(s: &SettingsView, codex: bool, cx: &App) -> impl IntoElement {
    let mut list = div().id("mcp-list").test_support().flex().flex_col().gap_1();
    if s.mcp_servers.is_empty() {
        list = list.child(
            div()
                .p_3()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("No MCP servers configured — add one to give the agent extra tools."),
        );
    }
    list.children(s.mcp_servers.iter().map(|srv| server_row(srv, s, codex, cx)))
}

/// One server row: status dot (codex only), name + command/url, enable
/// switch and a remove button.
fn server_row(srv: &McpServer, s: &SettingsView, codex: bool, cx: &App) -> impl IntoElement {
    let (name_t, name_rm) = (srv.name.clone(), srv.name.clone());
    let (panel_t, panel_rm) = (s.panel.clone(), s.panel.clone());
    let status = s.mcp_status.get(&srv.name);
    div()
        .id(SharedString::from(format!("mcp-row-{}", srv.name)))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .p_2()
        .rounded_md()
        .when_some(status_dot(&srv.name, status, codex, s.mcp_status_loading, cx), |d, dot| d.child(dot))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_xs().font_semibold().overflow_hidden().child(srv.name.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .child(subtitle(srv, status)),
                ),
        )
        .child(div().flex_1())
        .child(
            Switch::new(SharedString::from(format!("mcp-enable-{}", srv.name)))
                .checked(srv.enabled)
                .small()
                .accessibility_label(format!("Enable {}", srv.name))
                .on_click(move |on, _, cx| {
                    panel_t.update(cx, |this, cx| {
                        this.set_mcp_enabled(&name_t, *on);
                        cx.notify();
                    });
                }),
        )
        .child(icon_btn(&format!("mcp-remove-{}", srv.name), IconName::Trash, move |_, _, cx| {
            panel_rm.update(cx, |this, cx| {
                this.remove_mcp_server(&name_rm);
                cx.notify();
            });
        }))
}

/// The row's second line: the command/url plus a tool count when the
/// server is connected.
fn subtitle(srv: &McpServer, status: Option<&McpStatus>) -> String {
    match status {
        Some(st) if st.runtime == "connected" => format!("{} · {} tools", srv.command, st.tools),
        _ => srv.command.clone(),
    }
}

/// The status dot + label for a codex-reported server. `None` when codex
/// isn't the active provider (nothing to report) — a configured server
/// with no status yet shows "unknown".
fn status_dot(name: &str, status: Option<&McpStatus>, codex: bool, loading: bool, cx: &App) -> Option<AnyElement> {
    if !codex {
        return None;
    }
    let (color, label) = match status {
        _ if loading => (cx.theme().muted_foreground, "checking…".to_string()),
        Some(st) => status_style(st, cx),
        None => (cx.theme().muted_foreground, "unknown".to_string()),
    };
    Some(
        div()
            .id(SharedString::from(format!("mcp-status-{name}")))
            .test_support()
            .flex()
            .items_center()
            .gap_1()
            .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(color))
            .child(div().text_xs().text_color(color).child(label))
            .into_any_element(),
    )
}

/// (dot color, label) for one fetched status — runtime first, then the
/// tool-discovery error when the server is up but its catalog failed.
fn status_style(st: &McpStatus, cx: &App) -> (gpui_kit::Hsla, String) {
    let t = cx.theme();
    match st.runtime.as_str() {
        "connected" => (t.success, format!("connected · {} tools", st.tools)),
        "starting" | "notStarted" => (t.info, "starting".to_string()),
        "authenticationRequired" => (t.warning, "auth required".to_string()),
        "failed" | "cancelled" => (t.danger, st.tools_error.clone().unwrap_or_else(|| "failed".to_string())),
        "disabled" => (t.muted_foreground, "disabled".to_string()),
        _ if st.tools_error.is_some() => (t.danger, "tool discovery failed".to_string()),
        _ => (t.muted_foreground, "unknown".to_string()),
    }
}

/// A small clickable icon — the shared shape for remove controls.
fn icon_btn(id: &str, icon: IconName, on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_string()))
        .test_support()
        .cursor_pointer()
        .child(icon)
        .on_click(on_click)
}
