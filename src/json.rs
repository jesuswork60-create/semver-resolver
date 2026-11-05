//! A small strict JSON parser and serializer (RFC 8259).
//!
//! Objects keep their key order and duplicate keys are rejected, since a
//! duplicated package or version in a data file is always a mistake.

use std::fmt;

/// Nesting deeper than this is refused, so hostile input cannot overflow the stack.
const MAX_DEPTH: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for JsonError {}

impl Json {
    pub fn parse(text: &str) -> Result<Json, JsonError> {
        let mut parser = Parser { text, pos: 0, depth: 0 };
        parser.skip_whitespace();
        let value = parser.parse_value()?;
        parser.skip_whitespace();
        if parser.pos != text.len() {
            return Err(parser.error("trailing characters after the JSON value"));
        }
        Ok(value)
    }

    pub fn string(text: impl Into<String>) -> Json {
        Json::String(text.into())
    }

    /// Looks up a key on an object; `None` for other kinds of value.
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.as_object()?.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(entries) => Some(entries),
            _ => None,
        }
    }

    /// Serializes with two-space indentation and a trailing newline.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, 0);
        out.push('\n');
        out
    }

    fn write_pretty(&self, out: &mut String, indent: usize) {
        let pad = |out: &mut String, n: usize| out.extend(std::iter::repeat_n("  ", n));
        match self {
            Json::Array(items) if !items.is_empty() => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    pad(out, indent + 1);
                    item.write_pretty(out, indent + 1);
                    out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
                }
                pad(out, indent);
                out.push(']');
            }
            Json::Object(entries) if !entries.is_empty() => {
                out.push_str("{\n");
                for (i, (key, value)) in entries.iter().enumerate() {
                    pad(out, indent + 1);
                    write_escaped(out, key);
                    out.push_str(": ");
                    value.write_pretty(out, indent + 1);
                    out.push_str(if i + 1 < entries.len() { ",\n" } else { "\n" });
                }
                pad(out, indent);
                out.push('}');
            }
            Json::Array(_) => out.push_str("[]"),
            Json::Object(_) => out.push_str("{}"),
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            // JSON has no spelling for infinities or NaN.
            Json::Number(n) if !n.is_finite() => out.push_str("null"),
            Json::Number(n) => out.push_str(&n.to_string()),
            Json::String(s) => write_escaped(out, s),
        }
    }
}

fn write_escaped(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, message: impl Into<String>) -> JsonError {
        let before = &self.text[..self.pos];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let column = before[line_start..].chars().count() + 1;
        JsonError { line, column, message: message.into() }
    }

    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self) -> Result<Json, JsonError> {
        match self.peek() {
            None => Err(self.error("unexpected end of input")),
            Some(b'{') => self.nested(Self::parse_object),
            Some(b'[') => self.nested(Self::parse_array),
            Some(b'"') => self.parse_string().map(Json::String),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(_) => self.parse_literal(),
        }
    }

    fn nested(
        &mut self,
        parse: fn(&mut Self) -> Result<Json, JsonError>,
    ) -> Result<Json, JsonError> {
        if self.depth == MAX_DEPTH {
            return Err(self.error("nesting is too deep"));
        }
        self.depth += 1;
        let value = parse(self);
        self.depth -= 1;
        value
    }

    fn parse_literal(&mut self) -> Result<Json, JsonError> {
        for (word, value) in
            [("true", Json::Bool(true)), ("false", Json::Bool(false)), ("null", Json::Null)]
        {
            if self.text[self.pos..].starts_with(word) {
                self.pos += word.len();
                return Ok(value);
            }
        }
        Err(self.error("expected a JSON value"))
    }

    fn parse_array(&mut self) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.parse_value()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]` in array")),
            }
        }
    }

    fn parse_object(&mut self) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut entries: Vec<(String, Json)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(entries));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a string key in object"));
            }
            let key_start = self.pos;
            let key = self.parse_string()?;
            if entries.iter().any(|(k, _)| *k == key) {
                self.pos = key_start;
                return Err(self.error(format!("duplicate key `{key}`")));
            }
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(self.error("expected `:` after object key"));
            }
            self.pos += 1;
            self.skip_whitespace();
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(entries));
                }
                _ => return Err(self.error("expected `,` or `}` in object")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.text[self.pos..].chars().next() else {
                return Err(self.error("unterminated string"));
            };
            match c {
                '"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                '\\' => {
                    self.pos += 1;
                    out.push(self.parse_escape()?);
                }
                c if (c as u32) < 0x20 => {
                    return Err(self.error("unescaped control character in string"));
                }
                c => {
                    out.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
    }

    fn parse_escape(&mut self) -> Result<char, JsonError> {
        let Some(byte) = self.peek() else {
            return Err(self.error("unterminated string"));
        };
        let simple = match byte {
            b'"' => Some('"'),
            b'\\' => Some('\\'),
            b'/' => Some('/'),
            b'b' => Some('\u{8}'),
            b'f' => Some('\u{c}'),
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            _ => None,
        };
        if let Some(c) = simple {
            self.pos += 1;
            return Ok(c);
        }
        if byte != b'u' {
            return Err(self.error("invalid escape sequence"));
        }
        self.pos += 1;
        let first = self.parse_hex4()?;
        let code = match first {
            0xD800..=0xDBFF => {
                if !self.text[self.pos..].starts_with("\\u") {
                    return Err(self.error("high surrogate without a low surrogate"));
                }
                self.pos += 2;
                let second = self.parse_hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(self.error("high surrogate without a low surrogate"));
                }
                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            0xDC00..=0xDFFF => return Err(self.error("low surrogate without a high surrogate")),
            other => other,
        };
        char::from_u32(code).ok_or_else(|| self.error("invalid unicode escape"))
    }

    fn parse_hex4(&mut self) -> Result<u32, JsonError> {
        let digits = self.text.get(self.pos..self.pos + 4).filter(|d| {
            d.bytes().all(|b| b.is_ascii_hexdigit())
        });
        let Some(digits) = digits else {
            return Err(self.error("expected four hex digits"));
        };
        let code = u32::from_str_radix(digits, 16).map_err(|_| self.error("bad hex digits"))?;
        self.pos += 4;
        Ok(code)
    }

    fn take_digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn parse_number(&mut self) -> Result<Json, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let int_start = self.pos;
        let int_len = self.take_digits();
        if int_len == 0 {
            return Err(self.error("expected a digit"));
        }
        if int_len > 1 && self.text.as_bytes()[int_start] == b'0' {
            self.pos = int_start;
            return Err(self.error("number has a leading zero"));
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.take_digits() == 0 {
                return Err(self.error("expected a digit after the decimal point"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.take_digits() == 0 {
                return Err(self.error("expected a digit in the exponent"));
            }
        }
        let value: f64 =
            self.text[start..self.pos].parse().map_err(|_| self.error("invalid number"))?;
        if !value.is_finite() {
            self.pos = start;
            return Err(self.error("number is out of range"));
        }
        Ok(Json::Number(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_position_counts_characters_not_bytes() {
        let err = Json::parse("{\n  \"é\": tru\n}").unwrap_err();
        assert_eq!((err.line, err.column), (2, 8));
    }

    #[test]
    fn hex_reader_does_not_split_multibyte_characters() {
        let err = Json::parse("\"\\u12é\"").unwrap_err();
        assert!(err.message.contains("hex"), "{}", err.message);
    }
}
