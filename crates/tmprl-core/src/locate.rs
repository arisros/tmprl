//! gf with no resolver: the frames of a failure's stack trace, found in the local repository.

use std::collections::HashMap;
use std::path::Path;

use crate::history::Failure;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub path: String,
    pub line: u32,
    pub func: Option<String>,
}

pub fn traces(failure: &Failure) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = Some(failure);
    while let Some(f) = at {
        if let Some(trace) = f.stack_trace.as_deref().filter(|t| !t.trim().is_empty()) {
            out.push(trace.to_string());
        }
        at = f.cause.as_deref();
    }
    out
}

pub fn frames(trace: &str) -> Vec<Frame> {
    let lines: Vec<&str> = trace.lines().map(str::trim).collect();
    let mut out = Vec::new();
    let mut python = false;
    for (i, line) in lines.iter().enumerate() {
        let frame = if let Some(f) = python_frame(line) {
            python = true;
            Some(f)
        } else {
            dotnet_frame(line)
                .or_else(|| at_frame(line))
                .or_else(|| go_frame(line, i.checked_sub(1).map(|p| lines[p])))
        };
        out.extend(frame);
    }
    if python {
        out.reverse();
    }
    out
}

fn python_frame(line: &str) -> Option<Frame> {
    let rest = line.strip_prefix("File \"")?;
    let (path, rest) = rest.split_once('"')?;
    let rest = rest.strip_prefix(", line ")?;
    let (num, func) = match rest.split_once(", in ") {
        Some((n, f)) => (n, Some(f.trim().to_string())),
        None => (rest, None),
    };
    Some(Frame {
        path: path.to_string(),
        line: num.trim().parse().ok()?,
        func,
    })
}

fn dotnet_frame(line: &str) -> Option<Frame> {
    let (left, num) = line.rsplit_once(":line ")?;
    let (at, path) = left.rsplit_once(" in ")?;
    Some(Frame {
        path: path.to_string(),
        line: num.trim().parse().ok()?,
        func: Some(name_only(at.strip_prefix("at ").unwrap_or(at))),
    })
}

fn at_frame(line: &str) -> Option<Frame> {
    let rest = line.strip_prefix("at ")?;
    let (func, place) = match rest.rfind('(') {
        Some(open) if rest.ends_with(')') => (
            Some(rest[..open].trim().to_string()),
            &rest[open + 1..rest.len() - 1],
        ),
        _ => (None, rest),
    };
    let place = strip_scheme(place);
    let parts: Vec<&str> = place.rsplitn(3, ':').collect();
    match parts.as_slice() {
        [col, num, path] if col.parse::<u32>().is_ok() && num.parse::<u32>().is_ok() => {
            Some(Frame {
                path: path.to_string(),
                line: num.parse().ok()?,
                func: func.filter(|f| !f.is_empty()),
            })
        }
        [num, file, ..] if num.parse::<u32>().is_ok() => {
            let file = match parts.len() {
                3 => format!("{}:{}", parts[2], file),
                _ => file.to_string(),
            };
            let func = func?;
            let path = if file.contains('/') {
                file
            } else {
                java_path(&func, &file)
            };
            Some(Frame {
                path,
                line: num.parse().ok()?,
                func: Some(func),
            })
        }
        _ => None,
    }
}

fn java_path(func: &str, file: &str) -> String {
    let mut parts: Vec<&str> = func.split('.').collect();
    parts.truncate(parts.len().saturating_sub(2));
    let package = parts
        .iter()
        .take_while(|p| p.chars().next().is_some_and(char::is_lowercase))
        .copied()
        .collect::<Vec<_>>();
    if package.is_empty() {
        file.to_string()
    } else {
        format!("{}/{file}", package.join("/"))
    }
}

fn go_frame(line: &str, previous: Option<&str>) -> Option<Frame> {
    let token = line.split_whitespace().next()?;
    let (path, num) = token.rsplit_once(':')?;
    if !path.ends_with(".go") {
        return None;
    }
    Some(Frame {
        path: path.to_string(),
        line: num.parse().ok()?,
        func: previous
            .filter(|p| !p.is_empty() && !p.contains(".go:"))
            .map(name_only),
    })
}

/// Drops the argument list, which is the last parenthesis: `main.(*A).Charge(0x1)` keeps its receiver.
fn name_only(func: &str) -> String {
    let func = func.trim();
    match func.rfind('(') {
        Some(i) if i > 0 && func.ends_with(')') => func[..i].trim().to_string(),
        _ => func.to_string(),
    }
}

