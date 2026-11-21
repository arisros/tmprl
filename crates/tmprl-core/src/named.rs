//! gf by name: where an activity or workflow's registered name is defined, found in the source text.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Defines,
    Registers,
    Names,
    Uses,
}

#[derive(Debug)]
struct Hit {
    source: usize,
    line: u32,
    role: Role,
    label: String,
}

const EXTENSIONS: [&str; 10] = [
    "go", "ts", "tsx", "js", "mjs", "cjs", "py", "java", "kt", "cs",
];

const LIBRARY: [&str; 5] = [
    "vendor/",
    "node_modules/",
    "site-packages/",
    "dist-packages/",
    "dist/",
];

pub fn is_source(file: &str) -> bool {
    let ext = file.rsplit('.').next().unwrap_or("");
    EXTENSIONS.contains(&ext)
        && !file.ends_with(".d.ts")
        && !file.ends_with(".min.js")
        && !LIBRARY
            .iter()
            .any(|l| file.starts_with(l) || file.contains(&format!("/{l}")))
}

fn is_test(file: &str) -> bool {
    file.ends_with("_test.go")
        || file.contains(".test.")
        || file.contains(".spec.")
        || file.contains("/test/")
        || file.contains("/tests/")
        || file.contains("/__tests__/")
        || file
            .rsplit('/')
            .next()
            .is_some_and(|n| n.starts_with("test_"))
}

fn is_comment(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with("//") || l.starts_with('#') || l.starts_with("/*") || l.starts_with('*')
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

pub fn variants(name: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for part in name.split(|c: char| !c.is_alphanumeric()) {
        let mut word = String::new();
        let mut prev_lower = false;
        for c in part.chars() {
            if c.is_uppercase() && prev_lower && !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
            word.extend(c.to_lowercase());
        }
        if !word.is_empty() {
            words.push(word);
        }
    }
    let cap = |w: &str| {
        let mut c = w.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect::<String>())
            .unwrap_or_default()
    };
    let pascal: String = words.iter().map(|w| cap(w)).collect();
    let camel: String = words
        .iter()
        .enumerate()
        .map(|(i, w)| if i == 0 { w.clone() } else { cap(w) })
        .collect();
    let snake = words.join("_");
    let mut out = Vec::new();
    for v in [name.to_string(), pascal, camel, snake] {
        if !v.is_empty() && v.chars().all(is_ident) && !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

fn word_at(line: &str, word: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = line[from..].find(word).map(|i| i + from) {
        let before = line[..i].chars().next_back();
        let after = line[i + word.len()..].chars().next();
        if !before.is_some_and(is_ident) && !after.is_some_and(is_ident) {
            out.push(i);
        }
        from = i + word.len();
    }
    out
}

fn quoted_at(line: &str, name: &str) -> Option<usize> {
    ['"', '\'', '`']
        .iter()
        .find_map(|q| line.find(&format!("{q}{name}{q}")))
}

fn bound_ident(line: &str, at: usize) -> Option<String> {
    let before = line[..at].trim_end();
    let before = if let Some(b) = before.strip_suffix(":=") {
        b
    } else {
        let b = before.strip_suffix('=')?;
        if b.ends_with(['=', '!', '<', '>']) {
            return None;
        }
        b
    };
    let before = before.trim_end();
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident(*c))
        .last()
        .map(|(i, _)| i)?;
    let ident = &before[start..];
    let rest = before[..start].trim_end();
    if rest.ends_with(['(', ',']) || ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(ident.to_string())
}

fn ident_path_at(text: &str) -> Option<String> {
    let text = text.trim_start().trim_start_matches('&');
    let path: String = text
        .chars()
        .take_while(|c| is_ident(*c) || *c == '.')
        .collect();
    let last = path.rsplit('.').next()?.to_string();
    (!last.is_empty() && !last.chars().next().is_some_and(|c| c.is_ascii_digit())).then_some(last)
}

fn registered_fn(line: &str, key_end: usize) -> Option<String> {
    let rest = line[key_end..].trim_start();
    if let Some(value) = rest.strip_prefix(':').filter(|r| !r.starts_with(':'))
        && let Some(f) = ident_path_at(value)
    {
        return Some(f);
    }
    let lower = line.to_ascii_lowercase();
    let at = lower.find("register")?;
    let open = line[at..].find('(')? + at + 1;
    ident_path_at(&line[open..])
}

const MODIFIERS: [&str; 14] = [
    "public ",
    "private ",
    "protected ",
    "internal ",
    "static ",
    "override ",
    "async ",
    "final ",
    "suspend ",
    "fun ",
    "func ",
    "def ",
    "function ",
    "export ",
];

fn declares(line: &str, name: &str) -> bool {
    let trimmed = line.trim_start();
    word_at(trimmed, name).into_iter().any(|i| {
        let prefix = trimmed[..i].trim_end();
        if prefix.ends_with('.') {
            return false;
        }
        let after = trimmed[i + name.len()..].trim_start();
        let first = prefix.split_whitespace().next().unwrap_or("");
        if after.starts_with('(') || after.starts_with('<') {
            if prefix.is_empty() || prefix == "async" {
                return trimmed.trim_end().ends_with('{');
            }
            return MODIFIERS.iter().any(|m| trimmed.starts_with(m))
                || first == "func"
                || prefix.ends_with("def")
                || prefix.ends_with("function");
        }
        let binding = ["const", "let", "var"].contains(&prefix.rsplit(' ').next().unwrap_or(""));
        binding
            && after.strip_prefix('=').is_some_and(|v| {
                let v = v.trim_start();
                v.starts_with('(') || v.starts_with("async") || v.starts_with("function")
            })
    })
}

fn fn_name_of(line: &str) -> Option<String> {
    let open = line.find('(')?;
    let before = line[..open].trim_end();
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident(*c))
        .last()
        .map(|(i, _)| i)?;
    Some(before[start..].to_string())
}

