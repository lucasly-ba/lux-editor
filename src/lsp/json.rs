//! A small, dependency-free JSON implementation.
//!
//! The LSP client speaks JSON-RPC, so it needs to read and write JSON. Rather
//! than pull in `serde_json`, lux implements just enough JSON by hand: a value
//! type, a recursive-descent parser and a serializer. It is a self-contained,
//! well-tested piece, and writing it (rather than importing it) is part of the
//! point: it shows the wire format is understood, not just used.

use std::collections::BTreeMap;
use std::fmt::Write;

/// A JSON value.
///
/// Objects keep their keys in a `BTreeMap`, which is enough for LSP (key order
/// is not significant) and makes lookups and equality simple.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    /// Build an object from key/value pairs.
    pub fn object<I, K>(pairs: I) -> Json
    where
        I: IntoIterator<Item = (K, Json)>,
        K: Into<String>,
    {
        Json::Object(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// Look up a key in an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(map) => map.get(key),
            _ => None,
        }
    }

    /// Follow a path of object keys, e.g. `pointer(&["params", "uri"])`.
    pub fn pointer(&self, path: &[&str]) -> Option<&Json> {
        let mut node = self;
        for key in path {
            node = node.get(key)?;
        }
        Some(node)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Number(n) if n.fract() == 0.0 => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Number(n) => {
                // Emit integers without a trailing ".0".
                if n.fract() == 0.0 && n.is_finite() {
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    let _ = write!(out, "{n}");
                }
            }
            Json::Str(s) => write_json_string(s, out),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Object(map) => {
                out.push('{');
                for (i, (key, value)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }

    /// Parse a JSON document, returning an error message on malformed input.
    pub fn parse(input: &str) -> Result<Json, String> {
        let mut parser = Parser {
            chars: input.chars().collect(),
            pos: 0,
        };
        parser.skip_whitespace();
        let value = parser.parse_value()?;
        parser.skip_whitespace();
        if parser.pos != parser.chars.len() {
            return Err("trailing characters after JSON value".to_string());
        }
        Ok(value)
    }
}

/// Compact JSON serialization. Implementing `Display` gives a `to_string()`
/// for free and is what callers (the transport) use.
impl std::fmt::Display for Json {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = String::new();
        self.write(&mut out);
        f.write_str(&out)
    }
}

// Convenient constructors so request bodies read naturally.
impl From<&str> for Json {
    fn from(s: &str) -> Json {
        Json::Str(s.to_string())
    }
}
impl From<String> for Json {
    fn from(s: String) -> Json {
        Json::Str(s)
    }
}
impl From<i64> for Json {
    fn from(n: i64) -> Json {
        Json::Number(n as f64)
    }
}
impl From<bool> for Json {
    fn from(b: bool) -> Json {
        Json::Bool(b)
    }
}
impl From<Vec<Json>> for Json {
    fn from(items: Vec<Json>) -> Json {
        Json::Array(items)
    }
}

