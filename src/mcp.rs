//! Minimal MCP server (streamable HTTP transport, JSON-RPC over POST).
//! Exposes hss hosts to MCP clients: `list-servers` and `execute-command`.
//!
//! Set `HSS_MCP_TOKEN` env var or `mcp_token` in config.toml to require a
//! Bearer token. When unset the server accepts unauthenticated requests.
//! Hosts/credentials are re-read from disk on every tool call, so changes
//! made in a second hss window are visible without restart.

use std::sync::{Arc, Mutex};
use std::time::Instant;
use anyhow::Result;
use serde_json::{json, Value};

pub const MCP_PORT: u16 = 8822;

/// Severity of a log entry; drives colour in the MCP screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Info,
    Req,
    Ok,
    Err,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Req => "CALL",
            Level::Ok => " OK ",
            Level::Err => "FAIL",
        }
    }
}

/// One structured line: when it happened, how bad it is, which subsystem,
/// a one-line summary and optional indented detail lines.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub at: f64, // seconds since server start
    pub level: Level,
    pub tag: String,
    pub msg: String,
    pub detail: Vec<String>,
}

#[derive(Debug, Default)]
pub struct LogState {
    pub entries: Vec<LogEntry>,
    pub requests: u64,
    pub tool_calls: u64,
    pub errors: u64,
    pub client: Option<String>,
    pub log_file: Option<std::path::PathBuf>,
}

pub type Log = Arc<Mutex<LogState>>;

pub struct McpServer {
    server: Arc<tiny_http::Server>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub log: Log,
    pub started: Instant,
    pub port: u16,
}

impl McpServer {
    pub fn start() -> Result<Self> {
        let cfg = crate::config::load_config().unwrap_or_default();
        let port = if cfg.mcp_port != 0 { cfg.mcp_port } else { MCP_PORT };
        let server = tiny_http::Server::http(("127.0.0.1", port))
            .map_err(|e| anyhow::anyhow!("MCP server failed to bind 127.0.0.1:{port}: {e}"))?;
        let server = Arc::new(server);

        let log_file = cfg.mcp_log_file.as_deref().and_then(|p| {
            let p = p.trim();
            if p.is_empty() { None } else { Some(std::path::PathBuf::from(crate::config::expand_tilde(p))) }
        });

        let mut initial_state = LogState::default();
        initial_state.log_file = log_file;
        let log: Log = Arc::new(Mutex::new(initial_state));
        let started = Instant::now();

        let token = resolve_mcp_token();
        let auth_hint = if token.is_some() { "auth: bearer token required" } else { "auth: none (set HSS_MCP_TOKEN or mcp_token in config.toml)" };
        log_line(&log, started, Level::Info, "server", format!("listening on http://127.0.0.1:{port}"), vec![
            "transport: streamable HTTP (JSON-RPC over POST)".into(),
            "tools: list-servers, execute-command".into(),
            auth_hint.into(),
        ]);

        let (srv, lg) = (server.clone(), log.clone());
        let thread = std::thread::spawn(move || serve_loop(srv, lg, started, token));
        Ok(Self { server, thread: Some(thread), log, started, port })
    }

    pub fn url() -> String {
        let port = crate::config::load_config().map(|c| c.mcp_port).unwrap_or(MCP_PORT);
        let port = if port != 0 { port } else { MCP_PORT };
        format!("http://127.0.0.1:{port}")
    }

