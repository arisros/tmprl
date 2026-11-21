//! `gf` with no resolver: the stack trace, the name and the language server, over the repository's files.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tmprl_core::lsp;
use tmprl_core::named::{Place, Role};

// A language server's first answer waits on its index of the whole workspace.
const SERVER_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Default)]
pub(super) struct RepoSearch {
    pub roots: Vec<String>,
    pub home: Option<String>,
    pub lsp: bool,
    pub servers: Vec<(String, String)>,
}

fn git(args: &[&str], dir: &Path) -> Option<String> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The failure's stack trace, then the registered name in the source text and from each
/// root's language server, looked for in `[source] roots`, or with none the repository tmprl
/// runs in.
pub(super) fn find_in_repo(
    traces: &[String],
    name: &str,
    search: &RepoSearch,
) -> Result<String, String> {
    const LARGEST_FILE: u64 = 1024 * 1024;
    const MOST_BYTES: u64 = 256 * 1024 * 1024;
    let roots: Vec<PathBuf> = if search.roots.is_empty() {
        let cwd = std::env::current_dir().map_err(|e| format!("no working directory: {e}"))?;
        let top = git(&["rev-parse", "--show-toplevel"], &cwd)
            .map(|s| PathBuf::from(s.trim()))
            .unwrap_or(cwd);
        vec![top]
    } else {
        search
            .roots
            .iter()
            .map(|r| match (r.strip_prefix("~/"), search.home.as_deref()) {
                (Some(rest), Some(home)) => Path::new(home).join(rest),
                _ => PathBuf::from(r),
            })
            .collect()
    };
    let listed: Vec<(PathBuf, Vec<String>)> = roots
        .into_iter()
        .map(|root| {
            let root = std::fs::canonicalize(&root).unwrap_or(root);
            let files = repo_files(&root);
            (root, files)
        })
        .collect();
    for (root, files) in &listed {
        if let Some(out) = tmprl_core::locate::answer(traces, files, root) {
            return Ok(out);
        }
    }
    let mut read: Vec<(&Path, &str, String)> = Vec::new();
    let mut bytes = 0;
    'roots: for (root, files) in &listed {
        for file in files.iter().filter(|f| tmprl_core::named::is_source(f)) {
            let path = root.join(file);
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.len() > LARGEST_FILE {
                continue;
            }
            if bytes + meta.len() > MOST_BYTES {
                break 'roots;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                bytes += meta.len();
                read.push((root, file, text));
            }
        }
    }
    let sources: Vec<(&Path, &str, &str)> =
        read.iter().map(|(r, f, t)| (*r, *f, t.as_str())).collect();
    let found = tmprl_core::named::search(name, &sources);

    let mut places = Vec::new();
    let mut failed = Vec::new();
    if search.lsp {
        let mut leads = tmprl_core::named::variants(name);
        leads.extend(found.fns.iter().cloned());
        let mut seen = std::collections::HashSet::new();
        leads.retain(|l| seen.insert(l.clone()));
        for (root, _) in &listed {
            let top: Vec<String> = std::fs::read_dir(root)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            for (_, command) in lsp::servers(&top, &search.servers) {
                match ask_server(
                    &command,
                    root,
                    &leads,
                    &found.idents,
                    search.home.as_deref(),
                ) {
                    Ok(more) => places.extend(more),
                    Err(why) => failed.push(why),
                }
            }
        }
    }
    places.extend(found.places);
    tmprl_core::named::write(places).ok_or_else(|| {
        let at: Vec<String> = listed.iter().map(|(r, _)| r.display().to_string()).collect();
        let mut why = format!(
            "no stack trace frame and no `{name}` under {}; set `[source] roots` or a `[source] command`",
            at.join(", ")
        );
        for f in failed {
            why.push_str(&format!("; {f}"));
        }
        why
    })
}

/// A root's files: git's own list, so `.gitignore` holds; outside a repository, a bounded walk.
fn repo_files(root: &Path) -> Vec<String> {
    const MOST_FILES: usize = 200_000;
    if let Some(list) = git(
        &["ls-files", "--cached", "--others", "--exclude-standard"],
        root,
    ) {
        return list.lines().map(String::from).collect();
    }
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
            if out.len() >= MOST_FILES {
                return out;
            }
        }
    }
    out
}

