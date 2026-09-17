//! Tests for codex `account/*` auth: login-status parsing, `account/read`
//! mapping, and the device-code login drive — sibling file so
//! `codex_tests.rs` stays under the SLOC cap. No real subprocess is
//! ever spawned; canned transcripts drive the exchange.

// ── Auth: `account/*` wire shapes and the login drive ──

#[cfg(test)]
mod auth_tests {
    use serde_json::{Value, json};

    use crate::auth::{AuthEvent, AuthState};
    use crate::backend::codex::{login_drive, parse_account, parse_login_status};

    /// Feed `drive` a canned app-server transcript: each line the server
    /// would print, with the client's writes captured for assertions.
    fn drive(lines: &[&str]) -> (Result<AuthState, String>, Vec<AuthEvent>, String) {
        let mut stdin = Vec::new();
        let stdout = lines.join("\n").into_bytes();
        let (tx, rx) = std::sync::mpsc::channel();
        let result = login_drive(&mut stdin, std::io::Cursor::new(stdout), &tx);
        drop(tx);
        (result, rx.into_iter().collect(), String::from_utf8(stdin).unwrap())
    }

    #[test]
    fn login_status_parses_signed_in_and_out() {
        assert_eq!(parse_login_status("Logged in using ChatGPT\n"), AuthState::SignedIn("ChatGPT".into()));
        assert_eq!(parse_login_status("Logged in with API key\n"), AuthState::SignedIn("API key".into()));
        assert_eq!(parse_login_status("Not logged in\n"), AuthState::SignedOut);
        assert_eq!(parse_login_status(""), AuthState::Unknown);
        assert_eq!(parse_login_status("garbage"), AuthState::Unknown);
    }

    #[test]
    fn account_read_maps_plan_and_missing_account() {
        let chatgpt = parse_account(&json!({
            "account": {"type": "chatgpt", "email": "u@x.com", "planType": "plus"},
            "requiresOpenaiAuth": true,
        }));
        assert_eq!(chatgpt, AuthState::SignedIn("u@x.com · ChatGPT Plus".into()));
        let plan_only = parse_account(&json!({
            "account": {"type": "chatgpt", "email": null, "planType": "pro"},
            "requiresOpenaiAuth": true,
        }));
        assert_eq!(plan_only, AuthState::SignedIn("ChatGPT Pro".into()));
        let api_key = parse_account(&json!({"account": {"type": "apiKey"}, "requiresOpenaiAuth": true}));
        assert_eq!(api_key, AuthState::SignedIn("API key".into()));
        assert_eq!(parse_account(&json!({"account": null, "requiresOpenaiAuth": true})), AuthState::SignedOut);
        assert_eq!(parse_account(&json!({"account": null, "requiresOpenaiAuth": false})), AuthState::NotRequired);
    }

    #[test]
    fn login_surfaces_the_device_url_and_code() {
        let (result, events, sent) = drive(&[
            r#"{"id":1,"result":{}}"#,
            r#"{"id":2,"result":{"type":"chatgptDeviceCode","loginId":"l1","userCode":"ABCD-1234","verificationUrl":"https://auth.openai.com/codex/device"}}"#,
            r#"{"method":"account/login/completed","params":{"success":true,"loginId":"l1"}}"#,
            r#"{"id":3,"result":{"account":{"type":"chatgpt","email":"u@x.com","planType":"plus"},"requiresOpenaiAuth":true}}"#,
        ]);
        assert_eq!(result.unwrap(), AuthState::SignedIn("u@x.com · ChatGPT Plus".into()));
        let prompt = events.iter().find_map(|e| match e {
            AuthEvent::Prompt(t) => Some(t.clone()),
            _ => None,
        });
        let prompt = prompt.expect("the device prompt is surfaced");
        assert!(prompt.contains("https://auth.openai.com/codex/device") && prompt.contains("ABCD-1234"), "{prompt}");
        assert!(sent.contains(r#""method":"account/login/start""#), "stdin was: {sent}");
        assert!(sent.contains(r#""type":"chatgptDeviceCode""#), "stdin was: {sent}");
    }

    #[test]
    fn login_completed_failure_errors() {
        let (result, _events, _sent) = drive(&[
            r#"{"id":1,"result":{}}"#,
            r#"{"id":2,"result":{"type":"chatgptDeviceCode","loginId":"l1","userCode":"ABCD-1234","verificationUrl":"https://x"}}"#,
            r#"{"method":"account/login/completed","params":{"success":false,"error":"expired"}}"#,
        ]);
        assert_eq!(result.unwrap_err(), "expired");
    }

    #[test]
    fn login_eof_before_completion_errors() {
        let (result, _events, _sent) = drive(&[r#"{"id":1,"result":{}}"#]);
        assert!(result.unwrap_err().contains("closed stdout"));
    }

    #[test]
    fn login_writes_initialize_and_start() {
        let mut stdin = Vec::new();
        let stdout = std::io::Cursor::new(b"{\"id\":1,\"result\":{}}\n{\"id\":2,\"result\":{\"type\":\"apiKey\"}}\n{\"id\":3,\"result\":{\"account\":{\"type\":\"apiKey\"},\"requiresOpenaiAuth\":true}}\n".to_vec());
        let (tx, _rx) = std::sync::mpsc::channel();
        let result = login_drive(&mut stdin, stdout, &tx);
        assert_eq!(result.unwrap(), AuthState::SignedIn("API key".into()));
        let sent = String::from_utf8(stdin).unwrap();
        let methods: Vec<String> = sent
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| v["method"].as_str().map(str::to_string))
            .collect();
        assert_eq!(methods, ["initialize", "initialized", "account/login/start", "account/read"]);
    }
}
