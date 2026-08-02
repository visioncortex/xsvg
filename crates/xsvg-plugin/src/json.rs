//! A minimal JSON reader/writer.
//!
//! The plugin ABI is JSON strings across a `memory` boundary
//! (docs/Extensibility.md §4), and a plugin should stay small — pulling a full
//! serialization stack into every module would dwarf the geometry it exists to
//! compute. This handles the whole grammar, which is all the ABI needs.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }
}

pub fn parse(s: &str) -> Option<Json> {
    let b = s.as_bytes();
    let mut i = 0;
    let v = value(b, &mut i)?;
    skip_ws(b, &mut i);
    (i == b.len()).then_some(v)
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && matches!(b[*i], b' ' | b'\t' | b'\n' | b'\r') {
        *i += 1;
    }
}

fn value(b: &[u8], i: &mut usize) -> Option<Json> {
    skip_ws(b, i);
    match *b.get(*i)? {
        b'n' => lit(b, i, b"null", Json::Null),
        b't' => lit(b, i, b"true", Json::Bool(true)),
        b'f' => lit(b, i, b"false", Json::Bool(false)),
        b'"' => string(b, i).map(Json::Str),
        b'[' => {
            *i += 1;
            let mut out = Vec::new();
            skip_ws(b, i);
            if b.get(*i) == Some(&b']') {
                *i += 1;
                return Some(Json::Arr(out));
            }
            loop {
                out.push(value(b, i)?);
                skip_ws(b, i);
                match b.get(*i)? {
                    b',' => *i += 1,
                    b']' => {
                        *i += 1;
                        return Some(Json::Arr(out));
                    }
                    _ => return None,
                }
            }
        }
        b'{' => {
            *i += 1;
            let mut out = Vec::new();
            skip_ws(b, i);
            if b.get(*i) == Some(&b'}') {
                *i += 1;
                return Some(Json::Obj(out));
            }
            loop {
                skip_ws(b, i);
                let k = string(b, i)?;
                skip_ws(b, i);
                if b.get(*i) != Some(&b':') {
                    return None;
                }
                *i += 1;
                out.push((k, value(b, i)?));
                skip_ws(b, i);
                match b.get(*i)? {
                    b',' => *i += 1,
                    b'}' => {
                        *i += 1;
                        return Some(Json::Obj(out));
                    }
                    _ => return None,
                }
            }
        }
        _ => number(b, i),
    }
}

fn lit(b: &[u8], i: &mut usize, want: &[u8], v: Json) -> Option<Json> {
    b.get(*i..*i + want.len()).filter(|s| *s == want).map(|_| {
        *i += want.len();
        v
    })
}

fn string(b: &[u8], i: &mut usize) -> Option<String> {
    if b.get(*i) != Some(&b'"') {
        return None;
    }
    *i += 1;
    let mut out = String::new();
    loop {
        let c = *b.get(*i)?;
        *i += 1;
        match c {
            b'"' => return Some(out),
            b'\\' => {
                let e = *b.get(*i)?;
                *i += 1;
                match e {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hi = hex4(b, i)?;
                        // surrogate pair, else a lone code point
                        let ch = if (0xD800..0xDC00).contains(&hi) {
                            if b.get(*i) != Some(&b'\\') || b.get(*i + 1) != Some(&b'u') {
                                return None;
                            }
                            *i += 2;
                            let lo = hex4(b, i)?;
                            if !(0xDC00..0xE000).contains(&lo) {
                                return None;
                            }
                            char::from_u32(
                                0x10000 + ((hi as u32 - 0xD800) << 10) + (lo as u32 - 0xDC00),
                            )?
                        } else {
                            char::from_u32(hi as u32)?
                        };
                        out.push(ch);
                    }
                    _ => return None,
                }
            }
            // multi-byte UTF-8 passes through a byte at a time
            _ => {
                let start = *i - 1;
                let len = utf8_len(c)?;
                let s = core::str::from_utf8(b.get(start..start + len)?).ok()?;
                out.push_str(s);
                *i = start + len;
            }
        }
    }
}

fn utf8_len(c: u8) -> Option<usize> {
    Some(match c {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    })
}

fn hex4(b: &[u8], i: &mut usize) -> Option<u16> {
    let s = core::str::from_utf8(b.get(*i..*i + 4)?).ok()?;
    *i += 4;
    u16::from_str_radix(s, 16).ok()
}

fn number(b: &[u8], i: &mut usize) -> Option<Json> {
    let start = *i;
    if b.get(*i) == Some(&b'-') {
        *i += 1;
    }
    while matches!(b.get(*i), Some(c) if c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        *i += 1;
    }
    core::str::from_utf8(b.get(start..*i)?)
        .ok()?
        .parse()
        .ok()
        .map(Json::Num)
}

/// Append `s` as a quoted JSON string.
pub fn push_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u");
                for shift in [12, 8, 4, 0] {
                    out.push(char::from_digit((c as u32 >> shift) & 0xF, 16).unwrap_or('0'));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_the_shapes_the_abi_uses() {
        let src = r##"{"tag":"arrow","attrs":[["length","10"],["fill","#f00"]],
                      "input_d":null,"children":[{"svg":"<path/>","bbox":[1,2,3,4]}],
                      "tolerance":0.25}"##;
        let v = parse(src).expect("parses");
        assert_eq!(v.get("tag").and_then(Json::as_str), Some("arrow"));
        assert_eq!(v.get("tolerance").and_then(Json::as_num), Some(0.25));
        assert_eq!(v.get("input_d"), Some(&Json::Null));
        let attrs = v.get("attrs").and_then(Json::as_arr).unwrap();
        assert_eq!(attrs[1].as_arr().unwrap()[1].as_str(), Some("#f00"));
        let kid = &v.get("children").and_then(Json::as_arr).unwrap()[0];
        assert_eq!(kid.get("svg").and_then(Json::as_str), Some("<path/>"));
        assert_eq!(
            kid.get("bbox").and_then(Json::as_arr).unwrap()[3].as_num(),
            Some(4.0)
        );
    }

    #[test]
    fn decodes_escapes_including_surrogate_pairs() {
        let v = parse(r#""a\"b\\c\ndé😀""#).unwrap();
        assert_eq!(v.as_str(), Some("a\"b\\c\nd\u{e9}\u{1f600}"));
    }

    #[test]
    fn rejects_malformed_input_rather_than_guessing() {
        for bad in [
            "{",
            "[1,]",
            "{\"a\"}",
            "\"unterminated",
            "tru",
            "{\"a\":1}x",
            "[1 2]",
            r#""\ud83d""#, // lone high surrogate
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn writes_strings_a_json_reader_can_read_back() {
        let mut s = String::new();
        push_str(&mut s, "quote\" back\\slash\nnewline\u{1}ctrl");
        assert_eq!(
            parse(&s).unwrap().as_str(),
            Some("quote\" back\\slash\nnewline\u{1}ctrl")
        );
    }

    #[test]
    fn survives_multibyte_text_in_values() {
        let mut s = String::new();
        push_str(&mut s, "héllo → 世界 🌍");
        assert_eq!(parse(&s).unwrap().as_str(), Some("héllo → 世界 🌍"));
    }
}