/// One language server, spoken to over its stdio until dropped, which shuts it down.
struct Server {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    id: u64,
    deadline: Instant,
    trouble: Option<String>,
}

impl Server {
    fn start(program: &str, args: &[String], root: &Path) -> Result<Self, String> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{program}: {e}"))?;
        let (Some(stdin), Some(mut stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(format!("{program}: no stdio"));
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut reader = lsp::Reader::default();
            let mut buf = [0u8; 64 * 1024];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    return;
                }
                reader.push(&buf[..n]);
                while let Some(msg) = reader.message() {
                    if tx.send(msg).is_err() {
                        return;
                    }
                }
            }
        });
        let mut server = Self {
            child,
            stdin,
            rx,
            id: 0,
            deadline: Instant::now() + SERVER_TIMEOUT,
            trouble: None,
        };
        server.call(|id| lsp::initialize(id, root))?;
        server.send(&lsp::notification("initialized", json!({})))?;
        Ok(server)
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        self.stdin
            .write_all(&lsp::frame(msg))
            .and_then(|()| self.stdin.flush())
            .map_err(|e| e.to_string())
    }

    fn call(&mut self, make: impl FnOnce(u64) -> Value) -> Result<Value, String> {
        self.id += 1;
        let id = self.id;
        self.send(&make(id))?;
        loop {
            let left = self
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or("timed out")?;
            let msg = self
                .rx
                .recv_timeout(left)
                .map_err(|_| "timed out or exited".to_string())?;
            if let Some(c) = lsp::complaint(&msg) {
                self.trouble.get_or_insert(c);
            }
            if let Some(answer) = lsp::reply_to_server(&msg) {
                self.send(&answer)?;
                continue;
            }
            if let Some(result) = lsp::reply(&msg, id) {
                return result;
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let id = self.id + 1;
        let _ = self.send(&lsp::request(id, "shutdown", Value::Null));
        let _ = self.send(&lsp::notification("exit", Value::Null));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Where a server says each lead function is defined, and where each identifier bound to the
/// name is used.
fn ask_server(
    command: &str,
    root: &Path,
    leads: &[String],
    idents: &[String],
    home: Option<&str>,
) -> Result<Vec<Place>, String> {
    let (program, args) = tmprl_core::source::argv(command, home)
        .ok_or_else(|| "an empty server command".to_string())?;
    let says = Path::new(&program)
        .file_name()
        .map_or_else(|| program.clone(), |n| n.to_string_lossy().into_owned());
    let mut server = Server::start(&program, &args, root).map_err(|e| format!("{says}: {e}"))?;
    let under = root.display().to_string();
    let place = |path: String, line: u32, role: Role, label: String| Place {
        path,
        root: under.clone(),
        line: line + 1,
        role,
        label: format!("{label}, says {says}"),
    };
    let mut out = Vec::new();
    for lead in leads {
        let result = server
            .call(|id| lsp::symbols(id, lead))
            .map_err(|e| format!("{says}: {e}"))?;
        let symbols = lsp::parse_symbols(&result);
        for s in lsp::named(&symbols, lead, &under)
            .into_iter()
            .filter(|s| lsp::is_callable(s.kind))
        {
            out.push(place(
                s.path.clone(),
                s.line,
                Role::Defines,
                format!("defines `{lead}`"),
            ));
        }
    }
    for ident in idents {
        let result = server
            .call(|id| lsp::symbols(id, ident))
            .map_err(|e| format!("{says}: {e}"))?;
        let symbols = lsp::parse_symbols(&result);
        for s in lsp::named(&symbols, ident, &under)
            .into_iter()
            .filter(|s| lsp::is_value(s.kind))
        {
            let refs = server
                .call(|id| lsp::references(id, &s.path, s.line, s.col))
                .map_err(|e| format!("{says}: {e}"))?;
            for (path, line) in lsp::parse_locations(&refs) {
                if path.starts_with(&under) {
                    out.push(place(path, line, Role::Uses, format!("uses `{ident}`")));
                }
            }
        }
    }
    match server.trouble.take() {
        Some(why) if out.is_empty() => Err(format!("{says}: {why}")),
        _ => Ok(out),
    }
}
