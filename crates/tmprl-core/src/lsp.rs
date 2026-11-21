//! gf's language servers: which one a repository calls for, and the JSON-RPC that asks it where a name is defined and used.

use std::path::Path;

use serde_json::{Value, json};

pub const SERVERS: [(&str, &[&str], &str); 5] = [
    ("go", &["go.mod"], "gopls"),
    (
        "typescript",
        &["tsconfig.json", "package.json"],
        "typescript-language-server --stdio",
    ),
    (
        "python",
        &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
        ],
        "pyright-langserver --stdio",
    ),
    (
        "java",
        &["pom.xml", "build.gradle", "build.gradle.kts"],
        "jdtls",
    ),
    ("csharp", &[".sln", ".csproj"], "csharp-ls"),
];

pub fn servers(top: &[String], overrides: &[(String, String)]) -> Vec<(String, String)> {
    SERVERS
        .iter()
        .filter(|(_, marks, _)| {
            top.iter().any(|f| {
                marks
                    .iter()
                    .any(|m| f == m || (m.starts_with('.') && f.ends_with(m)))
            })
        })
        .map(|(lang, _, default)| {
            let command = overrides
                .iter()
                .find(|(l, _)| l == lang)
                .map_or(*default, |(_, c)| c.as_str());
            (lang.to_string(), command.to_string())
        })
        .collect()
}

pub fn frame(msg: &Value) -> Vec<u8> {
    let body = msg.to_string();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

#[derive(Debug, Default)]
pub struct Reader {
    buf: Vec<u8>,
}

impl Reader {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn message(&mut self) -> Option<Value> {
        loop {
            let end = self.buf.windows(4).position(|w| w == b"\r\n\r\n")?;
            let head = String::from_utf8_lossy(&self.buf[..end]).into_owned();
            let len = head.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim()
                    .eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse::<usize>().ok())?
            });
            let Some(len) = len else {
                self.buf.drain(..end + 4);
                continue;
            };
            if self.buf.len() < end + 4 + len {
                return None;
            }
            let body: Vec<u8> = self.buf.drain(..end + 4 + len).skip(end + 4).collect();
            if let Ok(msg) = serde_json::from_slice(&body) {
                return Some(msg);
            }
        }
    }
}