fn decorated(lines: &[&str], at: usize) -> Option<(usize, String)> {
    let line = lines[at].trim_start();
    if !line.starts_with('@') && !line.starts_with('[') {
        return None;
    }
    let (i, next) = lines
        .iter()
        .enumerate()
        .skip(at + 1)
        .take(4)
        .find(|(_, l)| {
            let l = l.trim_start();
            !l.is_empty() && !l.starts_with('@') && !l.starts_with('[')
        })?;
    Some((i, fn_name_of(next)?))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub path: String,
    pub root: String,
    pub line: u32,
    pub role: Role,
    pub label: String,
}

#[derive(Debug, Default)]
pub struct Found {
    pub places: Vec<Place>,
    pub fns: Vec<String>,
    pub idents: Vec<String>,
}

pub fn answer(name: &str, sources: &[(&Path, &str, &str)]) -> Option<String> {
    write(search(name, sources).places)
}

pub fn write(mut places: Vec<Place>) -> Option<String> {
    const MOST_HOPS: usize = 40;
    places.sort_by(|a, b| {
        (a.role, is_test(&a.path), &a.path, a.line).cmp(&(
            b.role,
            is_test(&b.path),
            &b.path,
            b.line,
        ))
    });
    let mut seen = std::collections::HashSet::new();
    places.retain(|p| seen.insert((p.path.clone(), p.line)));
    let first = places.first()?;
    let mut out = format!("{}:{}\n{}\n", first.path, first.line, first.root);
    if places.len() > 1 {
        for p in places.iter().take(MOST_HOPS) {
            out.push_str(&format!("{}:{}\t{}\t{}\n", p.path, p.line, p.root, p.label));
        }
    }
    Some(out)
}

