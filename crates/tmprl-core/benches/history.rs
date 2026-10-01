//! Timings for everything whose cost grows with the length of a workflow history.
//!
//! `cargo bench -p tmprl-core --bench history`, optionally followed by `-- <filter>` to run
//! only the cases whose name contains the filter. `TMPRL_BENCH_SIZES=10000,100000` picks the
//! history lengths.
//!
//! No harness and no dependency: a case is run a few times against `Instant` and the median
//! is printed. That is coarse, and it is enough, because the questions asked here are about
//! orders of magnitude: does a page cost the same at 500,000 events as at 10,000, and does
//! a frame cost the same over an 8 MB payload as over a small one.
//!
//! Nothing here needs a server. The histories are built in code, shaped like a real run:
//! workflow tasks around activities, timers, signals that join the workflow's own group from
//! anywhere, and activities that close several pages after they opened.

use std::hint::black_box;
use std::time::{Duration, Instant};

use tmprl_core::history::{
    Category, Failure, GroupRef, NormalizedEvent, Outcome, Role, group_events,
};
use tmprl_core::outline::{Outline, Row};
use tmprl_core::payload::{FlatCache, Payload, RenderCache};
use tmprl_core::search::{self, Search};

/// `HISTORY_PAGE_SIZE` in the reducer.
const PAGE: usize = 500;

/// Pages appended in a row when timing one.
const PAGES: usize = 20;

/// Lines the payload pane has room for on a tall terminal.
const PANE: usize = 50;

