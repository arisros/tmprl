//! Workflow queries: the handlers a workflow says it has.
//!
//! A query is answered by a worker, which replays the history and calls a function in the
//! workflow's own code. The server only relays it, so what can be asked differs per workflow
//! type and is known only by asking the workflow, through `__temporal_workflow_metadata`.
//! An SDK that predates that handler refuses it, and most refusals list the names it does
//! know, which is the fallback before asking the reader to type one.

use crate::payload::Payload;

pub const METADATA: &str = "__temporal_workflow_metadata";
pub const STACK_TRACE: &str = "__stack_trace";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handler {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Handlers {
    pub queries: Vec<Handler>,
    /// What the workflow says it is doing, `workflow.SetCurrentDetails` in Go.
    pub current_details: Option<String>,
}

impl Handlers {
    /// From the metadata as an SDK sends it, `json/protobuf`. Proto JSON is camel case;
    /// snake case is accepted too, since that is what a serde derive over the proto writes.
    pub fn from_metadata_json(text: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let field = |v: &serde_json::Value, camel: &str, snake: &str| {
            v.get(camel).or_else(|| v.get(snake)).cloned()
        };
        let definition = field(&v, "definition", "definition").unwrap_or_default();
        let queries = field(&definition, "queryDefinitions", "query_definitions")
            .and_then(|q| q.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|q| {
                Some(Handler {
                    name: q.get("name")?.as_str()?.to_string(),
                    description: q
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or_default()
                        .to_string(),
                })
            })
            .collect();
        let current_details = field(&v, "currentDetails", "current_details")
            .and_then(|d| d.as_str().map(str::to_string))
            .filter(|d| !d.is_empty());
        Some(Self {
            queries,
            current_details,
        })
    }

    /// From the names a refusal listed, which carry no descriptions.
    pub fn from_names(names: Vec<String>) -> Self {
        Self {
            queries: names
                .into_iter()
                .map(|name| Handler {
                    name,
                    description: String::new(),
                })
                .collect(),
            current_details: None,
        }
    }

    /// What a picker offers: the workflow's own handlers, then the stack trace. Other `__`
    /// names describe the SDK rather than the workflow, so they are left out.
    pub fn offered(&self) -> Vec<Handler> {
        let mut out: Vec<Handler> = self
            .queries
            .iter()
            .filter(|h| !h.name.starts_with("__"))
            .cloned()
            .collect();
        out.push(
            self.queries
                .iter()
                .find(|h| h.name == STACK_TRACE)
                .cloned()
                .unwrap_or_else(|| Handler {
                    name: STACK_TRACE.into(),
                    description: "where the workflow is blocked".into(),
                }),
        );
        out
    }
}

/// The names a refusal of an unknown query type lists, when it lists them.
///
/// Go says `KnownQueryTypes=[a b]`, Java `knownTypes=[a, b]`, TypeScript
/// `Registered queries: [a b]`. Anything else is not guessed at.
pub fn known_from_refusal(message: &str) -> Option<Vec<String>> {
    let after = ["KnownQueryTypes=", "knownTypes=", "Registered queries:"]
        .iter()
        .find_map(|marker| message.find(marker).map(|at| &message[at + marker.len()..]))?;
    let open = after.find('[')?;
    let close = after[open..].find(']')? + open;
    let names: Vec<String> = after[open + 1..close]
        .split([' ', ','])
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect();
    (!names.is_empty()).then_some(names)
}

/// Labels for the payload pane: the handler's name, indexed when it returned several.
pub fn labelled(name: &str, payloads: Vec<Payload>) -> Vec<(String, Payload)> {
    let several = payloads.len() > 1;
    payloads
        .into_iter()
        .map(as_lines)
        .enumerate()
        .map(|(i, p)| {
            let label = if several {
                format!("{name}[{i}]")
            } else {
                name.to_string()
            };
            (label, p)
        })
        .collect()
}

/// A JSON string that spans lines, as `__stack_trace` answers, shown as its lines rather
/// than as one line of `\n` escapes.
fn as_lines(p: Payload) -> Payload {
    if p.encoding != "json/plain" {
        return p;
    }
    match serde_json::from_slice::<serde_json::Value>(&p.data) {
        Ok(serde_json::Value::String(s)) if s.contains('\n') => {
            Payload::new("text/plain", s.into_bytes())
        }
        _ => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_in_proto_json_gives_handlers_and_details() {
        let h = Handlers::from_metadata_json(
            r#"{"definition":{"type":"LoanWorkflow","queryDefinitions":[
                {"name":"getState","description":"the loan stage"},
                {"name":"__stack_trace"}]},
               "currentDetails":"waiting for approval"}"#,
        )
        .unwrap();
        assert_eq!(h.queries.len(), 2);
        assert_eq!(h.queries[0].name, "getState");
        assert_eq!(h.queries[0].description, "the loan stage");
        assert_eq!(h.current_details.as_deref(), Some("waiting for approval"));
    }

    #[test]
    fn metadata_in_snake_case_is_read_too() {
        let h = Handlers::from_metadata_json(
            r#"{"definition":{"query_definitions":[{"name":"getState"}]},"current_details":""}"#,
        )
        .unwrap();
        assert_eq!(h.queries[0].name, "getState");
        assert_eq!(h.current_details, None, "empty details are no details");
    }

    #[test]
    fn metadata_that_is_not_json_is_none() {
        assert_eq!(Handlers::from_metadata_json("\u{8}\u{1}binary"), None);
    }

    #[test]
    fn the_picker_hides_sdk_internals_and_ends_on_the_stack_trace() {
        let h = Handlers::from_names(vec![
            "__stack_trace".into(),
            "__open_sessions".into(),
            "getState".into(),
        ]);
        let names: Vec<_> = h.offered().into_iter().map(|h| h.name).collect();
        assert_eq!(names, ["getState", "__stack_trace"]);
    }

    #[test]
    fn the_stack_trace_is_offered_even_when_not_listed() {
        let names: Vec<_> = Handlers::default()
            .offered()
            .into_iter()
            .map(|h| h.name)
            .collect();
        assert_eq!(names, ["__stack_trace"]);
    }

    #[test]
    fn refusals_from_each_sdk_list_their_names() {
        assert_eq!(
            known_from_refusal(
                "unknown queryType __temporal_workflow_metadata. KnownQueryTypes=[__stack_trace getState]"
            ),
            Some(vec!["__stack_trace".into(), "getState".into()])
        );
        assert_eq!(
            known_from_refusal("Unknown query type: x, knownTypes=[getState, getStage]"),
            Some(vec!["getState".into(), "getStage".into()])
        );
        assert_eq!(
            known_from_refusal(
                "Workflow did not register a handler for x. Registered queries: [__stack_trace getState]"
            ),
            Some(vec!["__stack_trace".into(), "getState".into()])
        );
    }

    #[test]
    fn a_refusal_without_a_list_is_not_guessed_at() {
        assert_eq!(known_from_refusal("query timed out"), None);
        assert_eq!(known_from_refusal("KnownQueryTypes=[]"), None);
    }

    #[test]
    fn one_result_is_labelled_by_the_handler_and_several_are_indexed() {
        let p = || Payload::new("json/plain", b"1".to_vec());
        let one = labelled("getState", vec![p()]);
        assert_eq!(one[0].0, "getState");
        let two = labelled("getState", vec![p(), p()]);
        assert_eq!(two[1].0, "getState[1]");
    }

    #[test]
    fn a_stack_trace_reads_as_lines_and_a_one_line_string_stays_json() {
        let trace = Payload::new(
            "json/plain",
            br#""coroutine 1 [blocked]:\nmain.Loan()""#.to_vec(),
        );
        let shown = &labelled(STACK_TRACE, vec![trace])[0].1;
        assert_eq!(shown.encoding, "text/plain");
        assert_eq!(shown.data, b"coroutine 1 [blocked]:\nmain.Loan()");

        let word = Payload::new("json/plain", br#""approved""#.to_vec());
        assert_eq!(labelled("getStage", vec![word])[0].1.encoding, "json/plain");
    }
}
