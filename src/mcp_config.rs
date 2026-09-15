//! `~/.codex/config.toml` `[mcp_servers]` I/O: the writer that hands the
//! configured servers to `codex app-server`, the reader that imports
//! servers declared there into settings, and the TOML quoting helpers.
//! Split from `mcp.rs` to stay under the SLOC cap.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::mcp::{McpServer, McpTransport, split_command};

/// Persist the MCP server list: settings.json is the source of truth, and
/// `~/.codex/config.toml` `[mcp_servers]` is rewritten to match so the
/// next `codex app-server` spawn sees the same set.
pub(crate) fn save_servers(servers: &[McpServer]) {
    let mut s = crate::persist::load_settings();
    s.mcp_servers = servers.to_vec();
    crate::persist::save_settings(&s);
    write_codex_config(servers);
}

/// Re-quote argv for the single-line command field — args containing
/// whitespace get their double quotes back.
fn join_command(argv: &[String]) -> String {
    argv.iter()
        .map(|a| if a.chars().any(char::is_whitespace) { format!("\"{a}\"") } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `~/.codex/config.toml` — honors `CODEX_HOME` like the CLI does.
pub fn codex_config_path() -> PathBuf {
    let home = std::env::var("CODEX_HOME").map_or_else(|_| crate::persist::dirs_home().join(".codex"), PathBuf::from);
    home.join("config.toml")
}

/// Merge servers declared directly in `~/.codex/config.toml` into the
/// settings list — the UI edits the union and writes the whole set back,
/// so a save can't drop a server it never showed.
pub(crate) fn import_codex_servers(s: &mut crate::persist::Settings) {
    for srv in codex_config_servers() {
        if !s.mcp_servers.iter().any(|m| m.name == srv.name) {
            s.mcp_servers.push(srv);
        }
    }
}

/// Servers declared in `~/.codex/config.toml` — imported into settings on
/// load so the UI manages the full set and a save can't drop them.
pub(crate) fn codex_config_servers() -> Vec<McpServer> {
    std::fs::read_to_string(codex_config_path()).map_or_else(|_| Vec::new(), |t| parse_codex_config(&t))
}

/// Rewrite the `[mcp_servers]` block of `~/.codex/config.toml` to match
/// `servers`, leaving every other setting untouched. Called whenever the
/// settings list changes so the next `codex app-server` spawn picks it up.
pub(crate) fn write_codex_config(servers: &[McpServer]) {
    let path = codex_config_path();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = strip_mcp_tables(&existing);
    for s in servers {
        out.push_str(&render_server(s));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("toml.tmp");
    if std::fs::write(&tmp, out).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Drop every `[mcp_servers…]` table (and its body) from `text` — the
/// managed block is re-rendered wholesale. Other tables pass through.
fn strip_mcp_tables(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut skipping = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            skipping = t.starts_with("[mcp_servers]") || t.starts_with("[mcp_servers.") || t.starts_with("[[mcp_servers.");
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    // Trim the blank tail a removed block may leave — the render appends
    // its own leading newline per server.
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// One server's `[mcp_servers.<name>]` table. The env/http_headers
/// sub-table goes last — keys written after a sub-header would land in it.
fn render_server(s: &McpServer) -> String {
    let mut out = format!("\n[mcp_servers.{}]\n", toml_key(&s.name));
    match s.transport {
        McpTransport::Stdio => {
            let argv = split_command(&s.command);
            if let Some((cmd, args)) = argv.split_first() {
                out.push_str(&format!("command = {}\n", toml_str(cmd)));
                if !args.is_empty() {
                    let args = args.iter().map(|a| toml_str(a)).collect::<Vec<_>>().join(", ");
                    out.push_str(&format!("args = [{args}]\n"));
                }
            }
        },
        McpTransport::Http => out.push_str(&format!("url = {}\n", toml_str(&s.command))),
    }
    if !s.enabled {
        out.push_str("enabled = false\n");
    }
    if !s.env.is_empty() {
        let sub = match s.transport {
            McpTransport::Stdio => "env",
            McpTransport::Http => "http_headers",
        };
        out.push_str(&format!("\n[mcp_servers.{}.{sub}]\n", toml_key(&s.name)));
        for (k, v) in &s.env {
            out.push_str(&format!("{} = {}\n", toml_key(k), toml_str(v)));
        }
    }
    out
}

/// Read the `[mcp_servers.*]` tables out of config.toml text.
pub(crate) fn parse_codex_config(text: &str) -> Vec<McpServer> {
    #[derive(Default)]
    struct Raw {
        command: String,
        args: Vec<String>,
        url: String,
        env: BTreeMap<String, String>,
        enabled: bool,
    }
    let mut order: Vec<String> = Vec::new();
    let mut map: std::collections::HashMap<String, Raw> = std::collections::HashMap::new();
    // (server name, inside its env/http_headers sub-table)
    let mut current: Option<(String, bool)> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            current = parse_mcp_header(t);
            continue;
        }
        let Some((name, in_env)) = &current else { continue };
        let Some((k, v)) = t.split_once('=') else { continue };
        let raw = map.entry(name.clone()).or_insert_with(|| {
            order.push(name.clone());
            // `enabled` defaults true in codex — only an explicit
            // `enabled = false` disables.
            Raw { enabled: true, ..Default::default() }
        });
        let (k, v) = (k.trim(), v.trim());
        if *in_env {
            raw.env.insert(unquote(k), unquote(v));
            continue;
        }
        match k {
            "command" => raw.command = unquote(v),
            "args" => raw.args = parse_str_array(v),
            "url" => raw.url = unquote(v),
            "enabled" => raw.enabled = v != "false",
            _ => {},
        }
    }
    order
        .into_iter()
        .map(|name| {
            let raw = map.remove(&name).unwrap_or_default();
            if raw.url.is_empty() {
                let mut argv = vec![raw.command];
                argv.extend(raw.args);
                McpServer {
                    name,
                    transport: McpTransport::Stdio,
                    command: join_command(&argv),
                    env: raw.env,
                    enabled: raw.enabled,
                }
            } else {
                McpServer {
                    name,
                    transport: McpTransport::Http,
                    command: raw.url,
                    env: raw.env,
                    enabled: raw.enabled,
                }
            }
        })
        .collect()
}

/// `[mcp_servers.<name>]` → (name, false); `[mcp_servers.<name>.env]` /
/// `.http_headers` → (name, true). Anything else → not ours.
fn parse_mcp_header(t: &str) -> Option<(String, bool)> {
    let inner = t.strip_prefix("[mcp_servers.")?.strip_suffix(']')?;
    let (name, is_env) = if let Some(n) = inner.strip_suffix(".env") {
        (n, true)
    } else if let Some(n) = inner.strip_suffix(".http_headers") {
        (n, true)
    } else {
        (inner, false)
    };
    Some((unquote(name.trim()), is_env))
}

/// A TOML key — bare when it's already a legal bare key, quoted otherwise.
fn toml_key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        k.to_string()
    } else {
        toml_str(k)
    }
}

/// A TOML basic string — always quoted, `"` and `\` escaped.
fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Strip TOML basic-string quotes and unescape; bare values pass through.
fn unquote(v: &str) -> String {
    let v = v.trim();
    let Some(inner) = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else { return v.to_string() };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next().unwrap_or('\\')),
            c => out.push(c),
        }
    }
    out
}

/// `["a", "b"]` → the unquoted strings; anything else → empty.
fn parse_str_array(v: &str) -> Vec<String> {
    let Some(inner) = v.trim().strip_prefix('[').and_then(|v| v.strip_suffix(']')) else { return Vec::new() };
    let mut out = Vec::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => s.push(chars.next().unwrap_or('\\')),
                '"' => break,
                c => s.push(c),
            }
        }
        out.push(s);
    }
    out
}
