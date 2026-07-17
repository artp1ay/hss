//! Minimal MCP server (streamable HTTP transport, JSON-RPC over POST).
//! Exposes hss hosts to MCP clients: `list-servers` and `execute-command`.

use std::sync::{Arc, Mutex};
use std::time::Instant;
use anyhow::Result;
use serde_json::{json, Value};

// ponytail: fixed localhost port; add a config field when someone actually needs to change it
pub const MCP_PORT: u16 = 8822;

pub struct McpServer {
    server: Arc<tiny_http::Server>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub log: Arc<Mutex<Vec<String>>>,
    pub started: Instant,
}

impl McpServer {
    pub fn start() -> Result<Self> {
        let server = tiny_http::Server::http(("127.0.0.1", MCP_PORT))
            .map_err(|e| anyhow::anyhow!("MCP server failed to bind 127.0.0.1:{MCP_PORT}: {e}"))?;
        let server = Arc::new(server);
        let log = Arc::new(Mutex::new(Vec::new()));
        let started = Instant::now();
        push_log(&log, started, format!("Listening on http://127.0.0.1:{MCP_PORT}"));

        let (srv, lg) = (server.clone(), log.clone());
        let thread = std::thread::spawn(move || serve_loop(srv, lg, started));
        Ok(Self { server, thread: Some(thread), log, started })
    }

    pub fn url() -> String {
        format!("http://127.0.0.1:{MCP_PORT}")
    }

    pub fn stop(mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn push_log(log: &Arc<Mutex<Vec<String>>>, started: Instant, msg: String) {
    let mut l = log.lock().unwrap();
    l.push(format!("[{:>4}s] {msg}", started.elapsed().as_secs()));
    // ponytail: cap memory; no scrollback beyond 500 lines
    if l.len() > 500 {
        let drop = l.len() - 500;
        l.drain(..drop);
    }
}

fn serve_loop(server: Arc<tiny_http::Server>, log: Arc<Mutex<Vec<String>>>, started: Instant) {
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let (status, body) = match method {
            tiny_http::Method::Post => {
                let mut buf = String::new();
                let _ = std::io::Read::read_to_string(request.as_reader(), &mut buf);
                handle_rpc(&buf, &log, started)
            }
            // Session termination per streamable-http spec — nothing to clean up
            tiny_http::Method::Delete => (200, None),
            _ => (405, None),
        };

        let response = match body {
            Some(json) => tiny_http::Response::from_string(json.to_string())
                .with_status_code(status)
                .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap()),
            None => tiny_http::Response::from_string("").with_status_code(status),
        };
        let _ = request.respond(response);
    }
}

/// Returns (http status, optional JSON-RPC response body).
fn handle_rpc(body: &str, log: &Arc<Mutex<Vec<String>>>, started: Instant) -> (u16, Option<Value>) {
    let Ok(req) = serde_json::from_str::<Value>(body) else {
        return (400, Some(rpc_error(Value::Null, -32700, "Parse error")));
    };
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // Notifications get no response body
    let Some(id) = id else {
        return (202, None);
    };

    let result = match method {
        "initialize" => {
            let proto = req.pointer("/params/protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2025-03-26");
            push_log(log, started, format!("Client connected (protocol {proto})"));
            json!({
                "protocolVersion": proto,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "hss", "version": env!("CARGO_PKG_VERSION") }
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tool_definitions() }),
        "tools/call" => {
            let name = req.pointer("/params/name").and_then(|v| v.as_str()).unwrap_or("");
            let args = req.pointer("/params/arguments").cloned().unwrap_or(json!({}));
            call_tool(name, &args, log, started)
        }
        _ => return (200, Some(rpc_error(id, -32601, &format!("Method not found: {method}")))),
    };

    (200, Some(json!({ "jsonrpc": "2.0", "id": id, "result": result })))
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "list-servers",
            "description": "List all SSH servers configured in hss (name, group, address, port, tags, description).",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "execute-command",
            "description": "Execute a shell command on one of the configured SSH servers and return its output.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string", "description": "Server name or IP as configured in hss" },
                    "command": { "type": "string", "description": "Shell command to execute" }
                },
                "required": ["host", "command"]
            }
        }
    ])
}

fn call_tool(name: &str, args: &Value, log: &Arc<Mutex<Vec<String>>>, started: Instant) -> Value {
    match name {
        "list-servers" => {
            push_log(log, started, "tool: list-servers".into());
            match crate::config::load_hosts() {
                Ok(hosts) => {
                    let list: Vec<Value> = hosts.iter().map(|h| json!({
                        "name": h.name, "group": h.group, "host": h.ip, "port": h.port,
                        "user": h.user, "tags": h.tags, "description": h.description,
                    })).collect();
                    tool_text(serde_json::to_string_pretty(&list).unwrap_or_default(), false)
                }
                Err(e) => tool_text(format!("Failed to load hosts: {e}"), true),
            }
        }
        "execute-command" => {
            let host = args.get("host").and_then(|v| v.as_str()).unwrap_or("");
            let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if host.is_empty() || command.is_empty() {
                return tool_text("Both 'host' and 'command' are required.".into(), true);
            }
            push_log(log, started, format!("tool: execute-command on '{host}': {command}"));
            match crate::ssh::exec_command(host, command) {
                Ok(out) => {
                    let code = out.status.code().unwrap_or(-1);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    push_log(log, started, format!("  → exit {code}"));
                    let mut text = format!("exit code: {code}\n");
                    if !stdout.is_empty() { text.push_str(&format!("stdout:\n{stdout}")); }
                    if !stderr.is_empty() { text.push_str(&format!("stderr:\n{stderr}")); }
                    tool_text(text, !out.status.success())
                }
                Err(e) => {
                    push_log(log, started, format!("  → error: {e}"));
                    tool_text(format!("Error: {e}"), true)
                }
            }
        }
        _ => tool_text(format!("Unknown tool: {name}"), true),
    }
}

fn tool_text(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpc(body: &str) -> (u16, Option<Value>) {
        handle_rpc(body, &Arc::new(Mutex::new(vec![])), Instant::now())
    }

    #[test]
    fn initialize_lists_tools() {
        let (status, resp) = rpc(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#);
        assert_eq!(status, 200);
        let resp = resp.unwrap();
        assert_eq!(resp["result"]["serverInfo"]["name"], "hss");

        let (_, resp) = rpc(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let tools = resp.unwrap()["result"]["tools"].as_array().unwrap().iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(tools, ["list-servers", "execute-command"]);
    }

    #[test]
    fn notification_gets_202_no_body() {
        let (status, resp) = rpc(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert_eq!(status, 202);
        assert!(resp.is_none());
    }

    #[test]
    fn unknown_method_and_parse_error() {
        let (_, resp) = rpc(r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#);
        assert_eq!(resp.unwrap()["error"]["code"], -32601);
        let (status, _) = rpc("not json");
        assert_eq!(status, 400);
    }

    #[test]
    fn missing_tool_args_is_error() {
        let (_, resp) = rpc(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"execute-command","arguments":{}}}"#);
        assert_eq!(resp.unwrap()["result"]["isError"], true);
    }

    #[test]
    fn server_starts_serves_and_stops() {
        let server = McpServer::start().expect("bind");
        let resp: Value = ureq::post(&format!("{}/mcp", McpServer::url()))
            .send_json(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
            .expect("http ok")
            .into_json().expect("json");
        assert_eq!(resp["id"], 1);
        server.stop(); // must not hang
    }
}
