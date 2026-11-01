//! Python's number and text conventions, which saved queries, exports and condition matching rely on.

use chrono::{NaiveDateTime, Timelike};

/// Python `repr(float)`: shortest round-trip digits, `50000000.0`, `1e+16`, `1.5e-05`.
pub fn py_float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let sci = format!("{f:e}");
    let (mantissa, exp) = sci.split_once('e').expect("exponent");
    let exp: i32 = exp.parse().expect("exponent number");
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.trim_start_matches('-').chars().filter(|c| *c != '.').collect();
    let sign = if negative { "-" } else { "" };
    if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                format!("{sign}{digits}{}.0", "0".repeat(int_len - digits.len()))
            } else {
                format!("{sign}{}.{}", &digits[..int_len], &digits[int_len..])
            }
        } else {
            format!("{sign}0.{}{digits}", "0".repeat((-exp - 1) as usize))
        }
    } else {
        let mant = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits };
        let esign = if exp < 0 { '-' } else { '+' };
        format!("{sign}{mant}e{esign}{:02}", exp.abs())
    }
}

/// Python `float(text)`: surrounding whitespace, `inf` / `nan` / `infinity` in any case, and
/// underscores between digits are accepted.
pub fn py_float(text: &str) -> Option<f64> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    let body = s.trim_start_matches(['+', '-']);
    if body.len() + 1 < s.len() {
        return None; // more than one sign
    }
    let negative = s.starts_with('-');
    match body.to_ascii_lowercase().as_str() {
        "inf" | "infinity" => return Some(if negative { f64::NEG_INFINITY } else { f64::INFINITY }),
        "nan" => return Some(f64::NAN),
        _ => {}
    }
    let cleaned;
    let s = if s.contains('_') {
        let chars: Vec<char> = s.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if *c == '_' {
                let before = i.checked_sub(1).and_then(|j| chars.get(j));
                let after = chars.get(i + 1);
                if !before.is_some_and(|c| c.is_ascii_digit()) || !after.is_some_and(|c| c.is_ascii_digit()) {
                    return None;
                }
            }
        }
        cleaned = s.replace('_', "");
        cleaned.as_str()
    } else {
        s
    };
    if !s.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E')) {
        return None;
    }
    s.parse::<f64>().ok()
}

/// pandas `to_numeric` of one string: a plain number (no thousands separators or percent signs).
pub fn pd_numeric(text: &str) -> Option<f64> {
    let s = text.trim();
    if s.contains('_') {
        return None;
    }
    py_float(s)
}

/// Python `str(Timestamp)`: `2024-01-05 13:00:00`, with microseconds when there are any.
pub fn py_str_datetime(d: &NaiveDateTime) -> String {
    if d.nanosecond() == 0 {
        d.format("%Y-%m-%d %H:%M:%S").to_string()
    } else {
        d.format("%Y-%m-%d %H:%M:%S%.6f").to_string()
    }
}

/// Python `repr(str)`: single quotes unless the text has a single quote and no double quote.
pub fn py_repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Python `str.casefold()` (full Unicode case folding: `Straße` -> `strasse`).
pub fn casefold(s: &str) -> String {
    caseless::default_case_fold_str(s)
}

/// Python `html.escape(s, quote=True)`.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            c => out.push(c),
        }
    }
    out
}

/// Escape every non-ASCII character as `\uXXXX`, as Python's `json.dumps` does by default.
/// Non-ASCII can only occur inside JSON strings, so this is safe on a whole document.
pub fn json_ensure_ascii(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for unit in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_repr() {
        for (f, s) in [
            (5e7, "50000000.0"),
            (1e16, "1e+16"),
            (1.5e-5, "1.5e-05"),
            (0.0001, "0.0001"),
            (123456789012345678.0, "1.2345678901234568e+17"),
            (-0.25, "-0.25"),
            (2.0, "2.0"),
            (0.1, "0.1"),
            (1e15, "1000000000000000.0"),
        ] {
            assert_eq!(py_float_repr(f), s);
        }
    }

    #[test]
    fn float_parse() {
        assert_eq!(py_float(" 5 "), Some(5.0));
        assert_eq!(py_float("1_000"), Some(1000.0));
        assert_eq!(py_float("1__0"), None);
        assert_eq!(py_float("Infinity"), Some(f64::INFINITY));
        assert!(py_float("nan").unwrap().is_nan());
        assert_eq!(py_float("1."), Some(1.0));
        assert_eq!(py_float("-.5"), Some(-0.5));
        assert_eq!(py_float("0x10"), None);
        assert_eq!(py_float("--1"), None);
    }

    #[test]
    fn repr_and_escape() {
        assert_eq!(py_repr_str("a"), "'a'");
        assert_eq!(py_repr_str("it's"), "\"it's\"");
        assert_eq!(html_escape("<a href='x'>&\""), "&lt;a href=&#x27;x&#x27;&gt;&amp;&quot;");
        assert_eq!(json_ensure_ascii("a › b 😀"), "a \\u203a b \\ud83d\\ude00");
    }
}
