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
    /// Not readable without the user's codec server. Renders as a badge until a decode
    /// resolves, the value is not lost, it is just not readable yet. The encoding is carried
    /// so the badge can name it: a codec names its output itself, and `binary/aes_comp` is
    /// as legitimate as `binary/encrypted`.
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

    /// Encodings a codec server has no part in: either we can read them, or they are
    /// Temporal's own raw-bytes encodings, which no codec produced and none will decode.
    const NOT_CODEC: [&'static str; 6] = [
        "json/plain",
        "json/protobuf",
        "text/plain",
        "binary/null",
        // Raw bytes by convention, not ciphertext. Sending these to a codec would earn an
        // error for a payload that is simply not text.
        "binary/plain",
        "binary/protobuf",
    ];

    /// Whether reading this needs a round trip to the user's codec server.
    ///
    /// Any encoding outside [`Self::NOT_CODEC`], not just Temporal's sample
    /// `binary/encrypted`. A codec names its own output and real ones do: LORA's writes
    /// `binary/aes_comp`, Temporal's own compression sample writes `binary/deflate`.
    /// Matching one sample's name means never calling the codec for the others, which is
    /// indistinguishable from a codec server that is not working.
    ///
    /// The cost of being wrong is asymmetric. Offering a custom converter's output to a
    /// codec earns one error; refusing to offer a codec's output leaves the value unread
    /// with nothing on screen to explain why.
    ///
    /// An empty encoding is excluded: that is a malformed payload, not ciphertext.
    pub fn needs_codec(&self) -> bool {
        !self.encoding.is_empty() && !Self::NOT_CODEC.contains(&self.encoding.as_str())
    }

    /// How to show it.
    ///
    /// The encodings are Temporal's own. Anything unrecognised is opaque rather than
    /// optimistically decoded as UTF-8: a payload from a custom converter can be arbitrary
    /// bytes, and printing those into a terminal is how you end up with a corrupted screen.
    pub fn render(&self) -> Rendered {
        match self.encoding.as_str() {
            "binary/null" => Rendered::Null,
            // `json/protobuf` is proto3-JSON, still JSON text on the wire.
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

    /// A single line, for a row that has no space for the whole value.
    pub fn summary(&self, width: usize) -> String {
        match self.render() {
            Rendered::Null => "null".into(),
            Rendered::Encrypted { bytes, encoding } => {
                format!("🔒 {encoding}, {bytes} bytes")
            }
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
    /// `None` when there is nothing meaningful to pipe, piping ciphertext or an opaque blob
    /// into `jq` produces a parse error that says nothing useful about why.
    pub fn pipeable(&self) -> Option<&[u8]> {
        match self.encoding.as_str() {
            "json/plain" | "json/protobuf" | "text/plain" => Some(&self.data),
            _ => None,
        }
    }
}

/// The payloads of one row, gathered into a single JSON object for piping.
///
/// Returns the JSON and the labels that could not be included.
///
/// A row usually carries more than one payload, an activity has both an `input` and a
/// `result`, so "pipe the payload" is ambiguous. Piping an object keyed by label removes the
/// ambiguity and makes the obvious `jq` expressions work: `.` shows everything, `.result`
/// picks one, `.input[1]` picks an argument.
///
/// A `json/plain` payload is embedded as the value it already is rather than as a string, so
/// `.result.total` works without a second parse. One that claims JSON but does not parse is
/// embedded as a string, it is still worth seeing, and a broken value should not make the
/// whole object unpipeable. Anything not textual is left out and reported, because piping
/// ciphertext into `jq` produces a parse error that explains nothing.
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

/// The sole value of a one-key JSON object, re-rendered without the wrapper.
///
/// `payloads_as_json` always builds an object; for a single payload the key is noise the
/// reader has to strip before pasting. A string is handed back raw rather than re-quoted,
/// since pasting `"abc"` where `abc` was meant is the same mistake one level down.
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

/// One payload as `path=value` lines, for `/` to match on.
///
/// Substring search over pretty-printed JSON finds `8812` but cannot tell a customer id from
/// an amount; one leaf per line with its path in front makes `/customer.id=8812` a precise
/// query and keeps `/8812` working. `key=value` is the form event fields already search as.
///
/// Only what is readable: ciphertext still waiting on the codec and opaque bytes yield
/// nothing, so a search never decodes. A payload the codec already answered was swapped for
/// its plaintext and flattens like any other.
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

/// Flattened payloads, kept between searches.
///
/// `/` rebuilds every row's label on each search, and again for every page a read-on
/// search pulls in, so without this a long run's payloads are parsed once per keystroke.
/// Keyed by content rather than by event: a payload the codec answers comes back as new
/// bytes and so as a new entry, and nothing has to be told to invalidate.
///
/// Bounded by [`FlatCache::sweep`], which drops whatever the last pass did not ask for.
/// What survives is one flattened copy of the history on screen, and leaving a history
/// frees it on the next search.
#[derive(Debug, Default)]
pub struct FlatCache {
    entries: std::collections::HashMap<u64, (u64, Vec<String>)>,
    pass: u64,
}

impl FlatCache {
    /// [`flatten`], computed once per distinct label and payload.
    pub fn lines(&mut self, label: &str, p: &Payload) -> &[String] {
        let pass = self.pass;
        let entry = self
            .entries
            .entry(Self::key(label, p))
            .or_insert_with(|| (pass, flatten(label, p)));
        entry.0 = pass;
        &entry.1
    }

    /// What is cached for this payload, without counting as a use: a read after the pass
    /// that swept must not keep an entry alive into the next one.
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

    /// End a pass: forget every entry it did not use.
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
        // Unquoted, so `/status=APPROVED` matches without the reader typing JSON quotes.
        Value::String(s) => out.push(format!("{path}={s}")),
        other => out.push(format!("{path}={other}")),
    }
}

/// Pretty-print JSON, or hand back the input unchanged when it is not JSON.
///
/// Payloads claim `json/plain` and are usually right, but a workflow can put anything in one.
/// A value that does not parse is shown as it arrived rather than rejected, seeing the raw
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
        // Optimistically decoding arbitrary bytes as UTF-8 is how a terminal ends up full of
        // control characters. `binary/plain` is raw bytes by convention and `""` is
        // malformed; neither is a codec's output, so neither becomes a decode request.
        for enc in ["binary/plain", "binary/protobuf", ""] {
            let p = Payload::new(enc, vec![0xff, 0xfe, 0x00, 0x01]);
            match p.render() {
                Rendered::Opaque { bytes, .. } => assert_eq!(bytes, 4),
                other => panic!("{enc} should be opaque, got {other:?}"),
            }
            assert_eq!(p.pipeable(), None);
            assert!(!p.needs_codec(), "{enc} must not be sent to a codec");
        }
        // An encoding we do not recognise may well be a codec's; offering it is the only way
        // to find out, and costs one error if it is not.
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
        // An empty object is still a value someone may search for, dropping it would make
        // `/items=[]` find nothing on the one row where it is true.
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
        // Searching must not reach for the codec; only what is already readable counts.
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
        // The same bytes as an input and as a result flatten to different paths.
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
        // "Pipe the payload" is ambiguous when a row carries two. An object makes the
        // obvious jq expressions work.
        let payloads = vec![
            ("input".to_string(), json(r#"{"amount":100}"#)),
            ("result".to_string(), json(r#""charged""#)),
        ];
        let (out, skipped) = payloads_as_json(&payloads);
        let out = out.expect("something to pipe");
        assert!(skipped.is_empty());

        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        // Embedded as values, not as strings: `.input.amount` must work without a re-parse.
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
        // One malformed value must not make the whole row unpipeable.
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
        // `<leader>yr` on an ordinary activity should give the result itself.
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
        // Two arguments are only distinguishable by label, so the object stays.
        let (json, _) = payloads_as_json(&[
            ("input[0]".into(), Payload::new("json/plain", b"1".to_vec())),
            ("input[1]".into(), Payload::new("json/plain", b"2".to_vec())),
        ]);
        assert_eq!(unwrap_single(&json.unwrap()), None, "must not unwrap");
    }

    #[test]
    fn a_codec_may_name_its_own_encoding() {
        // LORA's data converter writes `binary/aes_comp`. Matching only Temporal's sample
        // name meant never calling the codec for these, which is indistinguishable from a
        // codec server that is not working.
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
        // No encoding at all is a broken payload, not ciphertext; a codec has nothing to do
        // with it and the round trip would only fail slowly.
        assert!(!Payload::new("", vec![1, 2, 3]).needs_codec());
    }
}
