//! Temporal payloads: opaque bytes plus metadata saying how to read them.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Payload {
    pub encoding: String,
    pub type_hint: Option<String>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rendered {
    Null,
    Text(String),
    Opaque { bytes: usize, encoding: String },
    Encrypted { bytes: usize, encoding: String },
}

impl Payload {
    pub fn new(encoding: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            encoding: encoding.into(),
            type_hint: None,
            data: data.to_vec(),
        }
    }

    // Temporal's own raw-bytes encodings; no codec decodes these.
    const NOT_CODEC: [&'static str; 6] = [
        "json/plain",
        "json/protobuf",
        "text/plain",
        "binary/null",
        "binary/plain",
        "binary/protobuf",
    ];

    pub fn needs_codec(&self) -> bool {
        !self.encoding.is_empty() && !Self::NOT_CODEC.contains(&self.encoding.as_str())
    }

    pub fn render(&self) -> Rendered {
        match self.encoding.as_str() {
            "binary/null" => Rendered::Null,
            "json/plain" | "json/protobuf" => match std::str::from_utf8(&self.data) {
                Ok(text) => Rendered::Text(pretty_json(text)),
                Err(_) => self.opaque(),
            },
            "text/plain" => match std::str::from_utf8(&self.data) {
                Ok(text) => Rendered::Text(text.to_string()),
                Err(_) => self.opaque(),
            },
            _ if self.needs_codec() => Rendered::Encrypted {
                bytes: self.data.len(),
                encoding: self.encoding.clone(),
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

    pub fn summary(&self, width: usize) -> String {
        match self.render() {
            Rendered::Null => "null".into(),
            Rendered::Encrypted { bytes, encoding } => {
                format!("🔒 {encoding}, {bytes} bytes")
            }
            Rendered::Opaque { bytes, encoding } => format!("{encoding}, {bytes} bytes"),
            Rendered::Text(t) => {
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

    pub fn pipeable(&self) -> Option<&[u8]> {
        match self.encoding.as_str() {
            "json/plain" | "json/protobuf" | "text/plain" => Some(&self.data),
            _ => None,
        }
    }
}

pub fn payloads_as_json(payloads: &[(String, Payload)]) -> (Option<String>, Vec<String>) {
    let mut obj = serde_json::Map::new();
    let mut skipped = Vec::new();

    for (label, p) in payloads {
        match p.pipeable() {
            None => skipped.push(label.clone()),
            Some(bytes) => {
                let Ok(text) = std::str::from_utf8(bytes) else {
                    skipped.push(label.clone());
                    continue;
                };
                let value = match p.encoding.as_str() {
                    "json/plain" | "json/protobuf" => serde_json::from_str(text)
                        .unwrap_or_else(|_| serde_json::Value::String(text.to_string())),
                    _ => serde_json::Value::String(text.to_string()),
                };
                obj.insert(label.clone(), value);
            }
        }
    }

    let json = if obj.is_empty() {
        None
    } else {
        serde_json::to_string_pretty(&serde_json::Value::Object(obj)).ok()
    };
    (json, skipped)
}

pub fn unwrap_single(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let obj = value.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    match obj.values().next()? {
        serde_json::Value::String(s) => Some(s.clone()),
        other => serde_json::to_string_pretty(other).ok(),
    }
}

pub fn flatten(label: &str, p: &Payload) -> Vec<String> {
    let Some(text) = p.pipeable().and_then(|b| std::str::from_utf8(b).ok()) else {
        return Vec::new();
    };
    let root = format!(".{label}");
    let mut out = Vec::new();
    match p.encoding.as_str() {
        "json/plain" | "json/protobuf" => match serde_json::from_str(text) {
            Ok(v) => flatten_value(&root, &v, &mut out),
            Err(_) => out.push(format!("{root}={text}")),
        },
        _ => out.push(format!("{root}={text}")),
    }
    out
}

#[derive(Debug, Default)]
pub struct FlatCache {
    entries: std::collections::HashMap<u64, (u64, Vec<String>)>,
    pass: u64,
}

impl FlatCache {
    pub fn lines(&mut self, label: &str, p: &Payload) -> &[String] {
        let pass = self.pass;
        let entry = self
            .entries
            .entry(Self::key(label, p))
            .or_insert_with(|| (pass, flatten(label, p)));
        entry.0 = pass;
        &entry.1
    }

    pub fn peek(&self, label: &str, p: &Payload) -> Option<&[String]> {
        self.entries
            .get(&Self::key(label, p))
            .map(|(_, l)| l.as_slice())
    }

    fn key(label: &str, p: &Payload) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        label.hash(&mut h);
        p.encoding.hash(&mut h);
        p.data.hash(&mut h);
        h.finish()
    }

    pub fn sweep(&mut self) {
        let pass = self.pass;
        self.entries.retain(|_, (used, _)| *used == pass);
        self.pass += 1;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn flatten_value(path: &str, v: &serde_json::Value, out: &mut Vec<String>) {
    use serde_json::Value;
    match v {
        Value::Object(m) if !m.is_empty() => {
            for (k, v) in m {
                flatten_value(&format!("{path}.{k}"), v, out);
            }
        }
        Value::Array(a) if !a.is_empty() => {
            for (i, v) in a.iter().enumerate() {
                flatten_value(&format!("{path}[{i}]"), v, out);
            }
        }
        Value::String(s) => out.push(format!("{path}={s}")),
        other => out.push(format!("{path}={other}")),
    }
}

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
        assert_eq!(json("100").render(), Rendered::Text("100".into()));
        assert_eq!(
            json("\"Sleep\"").render(),
            Rendered::Text("\"Sleep\"".into())
        );
    }

    #[test]
    fn json_that_does_not_parse_is_shown_raw_rather_than_rejected() {
        let p = json("{not json");
        assert_eq!(p.render(), Rendered::Text("{not json".into()));
    }

    #[test]
    fn a_null_payload_is_not_an_empty_string() {
        let p = Payload::new("binary/null", Vec::new());
        assert_eq!(p.render(), Rendered::Null);
        assert_eq!(p.summary(40), "null");
        assert_eq!(json("\"\"").render(), Rendered::Text("\"\"".into()));
    }

    #[test]
    fn encrypted_payloads_announce_themselves_rather_than_showing_ciphertext() {
        let p = Payload::new("binary/encrypted", vec![0u8; 64]);
        assert!(p.needs_codec());
        assert_eq!(
            p.render(),
            Rendered::Encrypted {
                bytes: 64,
                encoding: "binary/encrypted".into()
            }
        );
        assert!(p.summary(40).contains("encrypted"));
        assert_eq!(p.pipeable(), None, "ciphertext is not worth piping to jq");
    }

    #[test]
    fn unknown_and_binary_encodings_stay_opaque() {
        for enc in ["binary/plain", "binary/protobuf", ""] {
            let p = Payload::new(enc, vec![0xff, 0xfe, 0x00, 0x01]);
            match p.render() {
                Rendered::Opaque { bytes, .. } => assert_eq!(bytes, 4),
                other => panic!("{enc} should be opaque, got {other:?}"),
            }
            assert_eq!(p.pipeable(), None);
            assert!(!p.needs_codec(), "{enc} must not be sent to a codec");
        }
        for enc in ["binary/deflate", "application/x-thrift"] {
            let p = Payload::new(enc, vec![0xff, 0xfe]);
            assert!(p.needs_codec(), "{enc} should be offered to a codec");
            assert_eq!(p.pipeable(), None);
        }
        assert!(
            Payload::new("", vec![1]).summary(40).contains("unknown"),
            "a missing encoding should say so"
        );
    }

    #[test]
    fn json_that_is_not_valid_utf8_is_not_trusted() {
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

        assert_eq!(json("42").summary(30), "42");
    }

    #[test]
    fn a_json_payload_flattens_to_one_line_per_leaf() {
        let p =
            json(r#"{"customer":{"id":"8812"},"items":[{"sku":"A"},{"sku":"B"}],"amount":5000}"#);
        assert_eq!(
            flatten("input", &p),
            vec![
                ".input.amount=5000",
                ".input.customer.id=8812",
                ".input.items[0].sku=A",
                ".input.items[1].sku=B",
            ]
        );
    }

    #[test]
    fn empty_containers_and_scalars_keep_a_line() {
        assert_eq!(flatten("result", &json("{}")), vec![".result={}"]);
        assert_eq!(
            flatten("result", &json(r#"{"items":[]}"#)),
            vec![".result.items=[]"]
        );
        assert_eq!(flatten("result", &json("null")), vec![".result=null"]);
        assert_eq!(flatten("result", &json("true")), vec![".result=true"]);
    }

    #[test]
    fn text_and_unparseable_json_flatten_whole() {
        let text = Payload::new("text/plain", b"hello".to_vec());
        assert_eq!(flatten("input", &text), vec![".input=hello"]);
        assert_eq!(flatten("input", &json("not json")), vec![".input=not json"]);
    }

    #[test]
    fn ciphertext_and_opaque_bytes_flatten_to_nothing() {
        let enc = Payload::new("binary/encrypted", vec![1, 2, 3]);
        let raw = Payload::new("binary/plain", vec![1, 2, 3]);
        assert!(flatten("input", &enc).is_empty());
        assert!(flatten("input", &raw).is_empty());
    }

    #[test]
    fn the_cache_answers_what_flatten_would() {
        let mut cache = FlatCache::default();
        let p = json(r#"{"id":"8812"}"#);
        assert_eq!(cache.lines("input", &p), flatten("input", &p).as_slice());
        assert_eq!(cache.lines("input", &p), [".input.id=8812"]);
        assert_eq!(cache.len(), 1, "the same payload is one entry");
    }

    #[test]
    fn a_peek_does_not_keep_an_entry_alive() {
        let mut cache = FlatCache::default();
        let p = json("1");
        cache.lines("input", &p);
        cache.sweep();
        assert_eq!(
            cache.peek("input", &p),
            Some([".input=1".to_string()].as_slice())
        );
        cache.sweep();
        assert!(cache.is_empty(), "only `lines` counts as a use");
    }

    #[test]
    fn the_label_is_part_of_the_key() {
        let mut cache = FlatCache::default();
        let p = json("1");
        assert_eq!(cache.lines("input", &p), [".input=1"]);
        assert_eq!(cache.lines("result", &p), [".result=1"]);
    }

    #[test]
    fn a_sweep_keeps_only_what_the_last_pass_used() {
        let mut cache = FlatCache::default();
        let kept = json("1");
        let dropped = json("2");
        cache.lines("input", &kept);
        cache.lines("input", &dropped);
        cache.sweep();
        cache.lines("input", &kept);
        cache.sweep();
        assert_eq!(cache.len(), 1, "a payload no longer on screen is forgotten");
        assert_eq!(cache.lines("input", &kept), [".input=1"]);
    }

    #[test]
    fn payloads_pipe_as_one_object_keyed_by_label() {
        let payloads = vec![
            ("input".to_string(), json(r#"{"amount":100}"#)),
            ("result".to_string(), json(r#""charged""#)),
        ];
        let (out, skipped) = payloads_as_json(&payloads);
        let out = out.expect("something to pipe");
        assert!(skipped.is_empty());

        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["input"]["amount"], 100);
        assert_eq!(v["result"], "charged");
    }

    #[test]
    fn indexed_arguments_keep_their_labels() {
        let payloads = vec![
            ("input[0]".to_string(), json("1")),
            ("input[1]".to_string(), json(r#""two""#)),
        ];
        let (out, _) = payloads_as_json(&payloads);
        let v: serde_json::Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(v["input[0]"], 1);
        assert_eq!(v["input[1]"], "two");
    }

    #[test]
    fn unpipeable_payloads_are_reported_rather_than_breaking_the_object() {
        let payloads = vec![
            ("input".to_string(), json("42")),
            (
                "result".to_string(),
                Payload::new("binary/encrypted", vec![0u8; 8]),
            ),
        ];
        let (out, skipped) = payloads_as_json(&payloads);
        let v: serde_json::Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(v["input"], 42);
        assert!(v.get("result").is_none(), "ciphertext must not be embedded");
        assert_eq!(skipped, ["result"], "and the reader is told which");
    }

    #[test]
    fn a_row_with_nothing_pipeable_yields_no_json() {
        let payloads = vec![(
            "input".to_string(),
            Payload::new("binary/encrypted", vec![0u8; 8]),
        )];
        let (out, skipped) = payloads_as_json(&payloads);
        assert_eq!(out, None, "an empty object is not worth piping");
        assert_eq!(skipped, ["input"]);
        assert_eq!(payloads_as_json(&[]), (None, Vec::new()));
    }

    #[test]
    fn a_broken_json_payload_is_embedded_as_a_string_rather_than_lost() {
        let payloads = vec![
            ("input".to_string(), json("{not json")),
            ("result".to_string(), json("1")),
        ];
        let (out, skipped) = payloads_as_json(&payloads);
        let v: serde_json::Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(v["input"], "{not json");
        assert_eq!(v["result"], 1);
        assert!(skipped.is_empty());
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

    #[test]
    fn a_single_payload_is_unwrapped_for_pasting() {
        let (json, _) = payloads_as_json(&[(
            "result".into(),
            Payload::new("json/plain", br#"{"total":42}"#.to_vec()),
        )]);
        let out = unwrap_single(&json.unwrap()).unwrap();
        assert!(out.contains(r#""total": 42"#), "{out}");
        assert!(
            !out.contains("result"),
            "the wrapper key should be gone: {out}"
        );
    }

    #[test]
    fn a_single_string_payload_is_not_requoted() {
        let (json, _) = payloads_as_json(&[(
            "result".into(),
            Payload::new("text/plain", b"already text".to_vec()),
        )]);
        assert_eq!(unwrap_single(&json.unwrap()).unwrap(), "already text");
    }

    #[test]
    fn several_payloads_keep_their_keys() {
        let (json, _) = payloads_as_json(&[
            ("input[0]".into(), Payload::new("json/plain", b"1".to_vec())),
            ("input[1]".into(), Payload::new("json/plain", b"2".to_vec())),
        ]);
        assert_eq!(unwrap_single(&json.unwrap()), None, "must not unwrap");
    }

    #[test]
    fn a_codec_may_name_its_own_encoding() {
        let p = Payload::new("binary/aes_comp", vec![0; 10232]);
        assert!(
            p.needs_codec(),
            "a custom codec encoding still needs a codec"
        );
        assert_eq!(
            p.render(),
            Rendered::Encrypted {
                bytes: 10232,
                encoding: "binary/aes_comp".into()
            },
            "and the badge names the encoding rather than guessing at `encrypted`"
        );
    }

    #[test]
    fn readable_encodings_never_ask_for_a_codec() {
        for e in ["json/plain", "json/protobuf", "text/plain", "binary/null"] {
            assert!(
                !Payload::new(e, b"{}".to_vec()).needs_codec(),
                "{e} is readable as it stands"
            );
        }
    }

    #[test]
    fn a_malformed_payload_is_not_sent_to_a_codec() {
        assert!(!Payload::new("", vec![1, 2, 3]).needs_codec());
    }
}