fn strip_scheme(place: &str) -> &str {
    ["file://", "webpack-internal:///", "webpack://"]
        .iter()
        .find_map(|s| place.strip_prefix(s))
        .unwrap_or(place)
}

fn is_library(frame: &Frame) -> bool {
    const PATHS: [&str; 6] = [
        "/vendor/",
        "node_modules",
        "site-packages",
        "dist-packages",
        "go.temporal.io",
        "/go/pkg/mod/",
    ];
    const FUNCS: [&str; 7] = [
        "java.",
        "javax.",
        "jdk.",
        "sun.",
        "kotlin.",
        "io.temporal.",
        "Temporalio.",
    ];
    PATHS.iter().any(|p| frame.path.contains(p))
        || frame
            .func
            .as_deref()
            .is_some_and(|f| FUNCS.iter().any(|p| f.starts_with(p)))
}

fn components(path: &str) -> Vec<&str> {
    path.split(['/', '\\']).filter(|c| !c.is_empty()).collect()
}

/// A tie leaves the path unnamed rather than open the wrong file.
pub fn by_suffix<'a>(path: &str, files: &'a [String]) -> Option<&'a str> {
    let want = components(path);
    let name = *want.last()?;
    let mut best: Option<(usize, &str)> = None;
    let mut tied = false;
    for file in files.iter().filter(|f| f.ends_with(name)) {
        let have = components(file);
        let shared = want
            .iter()
            .rev()
            .zip(have.iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        if shared == 0 {
            continue;
        }
        match best {
            Some((n, _)) if shared < n => {}
            Some((n, _)) if shared == n => tied = true,
            _ => {
                best = Some((shared, file));
                tied = false;
            }
        }
    }
    best.filter(|_| !tied).map(|(_, f)| f)
}