/// Write `s` as a quoted, escaped JSON string.
fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A tiny recursive-descent JSON parser over a vector of characters.
struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self) -> Result<Json, String> {
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => Ok(Json::Str(self.parse_string()?)),
            Some('t' | 'f') => self.parse_bool(),
            Some('n') => self.parse_null(),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            Some(c) => Err(format!("unexpected character '{c}'")),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.bump() == Some(c) {
            Ok(())
        } else {
            Err(format!("expected '{c}'"))
        }
    }

    fn parse_object(&mut self) -> Result<Json, String> {
        self.expect('{')?;
        let mut map = BTreeMap::new();
        self.skip_whitespace();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Json::Object(map));
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(':')?;
            self.skip_whitespace();
            let value = self.parse_value()?;
            map.insert(key, value);
            self.skip_whitespace();
            match self.bump() {
                Some(',') => continue,
                Some('}') => break,
                _ => return Err("expected ',' or '}' in object".to_string()),
            }
        }
        Ok(Json::Object(map))
    }

    fn parse_array(&mut self) -> Result<Json, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.parse_value()?);
            self.skip_whitespace();
            match self.bump() {
                Some(',') => continue,
                Some(']') => break,
                _ => return Err("expected ',' or ']' in array".to_string()),
            }
        }
        Ok(Json::Array(items))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut s = String::new();
        loop {
            match self.bump() {
                None => return Err("unterminated string".to_string()),
                Some('"') => break,
                Some('\\') => {
                    let escaped = self.bump().ok_or("unterminated escape")?;
                    match escaped {
                        '"' => s.push('"'),
                        '\\' => s.push('\\'),
                        '/' => s.push('/'),
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        'b' => s.push('\x08'),
                        'f' => s.push('\x0c'),
                        'u' => s.push(self.parse_unicode_escape()?),
                        other => return Err(format!("invalid escape '\\{other}'")),
                    }
                }
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    /// Parse the four hex digits after `\u`, combining UTF-16 surrogate pairs.
    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let high = self.parse_hex4()?;
        if (0xD800..=0xDBFF).contains(&high) {
            // High surrogate: must be followed by `\uXXXX` low surrogate.
            if self.bump() != Some('\\') || self.bump() != Some('u') {
                return Err("expected low surrogate".to_string());
            }
            let low = self.parse_hex4()?;
            let c = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
            char::from_u32(c).ok_or_else(|| "invalid surrogate pair".to_string())
        } else {
            char::from_u32(high).ok_or_else(|| "invalid unicode escape".to_string())
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let c = self.bump().ok_or("unterminated unicode escape")?;
            let digit = c.to_digit(16).ok_or("invalid hex digit")?;
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
        {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse::<f64>()
            .map(Json::Number)
            .map_err(|_| format!("invalid number '{text}'"))
    }

    fn parse_bool(&mut self) -> Result<Json, String> {
        if self.consume_keyword("true") {
            Ok(Json::Bool(true))
        } else if self.consume_keyword("false") {
            Ok(Json::Bool(false))
        } else {
            Err("invalid literal".to_string())
        }
    }

    fn parse_null(&mut self) -> Result<Json, String> {
        if self.consume_keyword("null") {
            Ok(Json::Null)
        } else {
            Err("invalid literal".to_string())
        }
    }

    fn consume_keyword(&mut self, keyword: &str) -> bool {
        let end = self.pos + keyword.len();
        if end <= self.chars.len()
            && self.chars[self.pos..end]
                .iter()
                .copied()
                .eq(keyword.chars())
        {
            self.pos = end;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_scalars() {
        for input in ["null", "true", "false", "42", "-7", "3.5", "\"hi\""] {
            let parsed = Json::parse(input).unwrap();
            assert_eq!(parsed.to_string(), input);
        }
    }

    #[test]
    fn parses_nested_structures() {
        let j = Json::parse(r#"{"a":[1,2,{"b":true}],"c":"x"}"#).unwrap();
        assert_eq!(j.pointer(&["a"]).unwrap().as_array().unwrap().len(), 3);
        assert_eq!(
            j.get("a").unwrap().as_array().unwrap()[2]
                .get("b")
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(j.get("c").unwrap().as_str(), Some("x"));
    }

    #[test]
    fn handles_string_escapes() {
        let j = Json::parse(r#""line1\nline2\t\"quoted\"""#).unwrap();
        assert_eq!(j.as_str(), Some("line1\nline2\t\"quoted\""));
    }

    #[test]
    fn handles_unicode_escapes() {
        assert_eq!(Json::parse(r#""é""#).unwrap().as_str(), Some("é"));
        // A surrogate pair for U+1F600 😀.
        assert_eq!(Json::parse(r#""😀""#).unwrap().as_str(), Some("😀"));
    }

    #[test]
    fn serializes_objects_with_escapes() {
        let obj = Json::object([("name", Json::from("a\"b")), ("n", Json::from(5i64))]);
        // BTreeMap orders keys, so this is deterministic.
        assert_eq!(obj.to_string(), r#"{"n":5,"name":"a\"b"}"#);
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(Json::parse("{} extra").is_err());
        assert!(Json::parse("[1,2").is_err());
    }

    #[test]
    fn whitespace_is_ignored() {
        let j = Json::parse("  {\n  \"k\" : [ 1 , 2 ]\n}  ").unwrap();
        assert_eq!(j.get("k").unwrap().as_array().unwrap().len(), 2);
    }
}