fn main() {
    // `cargo test --all-targets` runs a harness-less bench as a test, without this flag.
    if !std::env::args().any(|a| a == "--bench") {
        println!("history: timings only run under `cargo bench`");
        return;
    }
    let filter: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let wanted = |name: &str| filter.is_empty() || filter.iter().any(|f| name.contains(f));

    let sizes: Vec<usize> = std::env::var("TMPRL_BENCH_SIZES")
        .unwrap_or_else(|_| "10000,100000,500000".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    let mut table = Table::new(sizes.iter().map(|n| format!("{n} events")).collect());
    for (col, &n) in sizes.iter().enumerate() {
        eprintln!("building a history of {n} events");
        let events = history(n + PAGES * PAGE);

        if wanted("page") {
            let (median, slowest) = append_page(&events, n, false);
            table.set("append one 500 event page", col, median);
            table.set("the slowest of 20 such pages", col, slowest);
            let (median, _) = append_page(&events, n, true);
            table.set(
                "append one 500 event page, oldest group unfolded",
                col,
                median,
            );
        }
        if wanted("load") {
            table.set(
                "load the whole history, 500 a page",
                col,
                load_whole(&events[..n]),
            );
        }

        let needs_outline = ["row", "slice", "failure", "search"]
            .iter()
            .any(|c| wanted(c));
        if !needs_outline {
            continue;
        }
        let held = events[..n].to_vec();
        let groups = group_events(&held);
        let mut outline = Outline::new(held, groups);
        if wanted("search") {
            let (cold, warm) = search_outline(&outline);
            table.set("/ over a folded outline, first search", col, cold);
            table.set("/ over a folded outline, repeated", col, warm);
        }
        if wanted("failure") {
            table.set("]f from the last row, folded", col, next_failure(&outline));
        }
        outline.expand_all();
        if wanted("row") {
            table.set("row_at, deep, unfolded", col, row_lookup(&outline));
        }
        if wanted("slice") {
            table.set(
                "slice of 50 rows, deep, unfolded",
                col,
                slice_deep(&outline),
            );
        }
        if wanted("search") {
            let (_, warm) = search_outline(&outline);
            table.set("/ over an unfolded outline, repeated", col, warm);
        }
    }
    table.print();

    if wanted("payload") {
        let sizes = [1usize << 20, 8 << 20];
        let mut table = Table::new(
            sizes
                .iter()
                .map(|b| format!("{} MB of JSON", b >> 20))
                .collect(),
        );
        for (col, &bytes) in sizes.iter().enumerate() {
            let p = big_payload(bytes);
            let (first, frame, seek, lines) = payload_frames(&p);
            table.set("the first frame of the K pane", col, first);
            table.set("each frame after it", col, frame);
            table.set("the frame a search lands on", col, seek);
            table.set("lines in the pane", col, lines.to_string());
        }
        table.print();
    }
}

/// What the `Msg::History` arm of the reducer does when a page lands.
fn page_lands(outline: &mut Option<Outline>, page: Vec<NormalizedEvent>) {
    match outline {
        Some(outline) => {
            outline.append(page);
        }
        None => {
            let groups = group_events(&page);
            *outline = Some(Outline::new(page, groups));
        }
    }
}

/// One page landing on a pane that already holds `n` events.
///
/// `unfolded` opens the workflow's own group first, the oldest there is. The page carries
/// signals, which join that group, so every row below it moves: the one case where a page
/// costs more than what it holds.
///
/// Several pages in a row, reported as the median and the slowest. The slowest is the page
/// on which the vector of events outgrew its allocation and was moved: paid once per
/// doubling, so it does not show in a whole load, but it is the longest any one page takes.
fn append_page(events: &[NormalizedEvent], n: usize, unfolded: bool) -> (String, String) {
    let held = events[..n].to_vec();
    let groups = group_events(&held);
    let mut outline = Outline::new(held, groups);
    if unfolded {
        outline.toggle(0);
    }
    let mut outline = Some(outline);
    let mut runs: Vec<Duration> = events[n..]
        .chunks(PAGE)
        .map(|page| {
            let page = page.to_vec();
            let t = Instant::now();
            page_lands(&mut outline, page);
            t.elapsed()
        })
        .collect();
    black_box(&outline);
    runs.sort();
    (fmt(runs[runs.len() / 2]), fmt(runs[runs.len() - 1]))
}

/// Every page of a history, first to last, the way scrolling to the end reads it.
fn load_whole(events: &[NormalizedEvent]) -> String {
    sample(|| {
        let pages: Vec<Vec<NormalizedEvent>> = events.chunks(PAGE).map(<[_]>::to_vec).collect();
        let mut outline = None;
        let t = Instant::now();
        for page in pages {
            page_lands(&mut outline, page);
        }
        black_box(&outline);
        t.elapsed()
    })
}

fn row_lookup(outline: &Outline) -> String {
    const LOOKUPS: u32 = 1_000_000;
    let len = outline.len();
    let tail = (len / 10).max(1);
    per_op(LOOKUPS, || {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..LOOKUPS {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            // The last tenth of the rows: as far from the top as a reader gets.
            let row = len - 1 - (seed >> 33) as usize % tail;
            black_box(outline.row_at(black_box(row)));
        }
    })
}

fn slice_deep(outline: &Outline) -> String {
    const SLICES: u32 = 20_000;
    let first = outline.len().saturating_sub(60);
    per_op(SLICES, || {
        for _ in 0..SLICES {
            black_box(outline.slice(black_box(first), 50));
        }
    })
}

/// `]f` with nothing left to find, which has every failure in the run to rule out.
fn next_failure(outline: &Outline) -> String {
    let last = outline.len().saturating_sub(1);
    sample(|| {
        let t = Instant::now();
        black_box(outline.next_failure(black_box(last)));
        t.elapsed()
    })
}

/// A `/` that matches nothing, which is the search that reads every row: once cold, then
/// with the flattened payloads already cached, which is what each page of a read-on search
/// pays.
fn search_outline(outline: &Outline) -> (String, String) {
    let pattern = Search::new("no-such-value");
    let mut cache = FlatCache::default();
    let run = |cache: &mut FlatCache| {
        let t = Instant::now();
        let labels = search_labels(outline, cache);
        black_box(search::count(&pattern, &labels));
        black_box(search::find(&pattern, &labels, 0, true, true));
        t.elapsed()
    };
    let cold = fmt(run(&mut cache));
    let warm = sample(|| run(&mut cache));
    (cold, warm)
}

/// `View::search_labels` for a history, which lives in the binary where a bench cannot
/// reach it. Kept line for line the same so the number means what `/` costs.
fn search_labels(outline: &Outline, cache: &mut FlatCache) -> Vec<String> {
    let mut labels: Vec<String> = (0..outline.len())
        .map(|row| history_label(outline, row))
        .collect();
    for (row, label) in labels.iter_mut().enumerate() {
        for e in payload_events(outline, row) {
            for (name, p) in &e.payloads {
                for line in cache.lines(name, p) {
                    label.push(' ');
                    label.push_str(line);
                }
            }
        }
    }
    cache.sweep();
    labels
}

fn history_label(outline: &Outline, row: usize) -> String {
    match outline.row_at(row) {
        Some(Row::Group { group, .. }) => match outline.group(group) {
            Some(g) => {
                let mut s = format!("{} {}", category_label(g.category), g.subject);
                if let Some(f) = &g.failure {
                    s.push(' ');
                    s.push_str(&f.headline());
                }
                s
            }
            None => String::new(),
        },
        Some(Row::Event { event, .. }) => match outline.event(event) {
            Some(e) => {
                let mut s = format!("{} {}", e.id, e.name);
                for (k, v) in &e.fields {
                    s.push_str(&format!(" {k}={v}"));
                }
                if let Some(f) = &e.failure {
                    s.push(' ');
                    s.push_str(&f.headline());
                }
                s
            }
            None => String::new(),
        },
        None => String::new(),
    }
}

fn payload_events(outline: &Outline, row: usize) -> Vec<&NormalizedEvent> {
    match outline.row_at(row) {
        Some(Row::Group { group, .. }) => {
            let events = outline.events();
            outline
                .group(group)
                .map(|g| {
                    g.payload_ends()
                        .into_iter()
                        .filter_map(|id| {
                            events
                                .binary_search_by_key(&id, |e| e.id)
                                .ok()
                                .map(|i| &events[i])
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        Some(Row::Event { event, .. }) => outline.event(event).into_iter().collect(),
        None => Vec::new(),
    }
}

fn category_label(c: Category) -> &'static str {
    match c {
        Category::Workflow => "workflow",
        Category::WorkflowTask => "task",
        Category::Activity => "activity",
        Category::Timer => "timer",
        Category::ChildWorkflow => "child",
        Category::ExternalWorkflow => "external",
        Category::Update => "update",
        Category::Nexus => "nexus",
        Category::Marker => "marker",
        Category::SearchAttributes => "attrs",
    }
}

/// What the payload pane does with one payload: on the frame it first draws it, on every
/// frame after that, and on the frame after a `/` landed, when it looks through every line
/// for the match to scroll to. The window is cut from near the end of the payload.
fn payload_frames(p: &Payload) -> (String, String, String, usize) {
    let window = |cache: &mut RenderCache| {
        let mut drawn: Vec<String> = Vec::with_capacity(PANE);
        if let Some(lines) = cache.text(p) {
            let first = lines.len().saturating_sub(PANE * 2);
            drawn.extend(
                (first..first + PANE).filter_map(|i| Some(format!("    {}", lines.get(i)?))),
            );
        }
        cache.sweep();
        drawn
    };

    let first = sample(|| {
        let mut cache = RenderCache::default();
        let t = Instant::now();
        black_box(window(&mut cache));
        t.elapsed()
    });

    let mut cache = RenderCache::default();
    let frame = sample(|| {
        let t = Instant::now();
        black_box(window(&mut cache));
        t.elapsed()
    });

    let pattern = Search::new("no-such-value");
    let seek = sample(|| {
        let t = Instant::now();
        if let Some(lines) = cache.text(p) {
            black_box(
                lines
                    .iter()
                    .position(|l| pattern.matches(&format!("    {l}"))),
            );
        }
        black_box(window(&mut cache));
        t.elapsed()
    });

    let count = cache.text(p).map_or(0, |lines| lines.len());
    (first, frame, seek, count)
}

/// A JSON document of about `bytes` bytes: an array of orders, the kind of value a batch
/// activity is handed.
fn big_payload(bytes: usize) -> Payload {
    let mut body = String::with_capacity(bytes + 512);
    body.push('[');
    let mut i = 0u64;
    while body.len() < bytes {
        if i > 0 {
            body.push(',');
        }
        body.push_str(&format!(
            r#"{{"orderId":"ord-{i:08}","customer":{{"id":"cus-{c:06}","tier":"gold","email":"buyer{c}@example.com"}},"items":[{{"sku":"SKU-{s:05}","qty":2,"price":19.99}},{{"sku":"SKU-{t:05}","qty":1,"price":249.5}}],"total":289.48,"approved":true,"note":"left with the neighbour at number {i}"}}"#,
            c = i % 4096,
            s = i % 977,
            t = i % 613,
        ));
        i += 1;
    }
    body.push(']');
    Payload::new("json/plain", body.into_bytes())
}

/// A history of exactly `n` events.
///
/// Each turn is a workflow task and an activity. Around those: a timer every fourth turn, a
/// signal every sixteenth (it joins the workflow's own group, the oldest one there is), and
/// every fiftieth turn an activity that stays open for a few thousand events, so that pages
/// routinely close groups opened several pages back. One activity in ninety-seven fails.
fn history(n: usize) -> Vec<NormalizedEvent> {
    const TYPES: [&str; 6] = [
        "ChargeCard",
        "ReserveStock",
        "SendReceipt",
        "BookCourier",
        "ScoreFraud",
        "SettleLedger",
    ];
    let json = |body: String| Payload::new("json/plain", body.into_bytes());
    let at = |id: i64| Some(1_700_000_000_000 + id * 37);
    let ev = |id: i64, name: &'static str, category, group, role| {
        NormalizedEvent::new(id, name, category, group, role).with_time(at(id))
    };

    let mut out: Vec<NormalizedEvent> = Vec::with_capacity(n + 16);
    let mut started = ev(
        1,
        "WorkflowExecutionStarted",
        Category::Workflow,
        GroupRef::Workflow,
        Role::Opens,
    )
    .with_subject("OrderWorkflow");
    started
        .payloads
        .push(("input".into(), json(r#"{"batch":"2026-10-02"}"#.into())));
    out.push(started);

    // Activities still open, oldest first: the id that opened each and when it closes.
    let mut slow: std::collections::VecDeque<(i64, i64)> = std::collections::VecDeque::new();
    let mut id = 2i64;
    let mut turn = 0i64;
    while out.len() < n {
        turn += 1;

        let task = GroupRef::Opened(id);
        for (name, role, outcome) in [
            ("WorkflowTaskScheduled", Role::Opens, Outcome::Pending),
            ("WorkflowTaskStarted", Role::Continues, Outcome::Pending),
            ("WorkflowTaskCompleted", Role::Closes, Outcome::Completed),
        ] {
            out.push(ev(id, name, Category::WorkflowTask, task, role).with_outcome(outcome));
            id += 1;
        }

        while slow.front().is_some_and(|(_, due)| *due <= id) {
            let (opened, _) = slow.pop_front().unwrap_or_default();
            let mut done = ev(
                id,
                "ActivityTaskCompleted",
                Category::Activity,
                GroupRef::Opened(opened),
                Role::Closes,
            )
            .with_outcome(Outcome::Completed);
            done.payloads
                .push(("result".into(), json(format!(r#"{{"waited":{opened}}}"#))));
            out.push(done);
            id += 1;
        }

        let activity = GroupRef::Opened(id);
        let mut scheduled = ev(
            id,
            "ActivityTaskScheduled",
            Category::Activity,
            activity,
            Role::Opens,
        )
        .with_subject(TYPES[(turn % 6) as usize]);
        scheduled.fields.push(("activityId", turn.to_string()));
        scheduled.fields.push(("taskQueue", "orders".into()));
        scheduled.payloads.push((
            "input".into(),
            json(format!(
                r#"{{"orderId":"ord-{turn}","amount":{},"customer":{{"id":"cus-{}","tier":"gold"}}}}"#,
                turn * 7 % 9000,
                turn % 4096
            )),
        ));
        out.push(scheduled);
        id += 1;

        if turn % 50 == 0 {
            slow.push_back((id - 1, id + 2300));
        } else {
            let mut picked = ev(
                id,
                "ActivityTaskStarted",
                Category::Activity,
                activity,
                Role::Continues,
            );
            picked.attempt = Some(1);
            out.push(picked);
            id += 1;

            let mut closed = ev(
                id,
                "ActivityTaskCompleted",
                Category::Activity,
                activity,
                Role::Closes,
            );
            if turn % 97 == 0 {
                closed.name = "ActivityTaskFailed";
                closed.outcome = Outcome::Failed;
                closed.failure = Some(Failure {
                    kind: Some("PaymentDeclined".into()),
                    ..Failure::new("card declined")
                });
            } else {
                closed.outcome = Outcome::Completed;
                closed.payloads.push((
                    "result".into(),
                    json(format!(r#"{{"status":"APPROVED","ref":"r-{turn}"}}"#)),
                ));
            }
            out.push(closed);
            id += 1;
        }

        if turn % 4 == 0 {
            let timer = GroupRef::Opened(id);
            out.push(
                ev(id, "TimerStarted", Category::Timer, timer, Role::Opens)
                    .with_subject(format!("timer-{turn}")),
            );
            id += 1;
            out.push(
                ev(id, "TimerFired", Category::Timer, timer, Role::Closes)
                    .with_outcome(Outcome::Completed),
            );
            id += 1;
        }

        if turn % 16 == 0 {
            let mut signal = ev(
                id,
                "WorkflowExecutionSignaled",
                Category::Workflow,
                GroupRef::Workflow,
                Role::Continues,
            );
            signal
                .payloads
                .push(("input".into(), json(format!(r#"{{"nudge":{turn}}}"#))));
            out.push(signal);
            id += 1;
        }
    }
    out.truncate(n);
    out
}

/// Run a case until it has been timed a few times or has used its second, and report the
/// median. A case that takes longer than that on its first run is reported from that run.
fn sample(mut once: impl FnMut() -> Duration) -> String {
    const BUDGET: Duration = Duration::from_secs(1);
    let mut runs = Vec::new();
    let began = Instant::now();
    loop {
        runs.push(once());
        if runs.len() >= 50 || (runs.len() >= 3 && began.elapsed() >= BUDGET) {
            break;
        }
        if runs.len() < 3 && began.elapsed() >= BUDGET * 5 {
            break;
        }
    }
    runs.sort();
    fmt(runs[runs.len() / 2])
}

/// The median of a few runs of a loop of `ops` operations, per operation.
fn per_op(ops: u32, mut run: impl FnMut()) -> String {
    let mut runs: Vec<Duration> = (0..5)
        .map(|_| {
            let t = Instant::now();
            run();
            t.elapsed()
        })
        .collect();
    runs.sort();
    fmt(runs[runs.len() / 2] / ops)
}

fn fmt(d: Duration) -> String {
    let ns = d.as_nanos() as f64;
    if ns < 1e3 {
        format!("{ns:.0} ns")
    } else if ns < 1e6 {
        format!("{:.1} µs", ns / 1e3)
    } else if ns < 1e9 {
        format!("{:.1} ms", ns / 1e6)
    } else {
        format!("{:.2} s", ns / 1e9)
    }
}

/// Cases down the side, sizes across, printed as a markdown table.
struct Table {
    columns: Vec<String>,
    rows: Vec<(&'static str, Vec<String>)>,
}

impl Table {
    fn new(columns: Vec<String>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
        }
    }

    fn set(&mut self, case: &'static str, col: usize, cell: String) {
        let at = match self.rows.iter().position(|(name, _)| *name == case) {
            Some(at) => at,
            None => {
                self.rows
                    .push((case, vec![String::new(); self.columns.len()]));
                self.rows.len() - 1
            }
        };
        self.rows[at].1[col] = cell;
    }

    fn print(&self) {
        if self.rows.is_empty() {
            return;
        }
        println!();
        println!("| | {} |", self.columns.join(" | "));
        println!("|---|{}", "---:|".repeat(self.columns.len()));
        for (case, cells) in &self.rows {
            println!("| {case} | {} |", cells.join(" | "));
        }
    }
}