pub fn answer(traces: &[String], files: &[String], root: &Path) -> Option<String> {
    let mut by_name: HashMap<&str, Vec<String>> = HashMap::new();
    for f in files {
        let name = f.rsplit(['/', '\\']).next().unwrap_or(f);
        by_name.entry(name).or_default().push(f.clone());
    }
    let mut found: Vec<(String, u32, String)> = Vec::new();
    for frame in traces.iter().flat_map(|t| frames(t)) {
        if is_library(&frame) {
            continue;
        }
        let name = frame.path.rsplit(['/', '\\']).next().unwrap_or(&frame.path);
        let Some(candidates) = by_name.get(name) else {
            continue;
        };
        let Some(file) = by_suffix(&frame.path, candidates) else {
            continue;
        };
        let at = root.join(file).display().to_string();
        if found.iter().any(|(p, l, _)| *p == at && *l == frame.line) {
            continue;
        }
        let label = frame.func.clone().unwrap_or_else(|| file.to_string());
        found.push((at, frame.line, label));
    }
    let (first, line, _) = found.first()?;
    let root = root.display();
    let mut out = format!("{first}:{line}\n{root}\n");
    if found.len() > 1 {
        for (i, (path, line, label)) in found.iter().enumerate() {
            let label = if i == 0 {
                format!("{label}, where it threw")
            } else {
                label.clone()
            };
            out.push_str(&format!("{path}:{line}\t{root}\t{label}\n"));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(frames: &[Frame]) -> Vec<(String, u32)> {
        frames.iter().map(|f| (f.path.clone(), f.line)).collect()
    }

    #[test]
    fn go_names_the_function_on_the_line_before_the_file() {
        let trace = "goroutine 1 [running]:\n\
            main.(*Activities).Charge(0xc000010000, {0x1, 0x2})\n\
            \t/home/runner/app/orders/charge.go:41 +0x1d\n\
            go.temporal.io/sdk/internal.(*activityTaskHandlerImpl).Execute(...)\n\
            \t/go/pkg/mod/go.temporal.io/sdk@v1.26.0/internal/internal_task_handlers.go:2008 +0x3c";
        let f = frames(trace);
        assert_eq!(at(&f)[0], ("/home/runner/app/orders/charge.go".into(), 41));
        assert_eq!(f[0].func.as_deref(), Some("main.(*Activities).Charge"));
        assert!(is_library(&f[1]));
    }

    #[test]
    fn java_places_the_file_in_its_package() {
        let trace = "java.lang.IllegalStateException: card declined\n\
            \tat com.acme.orders.Charge.run(Charge.java:41)\n\
            \tat io.temporal.internal.activity.ActivityTaskExecutors.execute(ActivityTaskExecutors.java:88)\n\
            \tat java.base/java.lang.Thread.run(Native Method)";
        let f = frames(trace);
        assert_eq!(at(&f)[0], ("com/acme/orders/Charge.java".into(), 41));
        assert!(is_library(&f[1]));
        assert_eq!(f.len(), 2, "a native frame has no line");
    }

    #[test]
    fn node_reads_both_forms_and_drops_the_file_scheme() {
        let trace = "Error: card declined\n\
            \x20   at charge (/app/src/activities.ts:12:9)\n\
            \x20   at file:///app/lib/run.js:3:1\n\
            \x20   at Activity.execute (/app/node_modules/@temporalio/worker/lib/activity.js:9:2)";
        let f = frames(trace);
        assert_eq!(
            at(&f),
            [
                ("/app/src/activities.ts".into(), 12),
                ("/app/lib/run.js".into(), 3),
                (
                    "/app/node_modules/@temporalio/worker/lib/activity.js".into(),
                    9
                ),
            ]
        );
        assert_eq!(f[0].func.as_deref(), Some("charge"));
        assert!(is_library(&f[2]));
    }

    #[test]
    fn python_lists_the_frame_that_threw_last_so_it_is_turned_around() {
        let trace = "Traceback (most recent call last):\n\
            \x20 File \"/usr/lib/python3.12/site-packages/temporalio/worker/_activity.py\", line 447, in _run_activity\n\
            \x20 File \"/app/orders/activities.py\", line 23, in charge\n\
            ValueError: card declined";
        let f = frames(trace);
        assert_eq!(at(&f)[0], ("/app/orders/activities.py".into(), 23));
        assert_eq!(f[0].func.as_deref(), Some("charge"));
        assert!(is_library(&f[1]));
    }

    #[test]
    fn dotnet_reads_the_path_after_in() {
        let trace = "   at Orders.Charge.RunAsync(Order order) in /src/Orders/Charge.cs:line 41";
        let f = frames(trace);
        assert_eq!(at(&f), [("/src/Orders/Charge.cs".into(), 41)]);
        assert_eq!(f[0].func.as_deref(), Some("Orders.Charge.RunAsync"));
    }

    #[test]
    fn the_longest_shared_tail_wins_and_a_tie_names_nothing() {
        let files: Vec<String> = ["orders/charge.go", "refunds/charge.go", "charge.go"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            by_suffix("/home/runner/app/orders/charge.go", &files),
            Some("orders/charge.go")
        );
        let flat: Vec<String> = ["a/main.go", "b/main.go"].map(String::from).to_vec();
        assert_eq!(by_suffix("/build/main.go", &flat), None);
        assert_eq!(by_suffix("/build/other.go", &files), None);
    }

    #[test]
    fn the_answer_is_the_frame_that_threw_and_the_rest_are_hops() {
        let trace = "main.Charge()\n\t/build/app/orders/charge.go:41 +0x1\n\
                     main.Workflow()\n\t/build/app/orders/workflow.go:76 +0x2\n\
                     go.temporal.io/sdk/internal.run()\n\t/go/pkg/mod/go.temporal.io/sdk@v1/x.go:9 +0x3";
        let files: Vec<String> = ["orders/charge.go", "orders/workflow.go"]
            .map(String::from)
            .to_vec();
        let out = answer(&[trace.into()], &files, Path::new("/repo")).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "/repo/orders/charge.go:41");
        assert_eq!(lines[1], "/repo");
        assert_eq!(
            lines[2],
            "/repo/orders/charge.go:41\t/repo\tmain.Charge, where it threw"
        );
        assert_eq!(
            lines[3],
            "/repo/orders/workflow.go:76\t/repo\tmain.Workflow"
        );
        assert_eq!(lines.len(), 4);

        let one = answer(&[trace.into()], &files[..1], Path::new("/repo")).unwrap();
        assert_eq!(
            one, "/repo/orders/charge.go:41\n/repo\n",
            "one frame needs no picker"
        );
        assert_eq!(answer(&[trace.into()], &[], Path::new("/repo")), None);
    }

    #[test]
    fn every_cause_gives_its_trace_outermost_first() {
        let mut inner = Failure::new("declined");
        inner.stack_trace = Some("inner".into());
        let mut outer = Failure::new("activity failed");
        outer.stack_trace = Some(" ".into());
        outer.cause = Some(Box::new(inner));
        assert_eq!(traces(&outer), ["inner"]);
    }
}
