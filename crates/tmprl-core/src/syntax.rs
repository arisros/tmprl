//! A line of pretty-printed JSON cut into the parts the payload pane colours.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Key,
    String,
    Number,
    Literal,
    Punct,
    Plain,
}

/// Each part with its byte range; text that is not JSON comes back as one `Plain` part.
pub fn json_line(line: &str) -> Vec<(Token, std::ops::Range<usize>)> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let token = match bytes[i] {
            b' ' | b'\t' => {
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                    i += 1;
                }
                Token::Plain
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                let mut after = i;
                while after < bytes.len() && bytes[after] == b' ' {
                    after += 1;
                }
                if bytes.get(after) == Some(&b':') {
                    Token::Key
                } else {
                    Token::String
                }
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                i += 1;
                Token::Punct
            }
            b'-' | b'0'..=b'9' => {
                while i < bytes.len()
                    && matches!(bytes[i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    i += 1;
                }
                Token::Number
            }
            _ => {
                let word = ["true", "false", "null"]
                    .into_iter()
                    .find(|w| line[i..].starts_with(w));
                match word {
                    Some(w) => {
                        i += w.len();
                        Token::Literal
                    }
                    None => return vec![(Token::Plain, 0..line.len())],
                }
            }
        };
        out.push((token, start..i));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(line: &str) -> Vec<(Token, &str)> {
        json_line(line)
            .into_iter()
            .filter(|(t, _)| *t != Token::Plain)
            .map(|(t, r)| (t, &line[r]))
            .collect()
    }

    #[test]
    fn a_key_a_string_a_number_and_a_literal_each_get_their_part() {
        assert_eq!(
            parts(r#"  "asset_code": "DAIHATSU \"AYLA\"","#),
            vec![
                (Token::Key, r#""asset_code""#),
                (Token::Punct, ":"),
                (Token::String, r#""DAIHATSU \"AYLA\"""#),
                (Token::Punct, ","),
            ]
        );
        assert_eq!(
            parts(r#""year": -2018.5e2, "ok": true, "x": null"#)
                .into_iter()
                .map(|(t, _)| t)
                .collect::<Vec<_>>(),
            vec![
                Token::Key,
                Token::Punct,
                Token::Number,
                Token::Punct,
                Token::Key,
                Token::Punct,
                Token::Literal,
                Token::Punct,
                Token::Key,
                Token::Punct,
                Token::Literal,
            ]
        );
        assert_eq!(
            parts("  },"),
            vec![(Token::Punct, "}"), (Token::Punct, ",")]
        );
    }

    #[test]
    fn text_that_is_not_json_is_left_whole_and_ranges_cover_the_line() {
        assert_eq!(json_line("hello world"), vec![(Token::Plain, 0..11)]);
        let line = r#"    "a": [1, "b"]"#;
        let covered: usize = json_line(line).iter().map(|(_, r)| r.len()).sum();
        assert_eq!(covered, line.len());
        assert_eq!(json_line(r#""unterminated"#).last().unwrap().1.end, 13);
    }
}
