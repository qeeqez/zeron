//! `claude auth status/login/logout` — the CLI's own auth surface, split
//! from `claude.rs` for the SLOC cap.

use std::sync::mpsc::Sender;

/// `claude auth status --json` → the instance's sign-in state. Blocking —
/// call off the UI thread.
pub(crate) fn auth_status() -> crate::auth::AuthState {
    match std::process::Command::new("claude").args(["auth", "status", "--json"]).output() {
        Ok(o) => parse_auth_status(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => crate::auth::AuthState::Unknown,
    }
}

/// Map `claude auth status --json` output to a state. `loggedIn` decides;
/// the detail prefers the account email/org, then the auth method.
pub(super) fn parse_auth_status(out: &str) -> crate::auth::AuthState {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(out) else {
        return crate::auth::AuthState::Unknown;
    };
    match v["loggedIn"].as_bool() {
        Some(true) => {
            let detail = ["email", "orgName", "subscriptionType", "authMethod"]
                .iter()
                .filter_map(|k| v[k].as_str())
                .find(|s| !s.is_empty())
                .unwrap_or("")
                .to_string();
            crate::auth::AuthState::SignedIn(detail)
        },
        Some(false) => crate::auth::AuthState::SignedOut,
        None => crate::auth::AuthState::Unknown,
    }
}

/// `claude auth logout` — clears the CLI's stored credentials.
pub(crate) fn logout() -> Result<(), String> {
    let out = std::process::Command::new("claude")
        .args(["auth", "logout"])
        .output()
        .map_err(|e| format!("claude spawn: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Start `claude auth login`: the CLI prints the OAuth URL, opens the
/// browser, then waits for the pasted code on stdin — the session's
/// `stdin` slot is how `submit_auth_code` answers it. The worker reports
/// `AuthEvent`s and re-probes status when the child exits.
pub(crate) fn login(tx: Sender<crate::auth::AuthEvent>) -> Result<crate::auth::LoginHandle, String> {
    let mut child = std::process::Command::new("claude")
        .args(["auth", "login"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("claude spawn: {e}"))?;
    let stdout = child.stdout.take().expect("piped");
    let stdin = std::sync::Arc::new(parking_lot::Mutex::new(child.stdin.take()));
    let slot = std::sync::Arc::new(parking_lot::Mutex::new(Some(child)));
    let worker_slot = slot.clone();
    std::thread::spawn(move || {
        pump_login(stdout, &tx);
        let _ = tx.send(crate::auth::AuthEvent::Done(auth_status()));
        if let Some(mut child) = worker_slot.lock().take() {
            let _ = child.wait();
        }
    });
    Ok(crate::auth::LoginHandle { child: slot, stdin: Some(stdin) })
}

/// Read the login child's stdout until EOF, emitting `NeedsCode` once the
/// paste-back prompt appears (the URL rides along when it printed first).
/// Byte-wise because the "Paste code here" prompt has no trailing newline.
/// Testable with in-memory readers.
pub(super) fn pump_login(stdout: impl std::io::Read, tx: &Sender<crate::auth::AuthEvent>) {
    use std::io::Read;
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let mut reader = std::io::BufReader::new(stdout);
    let mut asked = false;
    while reader.read(&mut byte).is_ok_and(|n| n == 1) {
        buf.push(byte[0]);
        // Scan on prompt-looking tails and periodically — the paste
        // prompt ends with "> " and no newline.
        if asked || (!buf.ends_with(b"> ") && buf.len() % 64 != 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&buf);
        if text.contains("Paste code") || text.contains("paste the code") {
            asked = true;
            let url = text.split_whitespace().find(|w| w.starts_with("https://")).unwrap_or("");
            let prompt = if url.is_empty() {
                "Paste the sign-in code below.".to_string()
            } else {
                format!("Open {url} to sign in, then paste the code below.")
            };
            let _ = tx.send(crate::auth::AuthEvent::NeedsCode(prompt));
        }
    }
}
