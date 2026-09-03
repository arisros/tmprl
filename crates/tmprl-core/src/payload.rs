//! Temporal payloads: opaque bytes plus metadata saying how to read them.
//!
//! Every input, result and failure detail on the wire is one of these. The encoding is a
//! string in the metadata, and it decides everything: whether the bytes are text we can show,
//! bytes we should not try to, or ciphertext that needs a codec server we have not called yet.
//!
//! Deciding that is pure, so it lives here and is tested without a server. What is *not* here
//! is the codec round trip, which is network IO and belongs in `tmprl-client`.

/// One payload, as it arrived.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Payload {
    /// `metadata["encoding"]`, e.g. `json/plain`. Absent on a malformed payload, which is
    /// treated as opaque rather than guessed at.
    pub encoding: String,
    /// `metadata["type"]`, when the producer set one. Search attributes set `Keyword`; most
    /// SDK payloads set nothing.
    pub type_hint: Option<String>,
    pub data: Vec<u8>,
}

/// What a payload can be shown as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rendered {
    /// Nothing was sent. Distinct from an empty string, which is a value.
    Null,
    /// Text, ready to display. JSON is pretty-printed.
    Text(String),
    /// Bytes we will not try to render. Guessing at an encoding produces mojibake, and a
    /// terminal is an unforgiving place to paste control characters into.
    Opaque { bytes: usize, encoding: String },
    /// Encrypted by a codec server. Renders as a badge until a decode resolves — the value
    /// is not lost, it is just not readable yet.
    Encrypted { bytes: usize },
}

impl Payload {
    pub fn new(encoding: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            encoding: encoding.into(),
            type_hint: None,
            data: data.to_vec(),
        }
    }

    /// Whether reading this needs a round trip to the user's codec server.
    pub fn needs_codec(&self) -> bool {
        self.encoding == "binary/encrypted"
    }

    /// How to show it.
    ///
    /// The encodings are Temporal's own. Anything unrecognised is opaque rather than
    /// optimistically decoded as UTF-8: a payload from a custom converter can be arbitrary
    /// bytes, and printing those into a terminal is how you end up with a corrupted screen.
    pub fn render(&self) -> Rendered {
        match self.encoding.as_str() {
            "binary/null" => Rendered::Null,
            // `json/protobuf` is proto3-JSON — still JSON text on the wire.
            "json/plain" | "json/protobuf" => match std::str::from_utf8(&self.data) {
                Ok(text) => Rendered::Text(pretty_json(text)),
                // Declared JSON but not valid UTF-8: the declaration is wrong, so do not
                // trust it enough to print the bytes.
                Err(_) => self.opaque(),
            },
            "text/plain" => match std::str::from_utf8(&self.data) {
                Ok(text) => Rendered::Text(text.to_string()),
                Err(_) => self.opaque(),
            },
            "binary/encrypted" => Rendered::Encrypted {
                bytes: self.data.len(),
            },
            _ => self.opaque(),
        }
    }

    fn opaque(&self) -> Rendered {
        Rendered::Opaque {
            bytes: self.data.len(),
            encoding: if self.encoding.is_empty() {
                "unknown".to_string()
            } else {
                self.encoding.clone()
            },
        }
    }

    /// A single line, for a row that has no space for the whole value.
    pub fn summary(&self, width: usize) -> String {
        match self.render() {
            Rendered::Null => "null".into(),
            Rendered::Encrypted { bytes } => format!("🔒 encrypted, {bytes} bytes"),
            Rendered::Opaque { bytes, encoding } => format!("{encoding}, {bytes} bytes"),
            Rendered::Text(t) => {
                // Collapse to one line first: a pretty-printed value is mostly newlines, and
                // truncating those leaves a row that is blank but not empty.
                let flat = t.split_whitespace().collect::<Vec<_>>().join(" ");
                if flat.chars().count() <= width {
                    flat
                } else {
                    let keep: String = flat.chars().take(width.saturating_sub(1)).collect();
                    format!("{keep}…")
                }
            }
        }
    }

    /// The bytes to hand to an external command such as `jq`.
    ///
    /// `None` when there is nothing meaningful to pipe — piping ciphertext or an opaque blob
    /// into `jq` produces a parse error that says nothing useful about why.
    pub fn pipeable(&self) -> Option<&[u8]> {
        match self.encoding.as_str() {
            "json/plain" | "json/protobuf" | "text/plain" => Some(&self.data),
            _ => None,
        }
    }
}

