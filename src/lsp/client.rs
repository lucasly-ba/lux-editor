//! The LSP client: spawning a language server and driving its lifecycle.
//!
//! A language server is an ordinary child process that speaks JSON-RPC over its
//! stdin/stdout. The client:
//! - spawns it (e.g. `rust-analyzer`),
//! - runs a background thread that reads framed messages into a channel,
//! - sends requests/notifications and matches responses by id.
//!
//! Notifications that arrive while we're waiting for a response (most
//! importantly `publishDiagnostics`) are stashed and handed to the editor via
//! [`LspClient::poll`].

use std::collections::VecDeque;
use std::io::{self, BufReader};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use super::json::Json;
use super::protocol::{self, CompletionItem, Diagnostic};
use super::transport::{read_message, write_message};

/// A running language server connection.
pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    incoming: Receiver<Json>,
    next_id: i64,
    /// Notifications / server requests seen while waiting for a response.
    stashed: VecDeque<Json>,
}

impl LspClient {
    /// Spawn `command` as a language server rooted at `root` and run the
    /// `initialize` handshake. Returns an error if the server can't be started.
    pub fn start(command: &str, root: &Path) -> io::Result<LspClient> {
        let mut child = Command::new(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");

        // Read messages off the server on a background thread so the editor
        // never blocks on the server.
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(message)) = read_message(&mut reader) {
                if tx.send(message).is_err() {
                    break; // the client was dropped
                }
            }
        });

        let mut client = LspClient {
            child,
            stdin,
            incoming: rx,
            next_id: 1,
            stashed: VecDeque::new(),
        };
        client.initialize(root)?;
        Ok(client)
    }

    fn initialize(&mut self, root: &Path) -> io::Result<()> {
        let id = self.send_request("initialize", protocol::initialize_params(root))?;
        // The spec requires waiting for the initialize result before sending
        // the `initialized` notification.
        let _ = self.wait_for_response(id, Duration::from_secs(10));
        self.send_notification("initialized", Json::object([] as [(&str, Json); 0]))
    }

    // --- outgoing -----------------------------------------------------------

    fn send(&mut self, message: Json) -> io::Result<()> {
        write_message(&mut self.stdin, &message)
    }

    fn send_request(&mut self, method: &str, params: Json) -> io::Result<i64> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(request_envelope(id, method, params))?;
        Ok(id)
    }

    fn send_notification(&mut self, method: &str, params: Json) -> io::Result<()> {
        self.send(notification_envelope(method, params))
    }

    /// Notify the server that a document was opened.
    pub fn did_open(
        &mut self,
        uri: &str,
        language_id: &str,
        version: i64,
        text: &str,
    ) -> io::Result<()> {
        self.send_notification(
            "textDocument/didOpen",
            protocol::did_open_params(uri, language_id, version, text),
        )
    }

    /// Notify the server that a document changed (full-document sync).
    pub fn did_change(&mut self, uri: &str, version: i64, text: &str) -> io::Result<()> {
        self.send_notification(
            "textDocument/didChange",
            protocol::did_change_params(uri, version, text),
        )
    }

    /// Request completions at a position, blocking up to `timeout`.
    pub fn completion(
        &mut self,
        uri: &str,
        line: usize,
        character: usize,
        timeout: Duration,
    ) -> Vec<CompletionItem> {
        let Ok(id) = self.send_request(
            "textDocument/completion",
            protocol::completion_params(uri, line, character),
        ) else {
            return Vec::new();
        };
        match self.wait_for_response(id, timeout) {
            Some(result) => protocol::parse_completion(&result),
            None => Vec::new(),
        }
    }

    // --- incoming -----------------------------------------------------------

    /// Block until the response with `id` arrives or `timeout` elapses.
    ///
    /// Diagnostics met along the way are stashed for [`poll_diagnostics`]
    /// (Self::poll_diagnostics); server requests are answered; anything else is
    /// dropped.
    fn wait_for_response(&mut self, id: i64, timeout: Duration) -> Option<Json> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let message = self.incoming.recv_timeout(remaining).ok()?;
            if message.get("id").and_then(Json::as_i64) == Some(id)
                && message.get("method").is_none()
            {
                // The response we were waiting for.
                return Some(message.get("result").cloned().unwrap_or(Json::Null));
            }
            if is_server_request(&message) {
                self.reply_null(&message);
            } else if is_publish_diagnostics(&message) {
                self.stashed.push_back(message);
            }
            // Everything else (logs, $/progress, other responses) is dropped.
        }
    }

    /// Return the next pending diagnostics notification, if any. Non-blocking;
    /// call once per frame from the editor loop.
    pub fn poll_diagnostics(&mut self) -> Option<(String, Vec<Diagnostic>)> {
        loop {
            let message = self
                .stashed
                .pop_front()
                .or_else(|| self.incoming.try_recv().ok())?;
            if is_publish_diagnostics(&message) {
                let params = message.get("params")?;
                let uri = params.get("uri")?.as_str()?.to_string();
                return Some((uri, protocol::parse_diagnostics(params)));
            }
            // Answer server requests so the server doesn't stall; *drop*
            // everything else. Critically, non-diagnostic notifications are not
            // re-stashed here; doing so would re-pop them forever.
            if is_server_request(&message) {
                self.reply_null(&message);
            }
        }
    }

    /// Reply to a server-to-client request with a null result. lux doesn't
    /// implement these, but answering keeps the server from blocking.
    fn reply_null(&mut self, request: &Json) {
        let id = request.get("id").cloned().unwrap_or(Json::Null);
        let _ = self.send(Json::object([
            ("jsonrpc", Json::from("2.0")),
            ("id", id),
            ("result", Json::Null),
        ]));
    }
}

