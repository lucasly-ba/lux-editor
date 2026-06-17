//! LSP message bodies and the handful of types lux understands.
//!
//! This builds the JSON-RPC requests/notifications lux sends (`initialize`,
//! `didOpen`, `didChange`, `completion`) and parses the two responses it cares
//! about: published diagnostics and completion items. It is deliberately a
//! small slice of the protocol: enough to be useful and to show the shape of
//! it, not a complete implementation.

use std::path::Path;

use super::json::Json;

/// Diagnostic severity, mirroring the LSP integer codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    fn from_code(code: i64) -> Severity {
        match code {
            1 => Severity::Error,
            2 => Severity::Warning,
            3 => Severity::Information,
            _ => Severity::Hint,
        }
    }

    /// A one-letter tag for the status line.
    pub fn tag(self) -> char {
        match self {
            Severity::Error => 'E',
            Severity::Warning => 'W',
            Severity::Information => 'I',
            Severity::Hint => 'H',
        }
    }
}

/// A diagnostic (error/warning) reported by the server, in 0-based line/column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: usize,
    pub character: usize,
    pub severity: Severity,
    pub message: String,
}

/// A completion suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
}

/// Turn a filesystem path into a `file://` URI.
pub fn path_to_uri(path: &Path) -> String {
    let s = path.to_string_lossy();
    // Percent-encode the characters that matter for a path URI.
    let mut encoded = String::from("file://");
    for ch in s.chars() {
        match ch {
            ' ' => encoded.push_str("%20"),
            '#' => encoded.push_str("%23"),
            '?' => encoded.push_str("%3F"),
            c => encoded.push(c),
        }
    }
    encoded
}

/// Build the `initialize` request params.
pub fn initialize_params(root: &Path) -> Json {
    Json::object([
        ("processId", Json::from(std::process::id() as i64)),
        ("rootUri", Json::from(path_to_uri(root))),
        (
            "capabilities",
            Json::object([(
                "textDocument",
                Json::object([
                    (
                        "synchronization",
                        Json::object([("didSave", Json::from(true))]),
                    ),
                    (
                        "completion",
                        Json::object([("completionItem", Json::object([] as [(&str, Json); 0]))]),
                    ),
                    (
                        "publishDiagnostics",
                        Json::object([("relatedInformation", Json::from(false))]),
                    ),
                ]),
            )]),
        ),
    ])
}

/// Build a `textDocument/didOpen` notification's params.
pub fn did_open_params(uri: &str, language_id: &str, version: i64, text: &str) -> Json {
    Json::object([(
        "textDocument",
        Json::object([
            ("uri", Json::from(uri)),
            ("languageId", Json::from(language_id)),
            ("version", Json::from(version)),
            ("text", Json::from(text)),
        ]),
    )])
}

/// Build a `textDocument/didChange` notification's params (full-document sync).
pub fn did_change_params(uri: &str, version: i64, text: &str) -> Json {
    Json::object([
        (
            "textDocument",
            Json::object([("uri", Json::from(uri)), ("version", Json::from(version))]),
        ),
        (
            "contentChanges",
            Json::from(vec![Json::object([("text", Json::from(text))])]),
        ),
    ])
}

/// Build a `textDocument/completion` request's params at a 0-based position.
pub fn completion_params(uri: &str, line: usize, character: usize) -> Json {
    Json::object([
        ("textDocument", Json::object([("uri", Json::from(uri))])),
        (
            "position",
            Json::object([
                ("line", Json::from(line as i64)),
                ("character", Json::from(character as i64)),
            ]),
        ),
    ])
}

/// Parse a `textDocument/publishDiagnostics` notification into diagnostics.
pub fn parse_diagnostics(params: &Json) -> Vec<Diagnostic> {
    let Some(items) = params.get("diagnostics").and_then(Json::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|d| {
            let start = d.pointer(&["range", "start"])?;
            Some(Diagnostic {
                line: start.get("line")?.as_i64()? as usize,
                character: start.get("character")?.as_i64()? as usize,
                severity: d
                    .get("severity")
                    .and_then(Json::as_i64)
                    .map(Severity::from_code)
                    .unwrap_or(Severity::Error),
                message: d.get("message")?.as_str()?.to_string(),
            })
        })
        .collect()
}

/// Parse a completion response (either a bare array or a `CompletionList`).
pub fn parse_completion(result: &Json) -> Vec<CompletionItem> {
    let items = match result {
        Json::Array(items) => items.as_slice(),
        Json::Object(_) => result.get("items").and_then(Json::as_array).unwrap_or(&[]),
        _ => &[],
    };
    items
        .iter()
        .filter_map(|item| {
            Some(CompletionItem {
                label: item.get("label")?.as_str()?.to_string(),
                detail: item
                    .get("detail")
                    .and_then(Json::as_str)
                    .map(str::to_string),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_path_uri() {
        assert_eq!(
            path_to_uri(Path::new("/home/u/a b.rs")),
            "file:///home/u/a%20b.rs"
        );
    }

    #[test]
    fn parses_published_diagnostics() {
        let params = Json::parse(
            r#"{"uri":"file:///x.rs","diagnostics":[
                {"range":{"start":{"line":3,"character":4},"end":{"line":3,"character":9}},
                 "severity":1,"message":"cannot find value"}]}"#,
        )
        .unwrap();
        let diags = parse_diagnostics(&params);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 3);
        assert_eq!(diags[0].character, 4);
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].message, "cannot find value");
    }

    #[test]
    fn parses_completion_list_and_array() {
        let list = Json::parse(r#"{"items":[{"label":"push","detail":"fn"}]}"#).unwrap();
        let array = Json::parse(r#"[{"label":"pop"}]"#).unwrap();
        assert_eq!(parse_completion(&list)[0].label, "push");
        assert_eq!(parse_completion(&list)[0].detail.as_deref(), Some("fn"));
        assert_eq!(parse_completion(&array)[0].label, "pop");
        assert_eq!(parse_completion(&array)[0].detail, None);
    }

    #[test]
    fn did_change_carries_full_text() {
        let params = did_change_params("file:///x.rs", 2, "new text");
        let changes = params.get("contentChanges").unwrap().as_array().unwrap();
        assert_eq!(changes[0].get("text").unwrap().as_str(), Some("new text"));
        assert_eq!(
            params
                .pointer(&["textDocument", "version"])
                .unwrap()
                .as_i64(),
            Some(2)
        );
    }
}
