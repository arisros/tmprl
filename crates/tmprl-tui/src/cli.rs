//! The command line: arguments parsed by hand into what `main` should do.
//!
//! Parsing is a pure function over the arguments, so it prints nothing and exits nowhere;
//! `main` does both. The flags are public API (see `docs/RELEASING.md`), which is why every
//! one of them is pinned by a test here.

use tmprl_client::ProfileRef;

use crate::app::Startup;

pub const USAGE: &str = "\
tmprl, a terminal client for Temporal

USAGE:
    tmprl [OPTIONS]

OPTIONS:
    -p, --profile <NAME>        Profile from ~/.config/temporalio/temporal.toml
        --temporal-config <P>   Override that file's path (alias: --config)
        --address <HOST:PORT>   Override the profile's server address; its TLS and API
                                key still apply
    -n, --namespace <NAME>      Open that namespace's workflows; overrides the profile's
    -q, --query <QUERY>         Open the workflow list with this visibility query applied
    -w, --workflow <ID>         Open the history of this workflow id or run id
        --readonly              Refuse every mutation for this run
        --config-path           Print where tmprl reads its own config, and exit
    -h, --help                  Print this message
    -V, --version               Print version

A long option also takes its value as --option=value. With no -n, -q or -w tmprl opens
on the namespace list; -q and -w use the profile's namespace unless -n names another.

Connection settings come from the same files and TEMPORAL_* variables the
`temporal` CLI uses. tmprl's own config (config.toml, keys.toml, views.toml)
is a different directory; --config-path prints it. Press ? inside the
application for keybindings.
";

/// What the arguments ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cli {
    Run {
        profile: ProfileRef,
        startup: Startup,
    },
    Help,
    Version,
    /// `--config-path`, with the `--temporal-config` given before it, if one was.
    ConfigPath(Option<String>),
}

/// Parse everything after the program name.
///
/// `-h`, `-V` and `--config-path` answer as soon as they are reached, as they always have:
/// asking for help must work even when the rest of the line is wrong, and `--config-path`
/// reflects a `--temporal-config` only when that came first.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut profile = ProfileRef::default();
    let mut startup = Startup::default();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_string(), Some(value.to_string()))
            }
            _ => (arg, None),
        };
        let mut value = |name: &str| value_of(name, inline.clone(), &mut args);

        match flag.as_str() {
            "-p" | "--profile" => profile.name = Some(value("--profile")?),
            // `--config` predates tmprl having a config of its own and names the
            // *Temporal* profile file. Kept as an alias so existing wrappers keep working.
            "--temporal-config" | "--config" => {
                profile.config_file = Some(value("--temporal-config")?);
            }
            "-n" | "--namespace" => {
                let namespace = value("--namespace")?;
                // Both, deliberately: the connection is bound to it, and the app opens on it.
                profile.namespace = Some(namespace.clone());
                startup.namespace = Some(namespace);
            }
            "-q" | "--query" => startup.query = Some(value("--query")?),
            "-w" | "--workflow" => {
                let id = value("--workflow")?;
                // Refused here rather than after connecting: the lookup puts the id inside
                // a quoted literal, and Temporal's grammar has no escape for a quote.
                if tmprl_core::query::by_id(&id).is_none() {
                    return Err(format!(
                        "--workflow cannot look up `{id}`: it has a quote in it"
                    ));
                }
                startup.workflow = Some(id);
            }
            "--address" => profile.address = Some(value("--address")?),
            "--help" | "--version" | "--readonly" | "--config-path" if inline.is_some() => {
                return Err(format!("{flag} takes no value"));
            }
            "--readonly" => startup.readonly = true,
            "-h" | "--help" => return Ok(Cli::Help),
            "-V" | "--version" => return Ok(Cli::Version),
            "--config-path" => return Ok(Cli::ConfigPath(profile.config_file)),
            _ => {
                let given = match inline {
                    Some(value) => format!("{flag}={value}"),
                    None => flag,
                };
                return Err(format!("unknown argument `{given}`\n\n{USAGE}"));
            }
        }
    }
    Ok(Cli::Run { profile, startup })
}