pub fn search(name: &str, sources: &[(&Path, &str, &str)]) -> Found {
    let name = name.trim();
    if name.is_empty() {
        return Found::default();
    }
    let texts: Vec<Vec<&str>> = sources
        .iter()
        .map(|(_, _, t)| t.lines().collect())
        .collect();
    let mut hits: Vec<Hit> = Vec::new();
    let mut idents: Vec<String> = Vec::new();
    let mut fns: Vec<String> = variants(name);
    fn push(hits: &mut Vec<Hit>, source: usize, line: usize, role: Role, label: String) {
        let line = line as u32 + 1;
        match hits
            .iter_mut()
            .find(|h| h.source == source && h.line == line)
        {
            Some(h) if role < h.role => {
                h.role = role;
                h.label = label;
            }
            Some(_) => {}
            None => hits.push(Hit {
                source,
                line,
                role,
                label,
            }),
        }
    }

    for (s, lines) in texts.iter().enumerate() {
        for (n, line) in lines.iter().enumerate() {
            if is_comment(line) {
                continue;
            }
            let Some(at) = quoted_at(line, name) else {
                continue;
            };
            if let Some(id) = bound_ident(line, at) {
                push(&mut hits, s, n, Role::Names, format!("`{id}` names it"));
                if !idents.contains(&id) {
                    idents.push(id);
                }
                continue;
            }
            push(
                &mut hits,
                s,
                n,
                Role::Registers,
                format!("registers `{name}`"),
            );
            if let Some(f) = registered_fn(line, at + name.len() + 2) {
                fns.push(f);
            }
            if let Some((i, f)) = decorated(lines, n) {
                push(&mut hits, s, i, Role::Defines, format!("defines `{f}`"));
            }
        }
    }

    for (s, lines) in texts.iter().enumerate() {
        for (n, line) in lines.iter().enumerate() {
            if is_comment(line) {
                continue;
            }
            for id in &idents {
                let Some(&at) = word_at(line, id).first() else {
                    continue;
                };
                if quoted_at(line, name).is_some() {
                    continue;
                }
                match registered_fn(line, at + id.len()) {
                    Some(f) => {
                        push(
                            &mut hits,
                            s,
                            n,
                            Role::Registers,
                            format!("registers `{id}`"),
                        );
                        fns.push(f);
                    }
                    None => push(&mut hits, s, n, Role::Uses, format!("uses `{id}`")),
                }
                if let Some((i, f)) = decorated(lines, n) {
                    push(&mut hits, s, i, Role::Defines, format!("defines `{f}`"));
                }
            }
        }
    }

    let mut seen = std::collections::HashSet::new();
    fns.retain(|f| seen.insert(f.clone()));
    for (s, lines) in texts.iter().enumerate() {
        for (n, line) in lines.iter().enumerate() {
            if let Some(f) = fns.iter().find(|f| declares(line, f)) {
                push(&mut hits, s, n, Role::Defines, format!("defines `{f}`"));
            }
        }
    }

    let places = hits
        .into_iter()
        .map(|h| {
            let (root, file, _) = sources[h.source];
            Place {
                path: root.join(file).display().to_string(),
                root: root.display().to_string(),
                line: h.line,
                role: h.role,
                label: h.label,
            }
        })
        .collect();
    Found {
        places,
        fns,
        idents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(out: &str) -> Vec<&str> {
        out.lines().collect()
    }

    #[test]
    fn a_name_is_spelled_the_ways_code_spells_it() {
        assert_eq!(
            variants("calculate_duties"),
            ["calculate_duties", "CalculateDuties", "calculateDuties"]
        );
        assert_eq!(
            variants("ChargeCard"),
            ["ChargeCard", "chargeCard", "charge_card"]
        );
        assert_eq!(
            variants("shipment-v3"),
            ["ShipmentV3", "shipmentV3", "shipment_v3"]
        );
    }

    #[test]
    fn a_go_const_leads_through_its_registration_to_the_function() {
        let root = Path::new("/repo");
        let names = "const (\n\tCalculateDutiesName = \"calculate_duties\"\n)\n";
        let register = "var all = map[string]any{\n\tCalculateDutiesName: Duties,\n}\n";
        let code = "package x\n\nfunc Duties(ctx context.Context) error {\n\treturn nil\n}\n";
        let call = "\tworkflow.ExecuteActivity(ctx, CalculateDutiesName)\n";
        let out = answer(
            "calculate_duties",
            &[
                (root, "names.go", names),
                (root, "register.go", register),
                (root, "duties.go", code),
                (root, "workflow.go", call),
            ],
        )
        .unwrap();
        let l = lines(&out);
        assert_eq!(l[0], "/repo/duties.go:3");
        assert_eq!(l[1], "/repo");
        assert_eq!(l[2], "/repo/duties.go:3\t/repo\tdefines `Duties`");
        assert_eq!(
            l[3],
            "/repo/register.go:2\t/repo\tregisters `CalculateDutiesName`"
        );
        assert_eq!(
            l[4],
            "/repo/names.go:2\t/repo\t`CalculateDutiesName` names it"
        );
        assert_eq!(
            l[5],
            "/repo/workflow.go:1\t/repo\tuses `CalculateDutiesName`"
        );
    }

    #[test]
    fn registration_options_name_the_function_they_register() {
        let root = Path::new("/r");
        let reg = "w.RegisterActivityWithOptions(acts.Charge, activity.RegisterOptions{Name: \"charge-card\"})\n";
        let code = "func (a *Activities) Charge(ctx context.Context) error {\n";
        let out = answer(
            "charge-card",
            &[(root, "main.go", reg), (root, "acts.go", code)],
        )
        .unwrap();
        assert_eq!(lines(&out)[0], "/r/acts.go:1");
    }

    #[test]
    fn a_decorator_or_an_attribute_names_the_function_under_it() {
        let root = Path::new("/r");
        let py = "@activity.defn(name=\"compose_greeting\")\nasync def greet(input):\n    pass\n";
        assert_eq!(
            lines(&answer("compose_greeting", &[(root, "acts.py", py)]).unwrap())[0],
            "/r/acts.py:2"
        );
        let cs = "    [Activity(\"SendEmail\")]\n    public async Task Mail(string to)\n";
        assert_eq!(
            lines(&answer("SendEmail", &[(root, "Acts.cs", cs)]).unwrap())[0],
            "/r/Acts.cs:2"
        );
    }

    #[test]
    fn a_function_spelled_like_the_name_defines_it_in_any_language() {
        let root = Path::new("/r");
        for (file, text) in [
            ("a.go", "func ChargeCard(ctx context.Context) error {"),
            ("a.py", "async def charge_card(amount):"),
            ("a.ts", "export async function chargeCard(amount: number) {"),
            (
                "b.ts",
                "export const chargeCard = async (amount: number) => {",
            ),
            ("A.java", "    public String chargeCard(int amount) {"),
            (
                "C.ts",
                "  async chargeCard(amount: number): Promise<void> {",
            ),
        ] {
            let out = answer("ChargeCard", &[(root, file, text)]);
            assert_eq!(
                out.as_deref(),
                Some(format!("/r/{file}:1\n/r\n").as_str()),
                "{file}"
            );
        }
        let call = "\treturn chargeCard(1)\n\tx.ChargeCard(ctx)\n";
        assert_eq!(
            answer("ChargeCard", &[(root, "c.go", call)]),
            None,
            "calls define nothing"
        );
        let said = "// `ChargeCard` and \"ChargeCard\" are its names\n# 'ChargeCard'\n";
        assert_eq!(
            answer("ChargeCard", &[(root, "c.go", said)]),
            None,
            "comments say nothing"
        );
    }

    #[test]
    fn tests_come_after_the_code_and_libraries_are_not_read() {
        let root = Path::new("/r");
        let code = "func Ship() {\n";
        let out = answer(
            "Ship",
            &[(root, "ship_test.go", code), (root, "ship.go", code)],
        )
        .unwrap();
        assert_eq!(lines(&out)[0], "/r/ship.go:1");
        assert!(is_source("svc/ship.go"));
        assert!(!is_source("vendor/x/ship.go"));
        assert!(!is_source("web/node_modules/x/a.js"));
        assert!(!is_source("README.md"));
        assert!(!is_source("types/a.d.ts"));
    }

    #[test]
    fn a_keyword_argument_or_a_field_binds_no_identifier() {
        let bound = |line: &str| bound_ident(line, line.find('"').unwrap());
        assert_eq!(bound("X = \"a\"").as_deref(), Some("X"));
        assert_eq!(bound("\tX := \"a\"").as_deref(), Some("X"));
        assert_eq!(
            bound("public static final String NAME = \"a\";").as_deref(),
            Some("NAME")
        );
        assert_eq!(bound("defn(name=\"a\")"), None);
        assert_eq!(bound("{Name: \"a\"}"), None);
        assert_eq!(bound("if x == \"a\""), None);
    }
}
