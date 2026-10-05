# Security

## Supported versions

Only the latest release. A fix ships as a new release; nothing is backported.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting: the
[Security tab](https://github.com/arisros/tmprl/security) of this repository, then
**Report a vulnerability**. A report made that way is visible only to the maintainers.

If that button is not there, open an issue that says only that you have a security report,
with no details in it, and a private channel will be arranged.

Do not put the details in a public issue, pull request or discussion.

## What is security relevant here

`tmprl` runs with the access of whoever starts it, against clusters that may be production.
These are the places where a bug is a security bug:

| Area | What tmprl does |
|---|---|
| Cluster credentials | Connections come from the Temporal profile file, the `temporal.toml` the `temporal` CLI uses, and the `TEMPORAL_*` variables, through Temporal's own loader. That file can hold an API key and TLS certificate paths. tmprl reads it and never writes it |
| Codec server | Payloads that need decoding are sent to the codec endpoint set in tmprl's `config.toml`, as `POST {endpoint}/decode` with an `X-Namespace` header. `Authorization` is sent only when `auth` is set there, verbatim. That makes `config.toml` a file that can hold a credential |
| `!` | Runs the command you type through `sh -c`, with the readable payloads under the cursor as JSON on stdin |
| A source resolver | `gf` runs the program `[source] command` names, without a shell, with the focused event's names, worker identities and readable payloads as JSON on stdin. It then runs your editor, also without a shell, on the path the resolver printed |
| `$EDITOR` | `<leader>e` writes the readable payloads to a file with mode 0600 in a fresh directory with mode 0700 under the temp directory, runs `$VISUAL`, else `$EDITOR`, else `vi` on it without a shell, and removes the directory when the editor exits |
| Yank | Sent to the terminal as OSC 52, or inside tmux through `tmux load-buffer -w`. A yank over `[yank] max_bytes` (default 65536) is written instead to a file with mode 0600 in a fresh directory with mode 0700 under the temp directory, named in the status line, and is not removed by tmprl |
| Audit log | Every mutation attempted, failures included, is appended to `$XDG_STATE_HOME/tmprl/audit.jsonl`, else `~/.local/state/tmprl/audit.jsonl`. A line holds the time, the action, the profile, the cluster address, the namespace, the workflow id, the run id, the outcome and the equivalent `temporal` command |

Examples of what to report:

- a credential, or a payload, sent anywhere other than the cluster and the codec endpoint
  that were configured
- data that came from a server (a workflow id, a payload, a failure message) being executed,
  breaking out of the quoting of the `temporal` command shown in a confirmation, or reaching
  the terminal as control sequences
- a mutation that runs without its confirmation, runs on a profile marked `readonly`, or is
  not written to the audit log
- decoded payloads left on disk, or readable by another user

A command you type at the `!` prompt running with your own access is the feature, not a
vulnerability.
