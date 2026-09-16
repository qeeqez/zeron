//! `model/list` over `codex app-server`: spawn, handshake, paginate.
//!
//! Separate from the turn transport in `codex.rs` — a catalog fetch needs
//! no thread, no turn, and no event decoding; it just walks `nextCursor`
//! pages and exits. Runs on a background executor thread from
//! `Workspace::refresh_model_catalogs`.

use std::io::{BufRead, Write};

use serde_json::Value;

use super::rpc::{initialize_req, model_list_req, parse_model_page};
use crate::model::ModelInfo;

/// Fetch the real model catalog from `codex app-server`. The instance's
/// Variables land on the spawned child (a custom `CODEX_HOME` or base-URL
/// override reaches the probe). Returns Err on spawn failure, handshake
/// error, timeout, or EOF mid-list — callers fall back to the
/// cached/static catalog.
pub fn fetch_codex_models(p: &crate::providers::ProviderInstance) -> Result<Vec<ModelInfo>, String> {
    super::sessions::exchange(&p.env, |stdin, stdout| read_catalog(stdin, stdout))
}

/// Drive the handshake then paginate `model/list` until `nextCursor` is
/// absent. `stdin`/`stdout` are the app-server's pipes (testable with
/// in-memory cursors).
fn read_catalog(stdin: &mut dyn Write, stdout: impl std::io::Read) -> Result<Vec<ModelInfo>, String> {
    let send = |stdin: &mut dyn Write, v: &Value| -> Result<(), String> { writeln!(stdin, "{v}").map_err(|e| format!("codex stdin: {e}")) };
    send(stdin, &initialize_req(1))?;

    let reader = std::io::BufReader::new(stdout);
    let mut models = Vec::new();
    // Request ids: 1 = initialize, 2.. = model/list pages.
    let mut req_id = 1i64;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        // Notifications and server-initiated requests carry `method`;
        // only responses to our requests advance the fetch.
        if msg.get("method").is_some() || msg.get("id").is_none() {
            continue;
        }
        if let Some(err) = msg.get("error") {
            let m = err["message"].as_str().unwrap_or("request failed");
            return Err(format!("codex: {m}"));
        }
        match msg["id"].as_i64() {
            Some(1) => {
                send(stdin, &serde_json::json!({"method": "initialized", "params": {}}))?;
                req_id += 1;
                send(stdin, &model_list_req(req_id, None))?;
            },
            Some(id) if id == req_id => {
                let (page, next) = parse_model_page(&msg["result"]);
                models.extend(page);
                match next {
                    Some(cursor) => {
                        req_id += 1;
                        send(stdin, &model_list_req(req_id, Some(&cursor)))?;
                    },
                    None => return Ok(models),
                }
            },
            _ => {},
        }
    }
    Err("codex closed stdout before model/list completed".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `read_catalog` a scripted stdout; returns (result, requests
    /// the script saw on stdin).
    fn run(script: &str) -> (Result<Vec<ModelInfo>, String>, String) {
        let mut stdin = Vec::new();
        let out = read_catalog(&mut stdin, std::io::Cursor::new(script.as_bytes().to_vec()));
        (out, String::from_utf8(stdin).unwrap())
    }

    #[test]
    fn catalog_paginates_until_cursor_exhausted() {
        let script = concat!(
            r#"{"id":1,"result":{"userAgent":"codex"}}"#,
            "\n",
            r#"{"id":2,"result":{"data":[{"id":"a","displayName":"A","hidden":false}],"nextCursor":"c1"}}"#,
            "\n",
            r#"{"id":3,"result":{"data":[{"id":"b","displayName":"B","hidden":false}],"nextCursor":null}}"#,
            "\n"
        );
        let (out, sent) = run(script);
        let ids: Vec<String> = out.unwrap().iter().map(|m| m.id.to_string()).collect();
        assert_eq!(ids, ["a", "b"]);
        // Second page request carries the cursor.
        assert!(sent.contains(r#""cursor":"c1""#), "stdin was: {sent}");
        assert!(sent.contains(r#""method":"initialized""#));
    }

    #[test]
    fn catalog_skips_notifications_and_server_requests() {
        let script = concat!(
            r#"{"id":1,"result":{}}"#,
            "\n",
            r#"{"method":"remoteControl/status/changed","params":{}}"#,
            "\n",
            r#"{"method":"item/commandExecution/requestApproval","id":99,"params":{}}"#,
            "\n",
            r#"{"id":2,"result":{"data":[{"id":"m","displayName":"M","hidden":false}]}}"#,
            "\n"
        );
        let (out, _) = run(script);
        assert_eq!(out.unwrap().len(), 1);
    }

    #[test]
    fn catalog_errors_on_rpc_error_and_eof() {
        let (out, _) = run("{\"id\":1,\"result\":{}}\n{\"id\":2,\"error\":{\"message\":\"nope\"}}\n");
        assert!(out.unwrap_err().contains("nope"));
        let (out, _) = run("{\"id\":1,\"result\":{}}\n");
        assert!(out.is_err(), "EOF before model/list must error");
    }
}
