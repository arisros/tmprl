//! What a release promises not to break: every command id, default binding and flag.
//!
//! `docs/public-api.txt` is the promise written down. The test below fails when the code
//! drifts from it, so a rename is a decision someone made rather than one that slipped in.
//! `TMPRL_UPDATE_API=1 cargo test -p tmprl public_api` rewrites it.

#[cfg(test)]
mod tests {
    use tmprl_core::command::Registry;
    use tmprl_core::keymap::default_keymap;

    fn listing() -> String {
        let mut out = String::from("# Commands: id, group, title\n");
        for c in Registry::builtin().all() {
            out.push_str(&format!("{}\t{}\t{}\n", c.id, c.group, c.title));
        }
        out.push_str("\n# Default bindings: mode, keys, command\n");
        let mut bindings: Vec<String> = default_keymap()
            .bindings()
            .iter()
            .map(|b| format!("{}\t{}\t{}", b.mode, b.seq, b.command))
            .collect();
        bindings.sort();
        for b in bindings {
            out.push_str(&b);
            out.push('\n');
        }
        out.push_str("\n# Flags\n");
        for line in crate::cli::USAGE.lines() {
            let flag = line.trim_start();
            if flag.starts_with('-') {
                out.push_str(flag.split("  ").next().unwrap_or(flag).trim_end());
                out.push('\n');
            }
        }
        out
    }

    #[test]
    fn the_public_api_is_what_docs_public_api_txt_says() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/public-api.txt");
        let now = listing();
        if std::env::var_os("TMPRL_UPDATE_API").is_some() {
            std::fs::write(path, &now).unwrap();
            return;
        }
        let promised = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            promised == now,
            "the public API changed. If that is intended, rerun with TMPRL_UPDATE_API=1 \
             and say so in CHANGELOG.md; a rename or removal after 1.0 is a major release."
        );
    }
}
