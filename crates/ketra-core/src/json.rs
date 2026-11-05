//! Minimal JSON support: escaping, a tiny writer for JSONL records and a
//! small recursive-descent parser for settings / journals / manifests.
//!
//! The engine deliberately has zero external dependencies, so the subset of
//! JSON needed by KetraCopy is implemented here (~250 lines).

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Escape and quote a string as a JSON string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    /// Get a member of an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(key),
            _ => None,
        }
    }

    /// Interpret the value as a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Interpret the value as a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// Interpret the value as a non-negative integer.
    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64().filter(|n| *n >= 0.0 && n.fract() == 0.0).map(|n| n as u64)
    }

    /// Interpret the value as a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

/// Parse a JSON document. Returns an error message with byte offset on failure.
pub fn parse(s: &str) -> Result<Json, String> {
    let mut p = Parser { b: s.as_bytes(), i: 0 };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.b.len() {
        return Err(format!("trailing data at byte {}", p.i));
    }
    Ok(v)
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            _ => Err(format!("unexpected byte {} at {}", self.peek().unwrap_or(0), self.i)),
        }
    }

    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(format!("invalid literal at byte {}", self.i))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        while self.peek().map_or(false, |c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) {
            self.i += 1;
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).map_err(|e| e.to_string())?;
        text.parse::<f64>().map(Json::Num).map_err(|e| format!("bad number '{text}': {e}"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".into()),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    match self.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'n') => out.push('\n'),
                        Some(b't') => out.push('\t'),
                        Some(b'r') => out.push('\r'),
                        Some(b'b') => out.push('\u{08}'),
                        Some(b'f') => out.push('\u{0C}'),
                        Some(b'u') => {
                            if self.i + 4 >= self.b.len() {
                                return Err("bad \\u escape".into());
                            }
                            let hex = std::str::from_utf8(&self.b[self.i + 1..self.i + 5])
                                .map_err(|e| e.to_string())?;
                            let cp = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            out.push(ch);
                            self.i += 4;
                        }
                        _ => return Err(format!("bad escape at byte {}", self.i)),
                    }
                    self.i += 1;
                }
                Some(_) => {
                    // Consume one UTF-8 encoded char.
                    let rest = std::str::from_utf8(&self.b[self.i..])
                        .map_err(|_| "invalid utf-8".to_string())?;
                    let ch = rest.chars().next().unwrap();
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.i)),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut map = BTreeMap::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Json::Obj(map));
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            self.expect(b':')?;
            self.ws();
            let val = self.value()?;
            map.insert(key, val);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Obj(map));
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.i)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_escaping() {
        assert_eq!(quote("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
        assert_eq!(quote("tab\there"), "\"tab\\there\"");
        assert_eq!(quote("\u{1}"), "\"\\u0001\"");
    }

    #[test]
    fn test_parse_roundtrip_object() {
        let src = r#"{"a":1,"b":[true,null,"x\ny"],"c":{"d":2.5}}"#;
        let v = parse(src).expect("parse");
        assert_eq!(v.get("a").and_then(|j| j.as_u64()), Some(1));
        assert_eq!(v.get("c").and_then(|j| j.get("d")).and_then(|j| j.as_f64()), Some(2.5));
        let b = v.get("b").unwrap();
        if let Json::Arr(items) = b {
            assert_eq!(items.len(), 3);
            assert_eq!(items[2].as_str(), Some("x\ny"));
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_parse_errors_report_position() {
        assert!(parse("{").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse("nul").is_err());
        assert!(parse("\"abc\" trailing").is_err());
    }

    #[test]
    fn test_unicode_escape() {
        let v = parse(r#""Aé""#).expect("parse");
        assert_eq!(v.as_str(), Some("Aé"));
    }

    #[test]
    fn test_as_u64_rejects_fraction() {
        let v = parse("3.5").unwrap();
        assert_eq!(v.as_u64(), None);
        let v = parse("42").unwrap();
        assert_eq!(v.as_u64(), Some(42));
    }
}
