//! JSON-RPC message framing.
//!
//! LSP messages are sent over a stream (the server's stdin/stdout) using the
//! same framing as HTTP: a `Content-Length` header, a blank line, then exactly
//! that many bytes of JSON body. This module reads and writes that frame over
//! any [`Read`]/[`Write`], which keeps it testable with in-memory buffers and
//! no real server needed.

use std::io::{self, BufRead, Write};

use super::json::Json;

/// Write `body` as a framed JSON-RPC message.
pub fn write_message(writer: &mut impl Write, body: &Json) -> io::Result<()> {
    let content = body.to_string();
    // Content-Length is the number of *bytes* in the body.
    write!(writer, "Content-Length: {}\r\n\r\n", content.len())?;
    writer.write_all(content.as_bytes())?;
    writer.flush()
}

/// Read one framed JSON-RPC message. Returns `Ok(None)` at end of stream.
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Json>> {
    let mut content_length: Option<usize> = None;

    // Read headers up to the blank line.
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None); // clean EOF
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // end of headers
        }
        if let Some((name, value)) = trimmed.split_once(':')
            && name.trim().eq_ignore_ascii_case("Content-Length")
        {
            content_length = value.trim().parse::<usize>().ok();
        }
    }

    let len = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;

    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    let text = String::from_utf8(body)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "message body is not UTF-8"))?;
    Json::parse(&text)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trips_a_message() {
        let msg = Json::object([("jsonrpc", Json::from("2.0")), ("id", Json::from(1i64))]);
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();

        // The frame should start with a Content-Length header.
        let text = String::from_utf8(buf.clone()).unwrap();
        assert!(text.starts_with("Content-Length: "));
        assert!(text.contains("\r\n\r\n"));

        let mut cursor = Cursor::new(buf);
        let read = read_message(&mut cursor).unwrap().unwrap();
        assert_eq!(read, msg);
    }

    #[test]
    fn reads_two_messages_back_to_back() {
        let a = Json::object([("id", Json::from(1i64))]);
        let b = Json::object([("id", Json::from(2i64))]);
        let mut buf = Vec::new();
        write_message(&mut buf, &a).unwrap();
        write_message(&mut buf, &b).unwrap();

        let mut cursor = Cursor::new(buf);
        assert_eq!(read_message(&mut cursor).unwrap().unwrap(), a);
        assert_eq!(read_message(&mut cursor).unwrap().unwrap(), b);
        assert_eq!(read_message(&mut cursor).unwrap(), None); // EOF
    }

    #[test]
    fn content_length_counts_bytes_not_chars() {
        // "é" is one char but two UTF-8 bytes; the frame must use the byte count.
        let msg = Json::object([("s", Json::from("héllo 😀"))]);
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(read_message(&mut cursor).unwrap().unwrap(), msg);
    }
}
