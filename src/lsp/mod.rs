//! A from-scratch **LSP client**.
//!
//! The [Language Server Protocol] lets an editor talk to a language server
//! (here, `rust-analyzer`) to get diagnostics, completions and more. The
//! protocol is JSON-RPC 2.0 carried over the server's stdin/stdout, with each
//! message framed by a `Content-Length` header.
//!
//! lux implements every layer of this itself:
//! - [`json`]: a hand-written JSON value, parser and serializer,
//! - [`transport`]: the `Content-Length` framing over any reader/writer,
//! - [`protocol`]: the message bodies and the types lux understands,
//! - [`client`]: spawning the server and the request/response lifecycle.
//!
//! [Language Server Protocol]: https://microsoft.github.io/language-server-protocol/

pub mod client;
pub mod json;
pub mod protocol;
pub mod transport;

pub use client::LspClient;
pub use protocol::{CompletionItem, Diagnostic, Severity};
