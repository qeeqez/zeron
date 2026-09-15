//! Tests for the `thread/list`/`thread/resume` drivers — sibling file so
//! `sessions.rs` stays under the SLOC cap. No real subprocess is spawned;
//! the drivers run against scripted stdout and record what they wrote to
//! stdin.

#[cfg(test)]
mod tests {
    use crate::backend::sessions::{read_session, read_sessions};
    use crate::model::{MessageKind, Role, ToolStatus};

    /// Feed `read_sessions` a scripted stdout; returns (result, requests
    /// the script saw on stdin).
    fn run_list(script: &str) -> (Result<Vec<crate::backend::SessionInfo>, String>, String) {
        let mut stdin = Vec::new();
        let out = read_sessions(&mut stdin, std::io::Cursor::new(script.as_bytes().to_vec()));
        (out, String::from_utf8(stdin).unwrap())
    }

    /// Feed `read_session` a scripted stdout for thread `tid`.
    fn run_resume(script: &str, tid: &str) -> (Result<crate::backend::ResumedSession, String>, String) {
        let mut stdin = Vec::new();
        let out = read_session(&mut stdin, std::io::Cursor::new(script.as_bytes().to_vec()), tid);
        (out, String::from_utf8(stdin).unwrap())
    }

    #[test]
    fn sessions_paginate_until_cursor_exhausted() {
        let script = concat!(
            r#"{"id":1,"result":{"userAgent":"codex"}}"#,
            "\n",
            r#"{"id":2,"result":{"data":[{"id":"t1","preview":"first","updatedAt":10,"cwd":"/a","ephemeral":false}],"nextCursor":"c1"}}"#,
            "\n",
            r#"{"id":3,"result":{"data":[{"id":"t2","preview":"second","updatedAt":9,"cwd":"/b","ephemeral":false}],"nextCursor":null}}"#,
            "\n"
        );
        let (out, sent) = run_list(script);
        let ids: Vec<String> = out.unwrap().iter().map(|s| s.id.clone()).collect();
        assert_eq!(ids, ["t1", "t2"]);
        // Second page request carries the cursor.
        assert!(sent.contains(r#""cursor":"c1""#), "stdin was: {sent}");
        assert!(sent.contains(r#""method":"initialized""#));
        assert!(sent.contains(r#""method":"thread/list""#));
    }

    #[test]
    fn sessions_skip_notifications_and_server_requests() {
        let script = concat!(
            r#"{"id":1,"result":{}}"#,
            "\n",
            r#"{"method":"remoteControl/status/changed","params":{}}"#,
            "\n",
            r#"{"method":"item/commandExecution/requestApproval","id":99,"params":{}}"#,
            "\n",
            r#"{"id":2,"result":{"data":[{"id":"t","preview":"p","updatedAt":1,"cwd":"/","ephemeral":false}]}}"#,
            "\n"
        );
        let (out, _) = run_list(script);
        assert_eq!(out.unwrap().len(), 1);
    }

    #[test]
    fn sessions_error_on_rpc_error_and_eof() {
        let (out, _) = run_list("{\"id\":1,\"result\":{}}\n{\"id\":2,\"error\":{\"message\":\"nope\"}}\n");
        assert!(out.unwrap_err().contains("nope"));
        let (out, _) = run_list("{\"id\":1,\"result\":{}}\n");
        assert!(out.is_err(), "EOF before thread/list must error");
    }

    #[test]
    fn resume_sends_thread_resume_and_maps_history() {
        let script = concat!(
            r#"{"id":1,"result":{}}"#,
            "\n",
            r#"{"id":2,"result":{"thread":{"id":"tid-9","preview":"old chat","cwd":"/proj","turns":["#,
            r#"{"items":["#,
            r#"{"type":"userMessage","id":"u1","content":[{"type":"text","text":"hello there"}]},"#,
            r#"{"type":"agentMessage","id":"a1","text":"hi back"},"#,
            r#"{"type":"commandExecution","id":"c1","command":"ls -la","aggregatedOutput":"file.rs","status":"completed"},"#,
            r#"{"type":"reasoning","id":"r1","summary":[],"content":[]}"#,
            r#"]}]"#,
            r#"}}}"#,
            "\n"
        );
        let (out, sent) = run_resume(script, "tid-9");
        assert!(sent.contains(r#""method":"thread/resume""#), "stdin was: {sent}");
        assert!(sent.contains(r#""threadId":"tid-9""#), "stdin was: {sent}");
        let session = out.unwrap();
        assert_eq!(session.id, "tid-9");
        assert_eq!(session.title, "old chat");
        assert_eq!(session.cwd, "/proj");
        assert_eq!(session.messages.len(), 3, "empty reasoning drops out");
        assert!(matches!(&session.messages[0].kind, MessageKind::Text(t) if t == "hello there"));
        assert_eq!(session.messages[0].role, Role::User);
        assert!(matches!(&session.messages[1].kind, MessageKind::Text(t) if t == "hi back"));
        let MessageKind::Tool(tool) = &session.messages[2].kind else { panic!("expected tool card") };
        assert_eq!(tool.name.as_ref(), "shell");
        assert_eq!(tool.detail.as_ref(), "ls -la");
        assert_eq!(tool.output.as_ref(), "file.rs");
        assert_eq!(tool.status, ToolStatus::Done);
    }

    #[test]
    fn resume_errors_on_rpc_error() {
        let (out, _) = run_resume("{\"id\":1,\"result\":{}}\n{\"id\":2,\"error\":{\"message\":\"invalid session id\"}}\n", "bad");
        assert!(out.err().unwrap().contains("invalid session id"));
    }
}