pub fn path_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.display().to_string().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?.as_bytes();
    let mut out = Vec::with_capacity(rest.len());
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == b'%'
            && let Some(b) = rest
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(rest[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

pub fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

pub fn initialize(id: u64, root: &Path) -> Value {
    let uri = path_uri(root);
    request(
        id,
        "initialize",
        json!({
            "processId": null,
            "clientInfo": {"name": "tmprl"},
            "rootUri": uri,
            "workspaceFolders": [{"uri": uri, "name": "root"}],
            "capabilities": {
                "workspace": {"symbol": {}, "workspaceFolders": true, "configuration": true},
                "textDocument": {"references": {}},
                "window": {"workDoneProgress": false}
            }
        }),
    )
}

pub fn symbols(id: u64, query: &str) -> Value {
    request(id, "workspace/symbol", json!({"query": query}))
}

pub fn references(id: u64, path: &str, line: u32, col: u32) -> Value {
    request(
        id,
        "textDocument/references",
        json!({
            "textDocument": {"uri": path_uri(Path::new(path))},
            "position": {"line": line, "character": col},
            "context": {"includeDeclaration": false}
        }),
    )
}

pub fn reply_to_server(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?;
    let method = msg.get("method")?.as_str()?;
    let result = if method == "workspace/configuration" {
        let n = msg["params"]["items"].as_array().map_or(0, Vec::len);
        Value::Array(vec![Value::Null; n])
    } else {
        Value::Null
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

pub fn complaint(msg: &Value) -> Option<String> {
    let method = msg.get("method")?.as_str()?;
    if !matches!(method, "window/showMessage" | "window/logMessage") || msg["params"]["type"] != 1 {
        return None;
    }
    let text = msg["params"]["message"].as_str()?;
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    Some(line.chars().take(200).collect())
}

pub fn reply(msg: &Value, id: u64) -> Option<Result<Value, String>> {
    if msg.get("method").is_some() || msg.get("id").and_then(Value::as_u64) != Some(id) {
        return None;
    }
    Some(match msg.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
        None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: u64,
    pub path: String,
    pub line: u32,
    pub col: u32,
}

fn location(v: &Value) -> Option<(String, u32, u32)> {
    let path = uri_path(v.get("uri")?.as_str()?)?;
    let start = &v["range"]["start"];
    let line = start["line"].as_u64().unwrap_or(0) as u32;
    let col = start["character"].as_u64().unwrap_or(0) as u32;
    Some((path, line, col))
}

pub fn parse_symbols(result: &Value) -> Vec<Symbol> {
    result
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let (path, line, col) = location(s.get("location")?)?;
            Some(Symbol {
                name: s.get("name")?.as_str()?.to_string(),
                kind: s.get("kind")?.as_u64()?,
                path,
                line,
                col,
            })
        })
        .collect()
}

pub fn parse_locations(result: &Value) -> Vec<(String, u32)> {
    result
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(location)
        .map(|(p, l, _)| (p, l))
        .collect()
}

pub fn is_callable(kind: u64) -> bool {
    matches!(kind, 6 | 9 | 12)
}

pub fn is_value(kind: u64) -> bool {
    matches!(kind, 7 | 8 | 13 | 14)
}

pub fn named<'a>(symbols: &'a [Symbol], want: &str, root: &str) -> Vec<&'a Symbol> {
    symbols
        .iter()
        .filter(|s| s.path.starts_with(root))
        .filter(|s| {
            let base = s.name.split('(').next().unwrap_or(&s.name).trim();
            base == want || base.ends_with(&format!(".{want}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repositorys_top_files_pick_its_servers() {
        let top = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            servers(&top(&["go.mod", "README.md"]), &[]),
            [("go".to_string(), "gopls".to_string())]
        );
        let both = servers(
            &top(&["package.json", "Orders.csproj"]),
            &[("csharp".into(), "~/bin/csharp-ls".into())],
        );
        assert_eq!(both[0].0, "typescript");
        assert_eq!(both[1], ("csharp".into(), "~/bin/csharp-ls".into()));
        assert!(servers(&top(&["Cargo.toml"]), &[]).is_empty());
    }

    #[test]
    fn messages_are_cut_out_however_the_bytes_arrive() {
        let a = frame(&json!({"id": 1, "result": "é"}));
        let b = frame(&json!({"method": "window/logMessage"}));
        let all: Vec<u8> = a.iter().chain(b.iter()).copied().collect();
        let mut r = Reader::default();
        let mut got = Vec::new();
        for chunk in all.chunks(7) {
            r.push(chunk);
            while let Some(m) = r.message() {
                got.push(m);
            }
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0]["result"], "é");
        assert_eq!(got[1]["method"], "window/logMessage");
    }

    #[test]
    fn a_path_survives_the_round_trip_through_a_uri() {
        let p = Path::new("/src/my orders/a#b.go");
        let uri = path_uri(p);
        assert_eq!(uri, "file:///src/my%20orders/a%23b.go");
        assert_eq!(uri_path(&uri).as_deref(), Some("/src/my orders/a#b.go"));
        assert_eq!(uri_path("https://x"), None);
    }

    #[test]
    fn the_servers_requests_are_answered_and_replies_matched() {
        let config =
            json!({"id": 4, "method": "workspace/configuration", "params": {"items": [{}, {}]}});
        assert_eq!(
            reply_to_server(&config).unwrap()["result"],
            json!([null, null])
        );
        let other = json!({"id": "x", "method": "window/workDoneProgress/create"});
        assert_eq!(reply_to_server(&other).unwrap()["result"], Value::Null);
        assert_eq!(reply_to_server(&json!({"method": "$/progress"})), None);

        let failed = json!({"method": "window/logMessage", "params": {"type": 1, "message": "\ninitial workspace load failed\n\tmore"}});
        assert_eq!(
            complaint(&failed).as_deref(),
            Some("initial workspace load failed")
        );
        let info = json!({"method": "window/showMessage", "params": {"type": 3, "message": "Finished loading packages."}});
        assert_eq!(complaint(&info), None);

        assert_eq!(
            reply(&json!({"id": 2, "result": [1]}), 2),
            Some(Ok(json!([1])))
        );
        assert_eq!(reply(&json!({"id": 3, "result": [1]}), 2), None);
        assert_eq!(
            reply(&json!({"id": 2, "error": {"message": "no"}}), 2),
            Some(Err("no".into()))
        );
    }

    #[test]
    fn symbols_are_matched_by_their_exact_name_under_the_root() {
        let result = json!([
            {"name": "CalculateDuties", "kind": 12, "location": {"uri": "file:///r/a.go", "range": {"start": {"line": 131, "character": 5}}}},
            {"name": "Activities.Charge", "kind": 6, "location": {"uri": "file:///r/b.go", "range": {"start": {"line": 9, "character": 0}}}},
            {"name": "charge(int)", "kind": 6, "location": {"uri": "file:///r/C.java", "range": {"start": {"line": 3, "character": 4}}}},
            {"name": "CalculateDutiesTotal", "kind": 12, "location": {"uri": "file:///r/c.go", "range": {"start": {"line": 1, "character": 0}}}},
            {"name": "CalculateDuties", "kind": 12, "location": {"uri": "file:///go/pkg/mod/x.go", "range": {"start": {"line": 1, "character": 0}}}}
        ]);
        let syms = parse_symbols(&result);
        assert_eq!(syms.len(), 5);
        let found = named(&syms, "CalculateDuties", "/r");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (131, 5));
        assert_eq!(named(&syms, "Charge", "/r").len(), 1);
        assert_eq!(named(&syms, "charge", "/r").len(), 1);
        assert!(is_callable(12) && is_callable(6) && !is_callable(14));
        assert!(is_value(14) && !is_value(12));
        let refs =
            json!([{"uri": "file:///r/w.go", "range": {"start": {"line": 75, "character": 2}}}]);
        assert_eq!(parse_locations(&refs), [("/r/w.go".to_string(), 75)]);
    }
}