    pub fn server_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn stop(mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn log_line(log: &Log, started: Instant, level: Level, tag: &str, msg: String, detail: Vec<String>) {
    let mut l = log.lock().unwrap();
    if level == Level::Err {
        l.errors += 1;
    }

    if let Some(ref path) = l.log_file {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let at = started.elapsed().as_secs_f64();
            let _ = writeln!(f, "[{:>7.2}s] [{}] [{:<8}] {}", at, level.label(), tag, msg);
            for d in &detail {
                let _ = writeln!(f, "    | {d}");
            }
        }
    }

    l.entries.push(LogEntry {
        at: started.elapsed().as_secs_f64(),
        level,
        tag: tag.to_string(),
        msg,
        detail,
    });
    // Cap log memory to 500 entries
    if l.entries.len() > 500 {
        let drop = l.entries.len() - 500;
        l.entries.drain(..drop);
    }
}

/// Shorten a value for single-line display.
fn clip(s: &str, max: usize) -> String {
    let one_line = s.replace('\n', "⏎");
    if one_line.chars().count() <= max {
        return one_line;
    }
    let head: String = one_line.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

fn resolve_mcp_token() -> Option<String> {
    if let Ok(t) = std::env::var("HSS_MCP_TOKEN") {
        let t = t.trim().to_string();
        if !t.is_empty() { return Some(t); }
    }
    if let Ok(cfg) = crate::config::load_config() {
        if let Some(t) = cfg.mcp_token {
            let t = t.trim().to_string();
            if !t.is_empty() { return Some(t); }
        }
    }
    None
}

fn check_auth(request: &tiny_http::Request, token: &Option<String>) -> bool {
    let Some(expected) = token else { return true };
    for h in request.headers() {
        if h.field.equiv("Authorization") {
            if let Some(bearer) = h.value.as_str().strip_prefix("Bearer ") {
                return bearer.trim() == expected;
            }
        }
    }
    false
}

fn serve_loop(server: Arc<tiny_http::Server>, log: Log, started: Instant, token: Option<String>) {
    for mut request in server.incoming_requests() {
        let method = request.method().clone();

        // Auth check (applies to all methods when a token is configured)
        if !check_auth(&request, &token) {
            log_line(&log, started, Level::Err, "auth", "rejected: invalid or missing Bearer token".into(), vec![
                format!("{} {}", method, request.url()),
            ]);
            let _ = request.respond(make_response(401, Some(json!({"error": "unauthorized"}))));
            continue;
        }

        // Fast path: non-POST requests are answered inline.
        if method != tiny_http::Method::Post {
            let (status, body) = match method {
                tiny_http::Method::Delete => {
                    log_line(&log, started, Level::Info, "http", "session closed by client (DELETE)".into(), vec![]);
                    (200, None)
                }
                other => {
                    log_line(&log, started, Level::Err, "http", format!("rejected {other} {}", request.url()), vec![
                        "only POST (JSON-RPC) and DELETE are supported".into(),
                    ]);
                    (405, None)
                }
            };
            let _ = request.respond(make_response(status, body));
            continue;
        }

        // Read the body in the accept thread (the reader is tied to the request).
        let mut buf = String::new();
        let _ = std::io::Read::read_to_string(request.as_reader(), &mut buf);

        // Peek at the method to decide sync vs async.
        let rpc_method = serde_json::from_str::<Value>(&buf)
            .ok()
            .and_then(|v| v.get("method").and_then(|m| m.as_str()).map(String::from));

        let is_tool_call = rpc_method.as_deref() == Some("tools/call");

        if is_tool_call {
            // Tool calls may run SSH commands that take minutes.
            // Handle them in a separate thread so the accept loop keeps
            // responding to pings and other fast requests.
            let lg = log.clone();
            std::thread::spawn(move || {
                let (status, body) = handle_rpc(&buf, &lg, started);
                let _ = request.respond(make_response(status, body));
            });
        } else {
            // Fast RPC: ping, initialize, tools/list, notifications.
            let (status, body) = handle_rpc(&buf, &log, started);
            let _ = request.respond(make_response(status, body));
        }
    }
}

fn make_response(status: u16, body: Option<Value>) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    match body {
        Some(json) => {
            let bytes = json.to_string().into_bytes();
            let len = bytes.len();
            tiny_http::Response::new(
                tiny_http::StatusCode(status),
                vec!["Content-Type: application/json".parse::<tiny_http::Header>().unwrap()],
                std::io::Cursor::new(bytes),
                Some(len),
                None,
            )
        }
        None => tiny_http::Response::new(
            tiny_http::StatusCode(status),
            vec![],
            std::io::Cursor::new(vec![]),
            Some(0),
            None,
        ),
    }
}

/// Returns (http status, optional JSON-RPC response body).
fn handle_rpc(body: &str, log: &Log, started: Instant) -> (u16, Option<Value>) {
    log.lock().unwrap().requests += 1;

    let Ok(req) = serde_json::from_str::<Value>(body) else {
        log_line(log, started, Level::Err, "rpc", "malformed JSON request".into(), vec![
            format!("body: {}", clip(body, 120)),
        ]);
        return (400, Some(rpc_error(Value::Null, -32700, "Parse error")));
    };
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // Notifications get no response body
    let Some(id) = id else {
        log_line(log, started, Level::Info, "rpc", format!("notification {method}"), vec![]);
        return (202, None);
    };
    let id_str = id.to_string();

    let result = match method {
        "initialize" => {
            let proto = req.pointer("/params/protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2025-03-26");
            let client = req.pointer("/params/clientInfo/name").and_then(|v| v.as_str()).unwrap_or("unknown");
            let client_ver = req.pointer("/params/clientInfo/version").and_then(|v| v.as_str()).unwrap_or("?");
            log.lock().unwrap().client = Some(format!("{client} {client_ver}"));
            log_line(log, started, Level::Ok, "session", format!("client connected: {client} {client_ver}"), vec![
                format!("protocol: {proto}"),
                format!("server: hss {}", env!("CARGO_PKG_VERSION")),
            ]);
            json!({
                "protocolVersion": proto,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "hss", "version": env!("CARGO_PKG_VERSION") }
            })
        }
        "ping" => {
            log_line(log, started, Level::Info, "rpc", "ping".into(), vec![]);
            json!({})
        }
        "tools/list" => {
            log_line(log, started, Level::Info, "rpc", "tools/list → 2 tools".into(), vec![
                "list-servers, execute-command".into(),
            ]);
            json!({ "tools": tool_definitions() })
        }
        "tools/call" => {
            let name = req.pointer("/params/name").and_then(|v| v.as_str()).unwrap_or("");
            let args = req.pointer("/params/arguments").cloned().unwrap_or(json!({}));
            call_tool(name, &args, log, started, &id_str)
        }
        _ => {
            log_line(log, started, Level::Err, "rpc", format!("unknown method: {method}"), vec![
                format!("request id {id_str}"),
            ]);
            return (200, Some(rpc_error(id, -32601, &format!("Method not found: {method}"))));
        }
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

fn call_tool(name: &str, args: &Value, log: &Log, started: Instant, req_id: &str) -> Value {
    log.lock().unwrap().tool_calls += 1;
    let t0 = Instant::now();

    match name {
        "list-servers" => {
            log_line(log, started, Level::Req, "tool", "list-servers".into(), vec![
                format!("request id {req_id} · no arguments"),
            ]);
            match crate::config::load_hosts() {
                Ok(hosts) => {
                    let groups: std::collections::BTreeSet<&str> =
                        hosts.iter().map(|h| h.group.as_str()).filter(|g| !g.is_empty()).collect();
                    let list: Vec<Value> = hosts.iter().map(|h| json!({
                        "name": h.name, "group": h.group, "host": h.ip, "port": h.port,
                        "user": h.user, "tags": h.tags, "description": h.description,
                    })).collect();
                    let text = serde_json::to_string_pretty(&list).unwrap_or_default();
                    log_line(log, started, Level::Ok, "tool", format!("list-servers → {} servers ({} ms)", hosts.len(), t0.elapsed().as_millis()), vec![
                        format!("groups: {}", if groups.is_empty() { "—".to_string() } else { groups.into_iter().collect::<Vec<_>>().join(", ") }),
                        format!("payload: {} bytes", text.len()),
                    ]);
                    tool_text(text, false)
                }
                Err(e) => {
                    log_line(log, started, Level::Err, "tool", format!("list-servers failed ({} ms)", t0.elapsed().as_millis()), vec![
                        format!("error: {e}"),
                    ]);
                    tool_text(format!("Failed to load hosts: {e}"), true)
                }
            }
        }
        "execute-command" => {
            let host = args.get("host").and_then(|v| v.as_str()).unwrap_or("");
            let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if host.is_empty() || command.is_empty() {
                log_line(log, started, Level::Err, "tool", "execute-command rejected: missing arguments".into(), vec![
                    format!("host: {:?} · command: {:?}", host, clip(command, 60)),
                ]);
                return tool_text("Both 'host' and 'command' are required.".into(), true);
            }
            log_line(log, started, Level::Req, "tool", format!("execute-command @ {host}"), vec![
                format!("request id {req_id}"),
                format!("$ {}", clip(command, 100)),
            ]);
            match crate::ssh::exec_command(host, command) {
                Ok(out) => {
                    let code = out.status.code().unwrap_or(-1);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let ms = t0.elapsed().as_millis();
                    let mut detail = vec![format!(
                        "stdout {} lines/{} B · stderr {} lines/{} B",
                        stdout.lines().count(), stdout.len(), stderr.lines().count(), stderr.len()
                    )];
                    if let Some(first) = stdout.lines().find(|l| !l.trim().is_empty()) {
                        detail.push(format!("out: {}", clip(first, 100)));
                    }
                    if let Some(err) = stderr.lines().find(|l| !l.trim().is_empty()) {
                        detail.push(format!("err: {}", clip(err, 100)));
                    }
                    let level = if out.status.success() { Level::Ok } else { Level::Err };
                    log_line(log, started, level, "tool", format!("execute-command @ {host} → exit {code} ({ms} ms)"), detail);
                    let mut text = format!("exit code: {code}\n");
                    if !stdout.is_empty() { text.push_str(&format!("stdout:\n{stdout}")); }
                    if !stderr.is_empty() { text.push_str(&format!("stderr:\n{stderr}")); }
                    tool_text(text, !out.status.success())
                }
                Err(e) => {
                    log_line(log, started, Level::Err, "tool", format!("execute-command @ {host} failed ({} ms)", t0.elapsed().as_millis()), vec![
                        format!("error: {e}"),
                    ]);
                    tool_text(format!("Error: {e}"), true)
                }
            }
        }
        _ => {
            log_line(log, started, Level::Err, "tool", format!("unknown tool: {name}"), vec![
                format!("request id {req_id}"),
            ]);
            tool_text(format!("Unknown tool: {name}"), true)
        }
    }
}

fn tool_text(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpc(body: &str) -> (u16, Option<Value>) {
        handle_rpc(body, &Arc::new(Mutex::new(LogState::default())), Instant::now())
    }

    #[test]
    fn log_records_levels_counters_and_detail() {
        let log: Log = Arc::new(Mutex::new(LogState::default()));
        let started = Instant::now();
        handle_rpc(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"claude","version":"1.2"}}}"#, &log, started);
        handle_rpc("not json", &log, started);
        let l = log.lock().unwrap();
        assert_eq!(l.requests, 2);
        assert_eq!(l.errors, 1);
        assert_eq!(l.client.as_deref(), Some("claude 1.2"));
        assert_eq!(l.entries[0].level, Level::Ok);
        assert!(l.entries[0].msg.contains("claude 1.2"));
        assert!(l.entries[0].detail.iter().any(|d| d.starts_with("protocol:")));
        assert_eq!(l.entries[1].level, Level::Err);
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