/// Pretty-print JSON, or hand back the input unchanged when it is not JSON.
///
/// Payloads claim `json/plain` and are usually right, but a workflow can put anything in one.
/// A value that does not parse is shown as it arrived rather than rejected — seeing the raw
/// bytes is more useful than being told they were unparseable.
pub fn pretty_json(text: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| text.to_string()),
        Err(_) => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(body: &str) -> Payload {
        Payload::new("json/plain", body.as_bytes().to_vec())
    }

    #[test]
    fn a_json_payload_is_pretty_printed() {
        let p = json(r#"{"amount":100,"currency":"GBP"}"#);
        let Rendered::Text(t) = p.render() else {
            panic!("expected text, got {:?}", p.render())
        };
        assert!(t.contains("\n"), "should be pretty-printed:\n{t}");
        assert!(t.contains("\"amount\": 100"), "got:\n{t}");
    }

    #[test]
    fn a_scalar_json_payload_survives_intact() {
        // The common case from a real worker: an activity argument of `100`, or `"Sleep"`.
        assert_eq!(json("100").render(), Rendered::Text("100".into()));
        assert_eq!(
            json("\"Sleep\"").render(),
            Rendered::Text("\"Sleep\"".into())
        );
    }

    #[test]
    fn json_that_does_not_parse_is_shown_raw_rather_than_rejected() {
        // A workflow can put anything in a payload it labelled json/plain. Showing the bytes
        // beats telling the reader they were unparseable.
        let p = json("{not json");
        assert_eq!(p.render(), Rendered::Text("{not json".into()));
    }

    #[test]
    fn a_null_payload_is_not_an_empty_string() {
        let p = Payload::new("binary/null", Vec::new());
        assert_eq!(p.render(), Rendered::Null);
        assert_eq!(p.summary(40), "null");
        // An empty *string* is a value, and must not be confused with nothing being sent.
        assert_eq!(json("\"\"").render(), Rendered::Text("\"\"".into()));
    }

    #[test]
    fn encrypted_payloads_announce_themselves_rather_than_showing_ciphertext() {
        let p = Payload::new("binary/encrypted", vec![0u8; 64]);
        assert!(p.needs_codec());
        assert_eq!(p.render(), Rendered::Encrypted { bytes: 64 });
        assert!(p.summary(40).contains("encrypted"));
        assert_eq!(p.pipeable(), None, "ciphertext is not worth piping to jq");
    }

    #[test]
    fn unknown_and_binary_encodings_stay_opaque() {
        // Optimistically decoding arbitrary bytes as UTF-8 is how a terminal ends up full of
        // control characters.
        for enc in ["binary/plain", "binary/deflate", "application/x-thrift", ""] {
            let p = Payload::new(enc, vec![0xff, 0xfe, 0x00, 0x01]);
            match p.render() {
                Rendered::Opaque { bytes, .. } => assert_eq!(bytes, 4),
                other => panic!("{enc} should be opaque, got {other:?}"),
            }
            assert_eq!(p.pipeable(), None);
        }
        assert!(
            Payload::new("", vec![1]).summary(40).contains("unknown"),
            "a missing encoding should say so"
        );
    }

    #[test]
    fn json_that_is_not_valid_utf8_is_not_trusted() {
        // The payload says json/plain but the bytes are not text. The declaration is wrong,
        // so it is treated as opaque rather than printed.
        let p = Payload::new("json/plain", vec![0xff, 0xff]);
        assert!(matches!(p.render(), Rendered::Opaque { .. }));
    }

    #[test]
    fn a_summary_is_one_line_and_fits() {
        let p = json(r#"{"a":1,"b":2,"c":"a rather long string value here"}"#);
        let s = p.summary(30);
        assert!(!s.contains('\n'), "a summary must be one line: {s:?}");
        assert!(
            s.chars().count() <= 30,
            "{} chars: {s:?}",
            s.chars().count()
        );
        assert!(s.ends_with('…'));

        // Short values are shown whole, without an ellipsis.
        assert_eq!(json("42").summary(30), "42");
    }

    #[test]
    fn only_textual_payloads_are_pipeable() {
        assert_eq!(json("{}").pipeable(), Some(&b"{}"[..]));
        assert_eq!(
            Payload::new("text/plain", b"hello".to_vec()).pipeable(),
            Some(&b"hello"[..])
        );
        assert_eq!(Payload::new("binary/null", vec![]).pipeable(), None);
    }
}
