// SPDX-License-Identifier: Apache-2.0

//! String utilities for text plans.

/// Escapes a string for use in a textplan.
pub fn escape_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('"');

    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_ascii_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }

    result.push('"');
    result
}

/// Unescapes a string from a textplan.
pub fn unescape_string(s: &str) -> String {
    if s.len() < 2 || !s.starts_with('"') || !s.ends_with('"') {
        return s.to_string();
    }

    let s = &s[1..s.len() - 1];
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => result.push('"'),
                Some('\\') => result.push('\\'),
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('u') => {
                    let mut code = String::new();
                    for _ in 0..4 {
                        if let Some(d) = chars.next() {
                            code.push(d);
                        }
                    }
                    if let Ok(code) = u32::from_str_radix(&code, 16) {
                        if let Some(c) = std::char::from_u32(code) {
                            result.push(c);
                        }
                    }
                }
                Some(c) => result.push(c),
                None => break,
            }
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_string() {
        assert_eq!(escape_string("hello"), r#""hello""#);
        assert_eq!(escape_string("hello\nworld"), r#""hello\nworld""#);
        assert_eq!(escape_string("hello\"world"), r#""hello\"world""#);
        assert_eq!(escape_string("hello\\world"), r#""hello\\world""#);
    }

    #[test]
    fn test_unescape_string() {
        assert_eq!(unescape_string(r#""hello""#), "hello");
        assert_eq!(unescape_string(r#""hello\nworld""#), "hello\nworld");
        assert_eq!(unescape_string(r#""hello\"world""#), "hello\"world");
        assert_eq!(unescape_string(r#""hello\\world""#), "hello\\world");
    }
}
