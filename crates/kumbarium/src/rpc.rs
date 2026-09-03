//! The MCP stdio transport, hand-rolled (D-014): newline-
//! delimited JSON-RPC 2.0, legacy-era lifecycle (initialize /
//! notifications/initialized / ping / tools/list / tools/call).
//! stdout carries protocol ONLY; diagnostics go to stderr.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use super::tools::{self, ServerState};

/// Legacy protocol revisions this server will echo back. A
/// request for anything else is answered with LATEST (per the
/// legacy negotiation rule: respond with the server's preferred
/// version and let the client decide).
const KNOWN_VERSIONS: &[&str] =
  &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
const LATEST: &str = "2025-06-18";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Serve until the reader closes (the client hanging up ends
/// the process). One message per line in, one per line out.
pub fn serve<R: BufRead, W: Write>(
  reader: R,
  writer: &mut W,
  state: &mut ServerState,
) -> std::io::Result<()> {
  for line in reader.lines() {
    let line = line?;
    if line.trim().is_empty() {
      continue;
    }
    if let Some(response) = handle_line(state, &line) {
      writeln!(writer, "{response}")?;
      writer.flush()?;
    }
  }
  Ok(())
}

/// One raw line in, at most one response out (notifications and
/// client responses produce none).
fn handle_line(state: &mut ServerState, line: &str) -> Option<Value> {
  let msg: Value = match serde_json::from_str(line) {
    Ok(v) => v,
    Err(_) => {
      return Some(error_response(Value::Null, PARSE_ERROR, "parse error"));
    }
  };
  let id = msg.get("id").cloned();
  let method = msg.get("method").and_then(Value::as_str);
  match (id, method) {
    // Request: has id + method.
    (Some(id), Some(method)) => {
      let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
      Some(handle_request(state, id, method, &params))
    }
    // Notification: method only. Nothing requires action in the
    // legacy lifecycle (initialized, cancelled, ...).
    (None, Some(_)) => None,
    // A response from the client (we never send requests), or
    // garbage with an id: id-less garbage gets no reply.
    (Some(id), None) => {
      Some(error_response(id, INVALID_REQUEST, "invalid request"))
    }
    (None, None) => None,
  }
}

fn handle_request(
  state: &mut ServerState,
  id: Value,
  method: &str,
  params: &Value,
) -> Value {
  match method {
    "initialize" => initialize(state, id, params),
    "ping" => result_response(id, json!({})),
    "tools/list" => result_response(id, tools::list()),
    "tools/call" => tools_call(state, id, params),
    _ => error_response(
      id,
      METHOD_NOT_FOUND,
      &format!("method not found: {method}"),
    ),
  }
}

fn initialize(state: &mut ServerState, id: Value, params: &Value) -> Value {
  if let Some(name) = params
    .get("clientInfo")
    .and_then(|c| c.get("name"))
    .and_then(Value::as_str)
  {
    state.agent_id = name.to_string();
  }
  let requested = params
    .get("protocolVersion")
    .and_then(Value::as_str)
    .unwrap_or(LATEST);
  let version = if KNOWN_VERSIONS.contains(&requested) {
    requested
  } else {
    LATEST
  };
  result_response(
    id,
    json!({
      "protocolVersion": version,
      "capabilities": { "tools": {} },
      "serverInfo": {
        "name": "kumbarium",
        "version": env!("CARGO_PKG_VERSION"),
      },
    }),
  )
}

fn tools_call(state: &mut ServerState, id: Value, params: &Value) -> Value {
  let Some(name) = params.get("name").and_then(Value::as_str) else {
    return error_response(
      id,
      INVALID_PARAMS,
      "tools/call requires a tool name",
    );
  };
  let args = params
    .get("arguments")
    .cloned()
    .unwrap_or_else(|| json!({}));
  let (blocks, is_error) = tools::call(state, name, &args);
  let content: Vec<Value> = blocks
    .iter()
    .map(|text| json!({ "type": "text", "text": text }))
    .collect();
  result_response(id, json!({ "content": content, "isError": is_error }))
}

fn result_response(id: Value, result: Value) -> Value {
  json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
  json!({
    "jsonrpc": "2.0",
    "id": id,
    "error": { "code": code, "message": message },
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Feed newline-joined messages through the loop; parse each
  /// response line back out.
  fn drive(state: &mut ServerState, lines: &[Value]) -> Vec<Value> {
    let input = lines
      .iter()
      .map(|v| v.to_string())
      .collect::<Vec<_>>()
      .join("\n");
    let mut out = Vec::new();
    serve(input.as_bytes(), &mut out, state).unwrap();
    String::from_utf8(out)
      .unwrap()
      .lines()
      .map(|l| serde_json::from_str(l).unwrap())
      .collect()
  }

  fn request(id: u64, method: &str, params: Value) -> Value {
    json!({
      "jsonrpc": "2.0", "id": id,
      "method": method, "params": params,
    })
  }

  fn init_request(id: u64) -> Value {
    request(
      id,
      "initialize",
      json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {},
        "clientInfo": { "name": "test-client", "version": "0" },
      }),
    )
  }

  fn call(id: u64, tool: &str, args: Value) -> Value {
    request(id, "tools/call", json!({ "name": tool, "arguments": args }))
  }

  fn text_of(response: &Value) -> String {
    response["result"]["content"]
      .as_array()
      .unwrap()
      .iter()
      .map(|b| b["text"].as_str().unwrap())
      .collect::<Vec<_>>()
      .join("\n")
  }

  #[test]
  fn initialize_negotiates_and_captures_identity() {
    let mut state = ServerState::in_memory();
    let out = drive(&mut state, &[init_request(1)]);
    let result = &out[0]["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(result["serverInfo"]["name"], "kumbarium");
    assert!(result["capabilities"]["tools"].is_object());
    assert_eq!(state.agent_id, "test-client");
  }

  #[test]
  fn unknown_protocol_version_answers_with_latest() {
    let mut state = ServerState::in_memory();
    let out = drive(
      &mut state,
      &[request(
        1,
        "initialize",
        json!({ "protocolVersion": "1900-01-01" }),
      )],
    );
    assert_eq!(out[0]["result"]["protocolVersion"], LATEST);
  }

  #[test]
  fn tools_list_names_all_six_tools() {
    let mut state = ServerState::in_memory();
    let out = drive(&mut state, &[request(1, "tools/list", json!({}))]);
    let names: Vec<&str> = out[0]["result"]["tools"]
      .as_array()
      .unwrap()
      .iter()
      .map(|t| t["name"].as_str().unwrap())
      .collect();
    assert_eq!(
      names,
      [
        "remember",
        "link",
        "recall",
        "confirm",
        "supersede",
        "forget"
      ]
    );
  }

  #[test]
  fn remember_then_recall_round_trips_with_audit() {
    let mut state = ServerState::in_memory();
    kumbarium_store::register_namespace(&state.library, "project/demo", "test")
      .unwrap();
    let out = drive(
      &mut state,
      &[
        init_request(1),
        call(
          2,
          "remember",
          json!({
            "namespace": "project/demo",
            "kind": "decision",
            "content": "the demo project pins serde at 1.x",
            "tags": ["deps"],
          }),
        ),
        call(
          3,
          "recall",
          json!({
            "query": "serde version pin",
            "scope": "project/demo",
          }),
        ),
      ],
    );
    assert_eq!(out[1]["result"]["isError"], false);
    let recall_text = text_of(&out[2]);
    assert!(recall_text.contains("pins serde"));
    assert!(recall_text.contains("relevance="));
    // Provenance carried the declared identity.
    let agent: String = state
      .library
      .query_row("SELECT agent_id FROM entries", [], |r| r.get(0))
      .unwrap();
    assert_eq!(agent, "test-client");
    // Both calls were audited.
    let events: i64 = state
      .audit
      .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
      .unwrap();
    assert_eq!(events, 2);
  }

  #[test]
  fn unregistered_namespace_is_a_tool_error_with_hint() {
    let mut state = ServerState::in_memory();
    let out = drive(
      &mut state,
      &[call(
        1,
        "remember",
        json!({
          "namespace": "project/nope",
          "kind": "decision",
          "content": "x",
        }),
      )],
    );
    assert_eq!(out[0]["result"]["isError"], true);
    assert!(text_of(&out[0]).contains("kumbarium namespace add"));
  }

  #[test]
  fn supersede_and_forget_flow_through() {
    let mut state = ServerState::in_memory();
    kumbarium_store::register_namespace(&state.library, "global", "").ok();
    let out = drive(
      &mut state,
      &[
        call(
          1,
          "remember",
          json!({
            "namespace": "global",
            "kind": "preference",
            "content": "user edits in vs code",
          }),
        ),
        call(2, "recall", json!({ "query": "editor", "scope": "global" })),
      ],
    );
    let id_line = text_of(&out[0]);
    let old_id = id_line
      .split("id=")
      .nth(1)
      .unwrap()
      .split_whitespace()
      .next()
      .unwrap()
      .to_string();
    let out = drive(
      &mut state,
      &[
        call(
          1,
          "supersede",
          json!({
            "old_id": old_id,
            "namespace": "global",
            "kind": "preference",
            "content": "user edits in neovim",
          }),
        ),
        call(
          2,
          "recall",
          json!({ "query": "edits editor", "scope": "global" }),
        ),
      ],
    );
    assert_eq!(out[0]["result"]["isError"], false);
    let recall_text = text_of(&out[1]);
    assert!(recall_text.contains("neovim"));
    assert!(!recall_text.contains("vs code"));
    let new_id = text_of(&out[0])
      .split("id=")
      .nth(1)
      .unwrap()
      .split_whitespace()
      .next()
      .unwrap()
      .to_string();
    let out = drive(&mut state, &[call(1, "forget", json!({ "id": new_id }))]);
    assert_eq!(out[0]["result"]["isError"], false);
  }

  #[test]
  fn links_flow_through_remember_and_render_in_recall() {
    let mut state = ServerState::in_memory();
    let out = drive(
      &mut state,
      &[call(
        1,
        "remember",
        json!({
          "namespace": "global",
          "kind": "reference",
          "content": "part one of the split design memory",
        }),
      )],
    );
    let part_one = text_of(&out[0])
      .split("id=")
      .nth(1)
      .unwrap()
      .split_whitespace()
      .next()
      .unwrap()
      .to_string();
    let out = drive(
      &mut state,
      &[
        call(
          1,
          "remember",
          json!({
            "namespace": "global",
            "kind": "reference",
            "content": "part two of the split design memory",
            "links": [{ "id": part_one, "rel": "continues" }],
          }),
        ),
        call(
          2,
          "recall",
          json!({ "query": "split design", "scope": "global" }),
        ),
        call(
          3,
          "link",
          json!({
            "from_id": part_one,
            "to_id": "not-a-real-id",
            "rel": "relates_to",
          }),
        ),
      ],
    );
    assert!(text_of(&out[0]).contains("links=1"));
    let recall_text = text_of(&out[1]);
    assert!(recall_text.contains("links: continues ->"));
    assert!(recall_text.contains("links: continues <-"));
    // Dangling link through the link tool is a tool error.
    assert_eq!(out[2]["result"]["isError"], true);
  }

  #[test]
  fn oversized_remember_splits_and_chains_automatically() {
    let mut state = ServerState::in_memory();
    let content = (0..40)
      .map(|i| format!("paragraph {i} {}", "y".repeat(60)))
      .collect::<Vec<_>>()
      .join("\n\n");
    let out = drive(
      &mut state,
      &[call(
        1,
        "remember",
        json!({
          "namespace": "global",
          "kind": "reference",
          "content": content,
        }),
      )],
    );
    assert_eq!(out[0]["result"]["isError"], false);
    let text = text_of(&out[0]);
    assert!(text.contains("linked parts"), "{text}");
    let entries: i64 = state
      .library
      .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
      .unwrap();
    assert!(entries > 1);
    let chains: i64 = state
      .library
      .query_row(
        "SELECT count(*) FROM entry_links WHERE rel='continues'",
        [],
        |r| r.get(0),
      )
      .unwrap();
    assert_eq!(chains, entries - 1, "every part chained");
  }

  #[test]
  fn protocol_errors_are_json_rpc_errors() {
    let mut state = ServerState::in_memory();
    let input = "this is not json\n";
    let mut out = Vec::new();
    serve(input.as_bytes(), &mut out, &mut state).unwrap();
    let v: Value = serde_json::from_slice(out.trim_ascii_end()).unwrap();
    assert_eq!(v["error"]["code"], PARSE_ERROR);

    let out = drive(&mut state, &[request(7, "nonsense/method", json!({}))]);
    assert_eq!(out[0]["error"]["code"], METHOD_NOT_FOUND);
    assert_eq!(out[0]["id"], 7);
  }

  #[test]
  fn notifications_produce_no_response() {
    let mut state = ServerState::in_memory();
    let out = drive(
      &mut state,
      &[json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
      })],
    );
    assert!(out.is_empty());
  }
}