/// The value an option was given, from `--option=value` or from the next argument.
///
/// A next argument that looks like another option is not taken as the value. Without that,
/// `tmprl -n --readonly` would start writable in a namespace called `--readonly`, which is
/// the one mistake a read-only flag must not make possible. `--option=value` remains for a
/// value that really does begin with a dash.
fn value_of(
    name: &str,
    inline: Option<String>,
    rest: &mut impl Iterator<Item = String>,
) -> Result<String, String> {
    let value = match inline {
        Some(value) => value,
        None => match rest.next() {
            Some(next) if next.len() > 1 && next.starts_with('-') => {
                return Err(format!(
                    "{name} needs a value, and `{next}` looks like another option \
                     (write {name}={next} if it is the value)"
                ));
            }
            Some(next) => next,
            None => return Err(format!("{name} needs a value")),
        },
    };
    if value.trim().is_empty() {
        return Err(format!("{name} needs a value"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_line(line: &[&str]) -> Result<Cli, String> {
        parse(line.iter().map(|s| s.to_string()))
    }

    fn run(line: &[&str]) -> (ProfileRef, Startup) {
        match parse_line(line) {
            Ok(Cli::Run { profile, startup }) => (profile, startup),
            other => panic!("{line:?} should run, got {other:?}"),
        }
    }

    fn refusal(line: &[&str]) -> String {
        match parse_line(line) {
            Err(e) => e,
            other => panic!("{line:?} should be refused, got {other:?}"),
        }
    }

    #[test]
    fn no_arguments_is_the_default_profile_on_the_namespace_list() {
        let (profile, startup) = run(&[]);
        assert_eq!(profile, ProfileRef::default());
        assert_eq!(startup, Startup::default());
        assert!(!startup.opens_workflows());
    }

    #[test]
    fn the_flags_that_predate_this_file_still_parse() {
        // Wrappers and muscle memory depend on these exact spellings.
        let (profile, _) = run(&["-p", "prod", "--config", "/tmp/t.toml"]);
        assert_eq!(profile.name.as_deref(), Some("prod"));
        assert_eq!(profile.config_file.as_deref(), Some("/tmp/t.toml"));

        let (profile, _) = run(&["--profile", "sit", "--temporal-config", "/tmp/u.toml"]);
        assert_eq!(profile.name.as_deref(), Some("sit"));
        assert_eq!(profile.config_file.as_deref(), Some("/tmp/u.toml"));
    }

    #[test]
    fn a_namespace_binds_the_connection_and_opens_the_workflow_list() {
        for flag in ["-n", "--namespace"] {
            let (profile, startup) = run(&[flag, "orders"]);
            assert_eq!(profile.namespace.as_deref(), Some("orders"));
            assert_eq!(startup.namespace.as_deref(), Some("orders"));
            assert!(startup.opens_workflows());
        }
    }

    #[test]
    fn a_query_alone_opens_the_workflow_list_in_the_profiles_namespace() {
        let query = "ExecutionStatus = 'Failed'";
        for flag in ["-q", "--query"] {
            let (profile, startup) = run(&[flag, query]);
            assert_eq!(startup.query.as_deref(), Some(query));
            assert_eq!(profile.namespace, None, "the profile's namespace stands");
            assert!(startup.opens_workflows());
        }
    }

    #[test]
    fn a_workflow_can_be_combined_with_a_namespace() {
        let (profile, startup) = run(&["-n", "orders", "-w", "order-42"]);
        assert_eq!(profile.namespace.as_deref(), Some("orders"));
        assert_eq!(startup.workflow.as_deref(), Some("order-42"));

        let (_, same) = run(&["--workflow", "order-42", "--namespace", "orders"]);
        assert_eq!(same, startup, "the order of the flags does not matter");
    }

    #[test]
    fn every_startup_flag_together() {
        let (profile, startup) = run(&[
            "-p",
            "prod",
            "-n",
            "orders",
            "-q",
            "WorkflowType = 'Checkout'",
            "-w",
            "order-42",
            "--address",
            "localhost:7233",
            "--readonly",
        ]);
        assert_eq!(
            profile,
            ProfileRef {
                name: Some("prod".into()),
                config_file: None,
                address: Some("localhost:7233".into()),
                namespace: Some("orders".into()),
            }
        );
        assert_eq!(
            startup,
            Startup {
                namespace: Some("orders".into()),
                query: Some("WorkflowType = 'Checkout'".into()),
                workflow: Some("order-42".into()),
                readonly: true,
            }
        );
    }

    #[test]
    fn an_address_changes_the_connection_and_not_where_the_app_opens() {
        let (profile, startup) = run(&["--address", "temporal.internal:7233"]);
        assert_eq!(profile.address.as_deref(), Some("temporal.internal:7233"));
        assert!(!startup.opens_workflows());
    }

    #[test]
    fn readonly_is_off_unless_asked_for() {
        assert!(!run(&["-p", "prod"]).1.readonly);
        assert!(run(&["-p", "prod", "--readonly"]).1.readonly);
    }

    #[test]
    fn a_long_option_takes_its_value_after_an_equals_sign() {
        let (profile, startup) = run(&[
            "--profile=prod",
            "--namespace=orders",
            "--address=localhost:7233",
            "--query=WorkflowId = 'a=b'",
        ]);
        assert_eq!(profile.name.as_deref(), Some("prod"));
        assert_eq!(profile.namespace.as_deref(), Some("orders"));
        assert_eq!(profile.address.as_deref(), Some("localhost:7233"));
        assert_eq!(
            startup.query.as_deref(),
            Some("WorkflowId = 'a=b'"),
            "only the first = separates"
        );
    }

    #[test]
    fn a_repeated_flag_takes_the_last_value() {
        let (profile, _) = run(&["-n", "one", "-n", "two"]);
        assert_eq!(profile.namespace.as_deref(), Some("two"));
    }

    #[test]
    fn a_flag_missing_its_value_says_which() {
        for (line, name) in [
            (&["-p"][..], "--profile"),
            (&["--temporal-config"][..], "--temporal-config"),
            (&["-n"][..], "--namespace"),
            (&["-q"][..], "--query"),
            (&["-w"][..], "--workflow"),
            (&["--address"][..], "--address"),
            (&["--namespace="][..], "--namespace"),
            (&["-n", " "][..], "--namespace"),
        ] {
            assert_eq!(refusal(line), format!("{name} needs a value"), "{line:?}");
        }
    }

    #[test]
    fn another_option_is_not_swallowed_as_a_value() {
        // The dangerous reading: writable, in a namespace named `--readonly`.
        let e = refusal(&["-n", "--readonly"]);
        assert!(e.starts_with("--namespace needs a value"), "{e}");
        assert!(
            e.contains("--namespace=--readonly"),
            "says the way out: {e}"
        );

        assert!(refusal(&["-w", "-n", "orders"]).starts_with("--workflow needs a value"));
    }

    #[test]
    fn a_value_that_starts_with_a_dash_goes_after_an_equals_sign() {
        let (_, startup) = run(&["--workflow=-odd-id"]);
        assert_eq!(startup.workflow.as_deref(), Some("-odd-id"));
    }

    #[test]
    fn an_unknown_argument_is_refused_with_the_usage() {
        for line in [&["--nope"][..], &["orders"][..], &["-x"][..]] {
            let e = refusal(line);
            assert!(
                e.starts_with(&format!("unknown argument `{}`", line[0])),
                "{e}"
            );
            assert!(e.ends_with(USAGE), "the usage follows: {e}");
        }
        assert!(refusal(&["--nope=1"]).starts_with("unknown argument `--nope=1`"));
    }

    #[test]
    fn a_switch_given_a_value_is_refused() {
        assert_eq!(refusal(&["--readonly=false"]), "--readonly takes no value");
        assert_eq!(refusal(&["--help=me"]), "--help takes no value");
    }

    #[test]
    fn a_workflow_id_with_a_quote_is_refused_before_connecting() {
        let e = refusal(&["-w", "it's"]);
        assert!(e.contains("quote"), "{e}");
    }

    #[test]
    fn help_and_version_answer_wherever_they_appear() {
        assert_eq!(parse_line(&["-h"]), Ok(Cli::Help));
        assert_eq!(parse_line(&["-n", "orders", "--help"]), Ok(Cli::Help));
        assert_eq!(parse_line(&["-V"]), Ok(Cli::Version));
        assert_eq!(parse_line(&["--version", "--nope"]), Ok(Cli::Version));
        // Reached first, so the bad flag wins. Parsing is in order.
        assert!(parse_line(&["--nope", "--help"]).is_err());
    }

    #[test]
    fn config_path_reflects_a_temporal_config_given_before_it() {
        assert_eq!(parse_line(&["--config-path"]), Ok(Cli::ConfigPath(None)));
        assert_eq!(
            parse_line(&["--temporal-config", "/tmp/t.toml", "--config-path"]),
            Ok(Cli::ConfigPath(Some("/tmp/t.toml".into())))
        );
        assert_eq!(
            parse_line(&["--config-path", "--temporal-config", "/tmp/t.toml"]),
            Ok(Cli::ConfigPath(None))
        );
    }

    #[test]
    fn the_usage_names_every_flag() {
        for flag in [
            "--profile",
            "--namespace",
            "--query",
            "--workflow",
            "--address",
            "--readonly",
            "--temporal-config",
            "--config-path",
            "--help",
            "--version",
        ] {
            assert!(USAGE.contains(flag), "{flag} is missing from --help");
        }
    }
}
