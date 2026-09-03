//! The codec-server round trip.
//!
//! When a cluster encrypts payloads, only a service the *user* runs can read them. Temporal's
//! contract for that service is plain HTTP rather than gRPC:
//!
//! ```text
//! POST {endpoint}/decode
//! Content-Type: application/json
//! X-Namespace: {namespace}
//! Authorization: {auth}          (only when configured)
//!
//! {"payloads":[{"metadata":{"encoding":"<base64>"},"data":"<base64>"}]}
//! ```
//!
//! The body is **proto3-JSON**, which is why every `bytes` field — the payload data *and*
//! each metadata value — is base64. Sending `metadata.encoding` as the plain string
//! `binary/encrypted` is the mistake this module exists to not make: a conforming server
//! decodes it as base64, gets nonsense, and refuses the payload.
//!
//! The response is the same shape, with the payloads decoded.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use tmprl_core::payload::Payload;

use super::OpError;

/// A configured codec server.
#[derive(Debug, Clone)]
pub struct Codec {
    endpoint: String,
    auth: Option<String>,
    http: reqwest::Client,
}

impl Codec {
    pub fn new(endpoint: impl Into<String>, auth: Option<String>) -> Self {
        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            auth,
            http: reqwest::Client::new(),
        }
    }

    /// Decode a batch of payloads.
    ///
    /// Batched rather than one call per payload: a row carries several, and a codec server is
    /// usually a network hop away.
    ///
    /// The server is required to return exactly as many payloads as it was given, in order —
    /// that is what makes the result assignable back to what was sent. A server that returns
    /// a different number is a protocol error rather than something to guess around, because
    /// pairing them up wrongly would show one payload's plaintext under another's label.
    pub async fn decode(
        &self,
        namespace: &str,
        payloads: &[Payload],
    ) -> Result<Vec<Payload>, OpError> {
        if payloads.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/decode", self.endpoint);
        let body = serde_json::json!({
            "payloads": payloads.iter().map(to_wire).collect::<Vec<_>>(),
        });

        let mut req = self
            .http
            .post(&url)
            .header("X-Namespace", namespace)
            .json(&body);
        if let Some(auth) = &self.auth {
            req = req.header("Authorization", auth);
        }

        let resp = req.send().await.map_err(|e| OpError::Codec {
            message: format!("could not reach the codec server at {url}: {e}"),
        })?;

        let status = resp.status();
        if !status.is_success() {
            // The server's own body is usually the diagnosis — a wrong path, a rejected
            // credential — so it is shown rather than just the status code.
            let detail = resp.text().await.unwrap_or_default();
            let detail = detail.trim();
            return Err(OpError::Codec {
                message: if detail.is_empty() {
                    format!("codec server returned {status}")
                } else {
                    format!("codec server returned {status}: {detail}")
                },
            });
        }

        let value: serde_json::Value = resp.json().await.map_err(|e| OpError::Codec {
            message: format!("codec server sent a body that is not JSON: {e}"),
        })?;
        let out = value
            .get("payloads")
            .and_then(|p| p.as_array())
            .ok_or_else(|| OpError::Codec {
                message: "codec server sent no `payloads` array".into(),
            })?;

        if out.len() != payloads.len() {
            return Err(OpError::Codec {
                message: format!(
                    "codec server returned {} payload(s) for {} sent; they cannot be paired up",
                    out.len(),
                    payloads.len()
                ),
            });
        }
        out.iter().map(from_wire).collect()
    }
}

/// Domain payload to proto3-JSON. Both `data` and every metadata value are `bytes`, so both
/// are base64.
fn to_wire(p: &Payload) -> serde_json::Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "encoding".into(),
        serde_json::Value::String(B64.encode(p.encoding.as_bytes())),
    );
    if let Some(t) = &p.type_hint {
        metadata.insert(
            "type".into(),
            serde_json::Value::String(B64.encode(t.as_bytes())),
        );
    }
    serde_json::json!({
        "metadata": metadata,
        "data": B64.encode(&p.data),
    })
}