/// A message with both an id and a method is a request *from* the server.
fn is_server_request(message: &Json) -> bool {
    message.get("id").is_some() && message.get("method").is_some()
}

/// Whether `message` is a `textDocument/publishDiagnostics` notification.
fn is_publish_diagnostics(message: &Json) -> bool {
    message.get("method").and_then(Json::as_str) == Some("textDocument/publishDiagnostics")
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Best-effort polite shutdown, then make sure the process is gone.
        let _ = self.send_notification("exit", Json::Null);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Build a JSON-RPC request envelope.
fn request_envelope(id: i64, method: &str, params: Json) -> Json {
    Json::object([
        ("jsonrpc", Json::from("2.0")),
        ("id", Json::from(id)),
        ("method", Json::from(method)),
        ("params", params),
    ])
}

/// Build a JSON-RPC notification envelope (a request with no id).
fn notification_envelope(method: &str, params: Json) -> Json {
    Json::object([
        ("jsonrpc", Json::from("2.0")),
        ("method", Json::from(method)),
        ("params", params),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_has_id_and_method() {
        let env = request_envelope(7, "initialize", Json::object([] as [(&str, Json); 0]));
        assert_eq!(env.get("jsonrpc").unwrap().as_str(), Some("2.0"));
        assert_eq!(env.get("id").unwrap().as_i64(), Some(7));
        assert_eq!(env.get("method").unwrap().as_str(), Some("initialize"));
        assert!(env.get("params").is_some());
    }

    #[test]
    fn notification_has_no_id() {
        let env = notification_envelope("initialized", Json::Null);
        assert!(env.get("id").is_none());
        assert_eq!(env.get("method").unwrap().as_str(), Some("initialized"));
    }

    #[test]
    fn classifies_messages() {
        // A request from the server has both id and method.
        let server_request = request_envelope(1, "workspace/configuration", Json::Null);
        assert!(is_server_request(&server_request));
        assert!(!is_publish_diagnostics(&server_request));

        // A plain notification (e.g. progress) has a method but no id, and must
        // NOT be treated as diagnostics. This is the case that used to loop.
        let progress = notification_envelope("$/progress", Json::Null);
        assert!(!is_server_request(&progress));
        assert!(!is_publish_diagnostics(&progress));

        let diag = notification_envelope("textDocument/publishDiagnostics", Json::Null);
        assert!(is_publish_diagnostics(&diag));

        // A response has an id but no method.
        let response = Json::object([("id", Json::from(1i64)), ("result", Json::Null)]);
        assert!(!is_server_request(&response));
    }
}