/// proto3-JSON back to a domain payload.
///
/// A missing `data` is an empty payload, not an error: proto3-JSON omits fields at their
/// default, so a decoded empty value legitimately arrives with no `data` key at all.
fn from_wire(v: &serde_json::Value) -> Result<Payload, OpError> {
    let decode_b64 = |s: &str| -> Result<Vec<u8>, OpError> {
        B64.decode(s).map_err(|e| OpError::Codec {
            message: format!("codec server sent a field that is not base64: {e}"),
        })
    };

    let data = match v.get("data").and_then(|d| d.as_str()) {
        Some(s) => decode_b64(s)?,
        None => Vec::new(),
    };

    let meta = |key: &str| -> Result<Option<String>, OpError> {
        let Some(s) = v
            .pointer(&format!("/metadata/{key}"))
            .and_then(|m| m.as_str())
        else {
            return Ok(None);
        };
        let bytes = decode_b64(s)?;
        Ok(String::from_utf8(bytes).ok())
    };

    Ok(Payload {
        encoding: meta("encoding")?.unwrap_or_default(),
        type_hint: meta("type")?,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_goes_out_as_proto3_json_with_base64_everywhere() {
        // The mistake this pins: `encoding` is a bytes field, so it is base64 on the wire.
        // Sent as a plain string, a conforming server base64-decodes it into nonsense.
        let p = Payload::new("binary/encrypted", b"cipher".to_vec());
        let wire = to_wire(&p);

        assert_eq!(wire["data"], B64.encode(b"cipher"));
        assert_eq!(
            wire["metadata"]["encoding"],
            B64.encode(b"binary/encrypted")
        );
        assert_ne!(
            wire["metadata"]["encoding"], "binary/encrypted",
            "the encoding must not be sent as a plain string"
        );
    }

    #[test]
    fn a_type_hint_is_carried_only_when_present() {
        let mut p = Payload::new("json/plain", b"1".to_vec());
        assert!(to_wire(&p)["metadata"].get("type").is_none());

        p.type_hint = Some("Keyword".into());
        assert_eq!(to_wire(&p)["metadata"]["type"], B64.encode(b"Keyword"));
    }

    #[test]
    fn a_decoded_payload_round_trips_back() {
        let original = Payload::new("json/plain", br#"{"amount":100}"#.to_vec());
        let back = from_wire(&to_wire(&original)).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn a_payload_with_no_data_field_is_empty_rather_than_an_error() {
        // proto3-JSON omits fields at their default, so an empty decoded value arrives with
        // no `data` key at all.
        let v = serde_json::json!({"metadata": {"encoding": B64.encode(b"binary/null")}});
        let p = from_wire(&v).unwrap();
        assert_eq!(p.encoding, "binary/null");
        assert!(p.data.is_empty());
    }

    #[test]
    fn a_field_that_is_not_base64_is_reported() {
        let v = serde_json::json!({"data": "not base64!!", "metadata": {}});
        let err = from_wire(&v).unwrap_err();
        assert!(err.to_string().contains("base64"), "got {err}");
    }

    #[test]
    fn decoding_nothing_does_not_call_the_server() {
        // A round trip for an empty batch is a wasted network hop on every cursor move.
        let codec = Codec::new("http://127.0.0.1:1", None);
        let out = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(codec.decode("default", &[]))
            .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn the_endpoint_loses_a_trailing_slash() {
        // `//decode` is not the same path to every server.
        let codec = Codec::new("http://localhost:8081/", None);
        assert_eq!(codec.endpoint, "http://localhost:8081");
    }
}

/// End-to-end against a real HTTP server.
///
/// The unit tests above assert the *shape* of the body; these assert that a server on the
/// other end of a socket sees what Temporal's contract says it should. That distinction
/// matters here because the base64-metadata rule is exactly the sort of thing a shape
/// assertion can agree with while the wire is still wrong.
#[cfg(test)]
mod live {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A one-shot HTTP server. Returns its address and a handle to the request it saw.
    async fn serve(status: &'static str, body: &'static str) -> (String, Arc<Mutex<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(String::new()));
        let captured = seen.clone();

        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Read headers, then exactly Content-Length bytes of body.
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let len: usize = text
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length: ")
                                .or_else(|| l.strip_prefix("Content-Length: "))
                        })
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if buf.len() >= head_end + 4 + len {
                        *captured.lock().unwrap() = text;
                        break;
                    }
                }
            }
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
        });

        (format!("http://{addr}"), seen)
    }

    #[tokio::test]
    async fn the_server_sees_base64_metadata_and_the_namespace_header() {
        let decoded = concat!(
            r#"{"payloads":[{"metadata":{"encoding":"anNvbi9wbGFpbg=="},"#,
            r#""data":"eyJhbW91bnQiOjEwMH0="}]}"#
        );
        let (addr, seen) = serve("200 OK", decoded).await;
        let codec = Codec::new(addr, Some("Bearer tok".into()));

        let out = codec
            .decode(
                "payments",
                &[Payload::new("binary/encrypted", b"cipher".to_vec())],
            )
            .await
            .expect("a decode");

        // The decoded payload came back as plaintext JSON.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].encoding, "json/plain");
        assert_eq!(out[0].data, br#"{"amount":100}"#);

        let request = seen.lock().unwrap().clone();
        assert!(
            request.starts_with("POST /decode "),
            "wrong path:\n{request}"
        );
        assert!(
            request.contains("x-namespace: payments"),
            "namespace header missing:\n{request}"
        );
        assert!(
            request.contains("authorization: Bearer tok"),
            "auth missing:\n{request}"
        );
        // The contract's whole trap: `encoding` is a bytes field, so it is base64 on the
        // wire. A server that received the plain string would base64-decode it to nonsense.
        assert!(
            request.contains(&B64.encode(b"binary/encrypted")),
            "the encoding must be base64 on the wire:\n{request}"
        );
        assert!(
            !request.contains(r#""encoding":"binary/encrypted""#),
            "the encoding must not be sent as a plain string:\n{request}"
        );
    }

    #[tokio::test]
    async fn no_authorization_header_is_sent_when_none_is_configured() {
        // A codec server is a service the user runs; forwarding a credential they did not
        // configure would be a surprise.
        let (addr, seen) = serve("200 OK", r#"{"payloads":[{"metadata":{},"data":""}]}"#).await;
        let codec = Codec::new(addr, None);
        codec
            .decode("default", &[Payload::new("binary/encrypted", vec![1])])
            .await
            .unwrap();
        assert!(
            !seen
                .lock()
                .unwrap()
                .to_lowercase()
                .contains("authorization")
        );
    }

    #[tokio::test]
    async fn a_server_error_body_is_reported_rather_than_just_the_status() {
        let (addr, _) = serve("500 Internal Server Error", "key rotation in progress").await;
        let err = Codec::new(addr, None)
            .decode("default", &[Payload::new("binary/encrypted", vec![1])])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("500"), "got {err}");
        assert!(
            err.to_string().contains("key rotation"),
            "the server's own words: {err}"
        );
    }

    #[tokio::test]
    async fn a_short_response_is_refused_rather_than_mispaired() {
        // Pairing two sent payloads with one returned would show one value's plaintext
        // under the other's label.
        let (addr, _) = serve("200 OK", r#"{"payloads":[{"metadata":{},"data":""}]}"#).await;
        let err = Codec::new(addr, None)
            .decode(
                "default",
                &[
                    Payload::new("binary/encrypted", vec![1]),
                    Payload::new("binary/encrypted", vec![2]),
                ],
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot be paired up"), "got {err}");
    }

    #[tokio::test]
    async fn an_unreachable_codec_server_names_the_url() {
        // "connection refused" with no address is the least useful error there is.
        let err = Codec::new("http://127.0.0.1:1", None)
            .decode("default", &[Payload::new("binary/encrypted", vec![1])])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("127.0.0.1:1/decode"), "got {err}");
    }
}
