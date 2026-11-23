use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MAX_ACTIVE_CONNS: usize = 64;
static ACTIVE_PROXY_CONNS: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
pub struct ProxyMetrics {
    pub active_conns: AtomicUsize,
    pub total_requests: AtomicU64,
    last_error: Mutex<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct ProxyMetricsSnapshot {
    pub active_conns: usize,
    pub total_requests: u64,
    pub last_error: Option<String>,
}

impl ProxyMetrics {
    pub fn snapshot(&self) -> ProxyMetricsSnapshot {
        let last_error = self.last_error.lock().ok().and_then(|g| g.clone());
        ProxyMetricsSnapshot {
            active_conns: self.active_conns.load(Ordering::Relaxed),
            total_requests: self.total_requests.load(Ordering::Relaxed),
            last_error,
        }
    }

    fn record_error(&self, err: &anyhow::Error) {
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = Some(format!("{err:#}"));
        }
    }
}

struct ProxyConnGuard {
    metrics: Option<Arc<ProxyMetrics>>,
}

impl Drop for ProxyConnGuard {
    fn drop(&mut self) {
        ACTIVE_PROXY_CONNS.fetch_sub(1, Ordering::SeqCst);
        if let Some(ref metrics) = self.metrics {
            metrics.active_conns.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

pub struct HttpProxy {
    port: u16,
    stopped: Arc<AtomicBool>,
    metrics: Arc<ProxyMetrics>,
    thread: Option<JoinHandle<()>>,
}

impl HttpProxy {
    #[cfg(test)]
    pub fn start(port: u16, socks_port: u16) -> Result<Self> {
        Self::start_with_log(port, socks_port, None)
    }

    pub fn start_with_log(
        port: u16,
        socks_port: u16,
        log: Option<Arc<Mutex<crate::tui::ProxyStderrLog>>>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .with_context(|| format!("Cannot bind HTTP proxy to 127.0.0.1:{port}"))?;
        let bound_port = listener.local_addr()?.port();
        if let Some(ref l) = log {
            if let Ok(mut g) = l.lock() {
                g.push_event(
                    "http",
                    format!("HTTP/HTTPS proxy listening on 127.0.0.1:{bound_port} -> SOCKS5 127.0.0.1:{socks_port}"),
                );
            }
        }
        let stopped = Arc::new(AtomicBool::new(false));
        let stopped_bg = Arc::clone(&stopped);
        let metrics = Arc::new(ProxyMetrics::default());
        let metrics_bg = Arc::clone(&metrics);

        let thread = thread::spawn(move || loop {
            match listener.accept() {
                Ok((mut stream, peer_addr)) => {
                    if stopped_bg.load(Ordering::SeqCst) {
                        break;
                    }
                    let reserved =
                        ACTIVE_PROXY_CONNS.try_update(Ordering::SeqCst, Ordering::SeqCst, |cur| {
                            (cur < MAX_ACTIVE_CONNS).then_some(cur + 1)
                        });
                    if reserved.is_err() {
                        if let Some(ref l) = log {
                            if let Ok(mut g) = l.lock() {
                                g.push_event(
                                    "http",
                                    format!("503 limit ({MAX_ACTIVE_CONNS} active conns) from {peer_addr}"),
                                );
                            }
                        }
                        let _ = stream.write_all(
                            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                        continue;
                    }
                    metrics_bg.active_conns.fetch_add(1, Ordering::Relaxed);
                    metrics_bg.total_requests.fetch_add(1, Ordering::Relaxed);
                    let conn_metrics = Arc::clone(&metrics_bg);
                    let conn_log = log.clone();
                    let guard = ProxyConnGuard {
                        metrics: Some(Arc::clone(&metrics_bg)),
                    };
                    thread::spawn(move || {
                        let _guard = guard;
                        if let Err(err) = serve(stream, socks_port, conn_log.as_ref()) {
                            conn_metrics.record_error(&err);
                            if let Some(ref l) = conn_log {
                                if let Ok(mut g) = l.lock() {
                                    g.push_event("http", format!("ERROR: {err:#}"));
                                }
                            }
                        }
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    if stopped_bg.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
        });
        Ok(Self {
            port: bound_port,
            stopped,
            metrics,
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn metrics(&self) -> Arc<ProxyMetrics> {
        Arc::clone(&self.metrics)
    }
}

impl Drop for HttpProxy {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(100));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn send_error(client: &mut TcpStream, status: &str) {
    let _ = client.write_all(
        format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes(),
    );
    let _ = client.flush();
}

fn serve(
    mut client: TcpStream,
    socks_port: u16,
    log: Option<&Arc<Mutex<crate::tui::ProxyStderrLog>>>,
) -> Result<()> {
    let _ = client.set_nodelay(true);
    client.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut header = Vec::new();
    let mut byte = [0];
    while header.len() < 16 * 1024 && !header.ends_with(b"\r\n\r\n") {
        match client.read(&mut byte) {
            Ok(0) => return Ok(()),
            Ok(_) => {
                if header.is_empty() && (byte[0] == 4 || byte[0] == 5) {
                    anyhow::bail!(
                        "Received SOCKS{} handshake on HTTP proxy port (use http:// for this port, or use the SOCKS5 port for socks5h://)",
                        byte[0]
                    );
                }
                header.push(byte[0]);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) && header.is_empty() =>
            {
                // Idle preconnect socket closed without sending an HTTP request.
                return Ok(());
            }
            Err(e) => return Err(e).context("Reading HTTP request header from client"),
        }
    }
    if !header.ends_with(b"\r\n\r\n") {
        send_error(&mut client, "431 Request Header Fields Too Large");
        anyhow::bail!("HTTP headers too large");
    }
    let text = match std::str::from_utf8(&header) {
        Ok(t) => t,
        Err(err) => {
            send_error(&mut client, "400 Bad Request");
            return Err(err.into());
        }
    };
    let first = match text.lines().next() {
        Some(line) => line,
        None => {
            send_error(&mut client, "400 Bad Request");
            anyhow::bail!("Empty HTTP request");
        }
    };
    let mut parts = first.split_whitespace();
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        send_error(&mut client, "400 Bad Request");
        anyhow::bail!("Malformed HTTP request line");
    };
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) {
        send_error(&mut client, "400 Bad Request");
        anyhow::bail!("Invalid HTTP method");
    }
    if version != "HTTP/1.0" && version != "HTTP/1.1" {
        send_error(&mut client, "505 HTTP Version Not Supported");
        anyhow::bail!("Unsupported HTTP version");
    }

    let header_lines: Vec<&str> = text
        .split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .collect();

    let mut host_header: Option<&str> = None;
    let mut content_length: Option<u64> = None;
    let mut has_transfer_encoding = false;
    for line in &header_lines {
        if let Some((name, value)) = line.split_once(':') {
            let trimmed = value.trim();
            if name.eq_ignore_ascii_case("host") {
                host_header = Some(trimmed);
            } else if name.eq_ignore_ascii_case("content-length") {
                if let Ok(len) = trimmed.parse::<u64>() {
                    content_length = Some(len);
                }
            } else if name.eq_ignore_ascii_case("transfer-encoding") {
                has_transfer_encoding = true;
            }
        }
    }

    let connect = method.eq_ignore_ascii_case("CONNECT");
    let (authority, path) = if connect {
        (target, "")
    } else if let Some(url) = target.strip_prefix("http://") {
        let split = url.find(['/', '?']).unwrap_or(url.len());
        (
            &url[..split],
            if split == url.len() {
                "/"
            } else {
                &url[split..]
            },
        )
    } else if target.starts_with('/') {
        let Some(host_val) = host_header else {
            send_error(&mut client, "400 Bad Request");
            anyhow::bail!("Missing Host header for origin-form HTTP request");
        };
        (host_val, target)
    } else {
        send_error(&mut client, "400 Bad Request");
        anyhow::bail!("Only HTTP URLs or CONNECT are supported");
    };

    let (host, port) = match parse_authority(authority, if connect { 443 } else { 80 }) {
        Ok(parsed) => parsed,
        Err(err) => {
            send_error(&mut client, "400 Bad Request");
            return Err(err);
        }
    };

    let t0 = std::time::Instant::now();
    if let Some(l) = log {
        if let Ok(mut g) = l.lock() {
            g.push_event(
                "http",
                format!("{method} {host}:{port}{} -> SOCKS5 127.0.0.1:{socks_port}", if connect { String::new() } else { format!(" {path}") }),
            );
        }
    }

    let mut remote = match socks_connect(socks_port, host, port) {
        Ok(stream) => {
            if let Some(l) = log {
                if let Ok(mut g) = l.lock() {
                    g.push_event(
                        "socks",
                        format!("CONNECTED {host}:{port} ({} ms)", t0.elapsed().as_millis()),
                    );
                }
            }
            stream
        }
        Err(err) => {
            send_error(&mut client, "502 Bad Gateway");
            return Err(err);
        }
    };
    let _ = remote.set_nodelay(true);

    if connect {
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
        client.flush()?;
        client.set_read_timeout(None)?;
        let mut upstream = remote.try_clone()?;
        let mut downstream = client.try_clone()?;
        let upload = thread::spawn(move || {
            let _ = std::io::copy(&mut downstream, &mut upstream);
            let _ = upstream.shutdown(std::net::Shutdown::Write);
        });
        let _ = std::io::copy(&mut remote, &mut client);
        // Propagate remote EOF to the client immediately so HTTPS/TLS clients close cleanly.
        let _ = client.shutdown(std::net::Shutdown::Write);
        // Bound waiting for the client half-close so stalled peers cannot leak connection slots.
        let _ = client.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = upload.join();
        let _ = client.shutdown(std::net::Shutdown::Both);
        return Ok(());
    }

    let path = if path.starts_with('?') {
        format!("/{path}")
    } else {
        path.to_string()
    };
    remote.write_all(format!("{method} {path} {version}\r\n").as_bytes())?;
    for line in &header_lines {
        let lower = line.to_ascii_lowercase();
        if !lower.starts_with("proxy-connection:")
            && !lower.starts_with("proxy-authorization:")
            && !lower.starts_with("connection:")
        {
            remote.write_all(line.as_bytes())?;
            remote.write_all(b"\r\n")?;
        }
    }
    if host_header.is_none() {
        remote.write_all(format!("Host: {authority}\r\n").as_bytes())?;
    }
    // Single-request proxy mode: ask upstream to close after responding to prevent keep-alive desync.
    remote.write_all(b"Connection: close\r\n\r\n")?;
    remote.flush()?;

    client.set_read_timeout(None)?;
    let body_len = content_length.unwrap_or(0);
    if !has_transfer_encoding && body_len == 0 {
        // No request body (e.g. GET/HEAD/DELETE): stream response directly without blocking on client EOF.
        let _ = std::io::copy(&mut remote, &mut client);
        let _ = client.flush();
        let _ = client.shutdown(std::net::Shutdown::Both);
        return Ok(());
    }

    let mut upstream = remote.try_clone()?;
    let downstream = client.try_clone()?;
    let upload = thread::spawn(move || {
        if has_transfer_encoding {
            let mut reader = downstream;
            let _ = std::io::copy(&mut reader, &mut upstream);
        } else {
            let mut limited = downstream.take(body_len);
            let _ = std::io::copy(&mut limited, &mut upstream);
        }
        let _ = upstream.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut remote, &mut client);
    let _ = client.flush();
    let _ = client.shutdown(std::net::Shutdown::Write);
    let _ = client.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = upload.join();
    let _ = client.shutdown(std::net::Shutdown::Both);
    Ok(())
}

fn parse_authority(authority: &str, default_port: u16) -> Result<(&str, u16)> {
    let (host, port) = if authority.starts_with('[') {
        let end = authority.find(']').context("Invalid IPv6 address")?;
        let rest = &authority[end + 1..];
        anyhow::ensure!(
            rest.is_empty() || rest.starts_with(':'),
            "Invalid IPv6 authority"
        );
        (&authority[1..end], rest.strip_prefix(':'))
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    anyhow::ensure!(
        !host.is_empty()
            && !host.contains(['@', '/', '?', '#'])
            && !host
                .chars()
                .any(|c| c.is_control() || c.is_ascii_whitespace()),
        "Invalid proxy target"
    );
    let port = port.map(str::parse).transpose()?.unwrap_or(default_port);
    anyhow::ensure!(port != 0, "Invalid proxy port");
    Ok((host, port))
}

fn socks_reply_message(code: u8) -> &'static str {
    match code {
        1 => "general SOCKS server failure",
        2 => "connection not allowed by ruleset",
        3 => "network unreachable",
        4 => "host unreachable",
        5 => "connection refused",
        6 => "TTL expired",
        7 => "command not supported",
        8 => "address type not supported",
        _ => "unknown SOCKS error",
    }
}

pub fn test_socks_connect(socks_port: u16, host: &str, port: u16) -> Result<()> {
    let stream = socks_connect(socks_port, host, port)?;
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}

pub fn test_http_connect(http_port: u16, host: &str, port: u16) -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], http_port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .with_context(|| format!("Cannot reach local HTTP proxy 127.0.0.1:{http_port}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(8)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n").as_bytes())?;
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf)?;
    let text = String::from_utf8_lossy(&buf[..n]);
    let status_line = text.lines().next().unwrap_or("");
    anyhow::ensure!(
        status_line.contains(" 200 "),
        "HTTP proxy returned '{status_line}'"
    );
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}

fn socks_connect(socks_port: u16, host: &str, port: u16) -> Result<TcpStream> {
    let addr = SocketAddr::from(([127, 0, 0, 1], socks_port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .with_context(|| format!("Cannot reach local SOCKS5 listener 127.0.0.1:{socks_port}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    stream
        .write_all(&[5, 1, 0])
        .context("SOCKS5 greeting write failed")?;
    let mut response = [0; 2];
    stream
        .read_exact(&mut response)
        .context("SOCKS5 greeting read timed out or closed")?;
    anyhow::ensure!(response == [5, 0], "SOCKS authentication failed");
    anyhow::ensure!(host.len() <= 255, "Host name too long");
    let mut request = vec![5, 1, 0];
    if let Ok(ip4) = host.parse::<std::net::Ipv4Addr>() {
        request.push(1);
        request.extend_from_slice(&ip4.octets());
    } else if let Ok(ip6) = host.parse::<std::net::Ipv6Addr>() {
        request.push(4);
        request.extend_from_slice(&ip6.octets());
    } else {
        request.push(3);
        request.push(host.len() as u8);
        request.extend_from_slice(host.as_bytes());
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream
        .write_all(&request)
        .with_context(|| format!("SOCKS5 request write to {host}:{port} failed"))?;
    let mut reply = [0; 4];
    stream.read_exact(&mut reply).with_context(|| {
        format!("SOCKS5 connect to {host}:{port} timed out waiting for remote host")
    })?;
    anyhow::ensure!(
        reply[0] == 5 && reply[1] == 0,
        "SOCKS connect to {host}:{port} failed ({}): {}",
        reply[1],
        socks_reply_message(reply[1])
    );
    let len = match reply[3] {
        1 => 4,
        3 => {
            let mut n = [0];
            stream.read_exact(&mut n)?;
            n[0] as usize
        }
        4 => 16,
        _ => anyhow::bail!("Invalid SOCKS reply"),
    };
    let mut address = vec![0; len + 2];
    stream.read_exact(&mut address)?;
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(None)?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_tunnels_through_socks() {
        let socks = TcpListener::bind("127.0.0.1:0").unwrap();
        let socks_port = socks.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut peer, _) = socks.accept().unwrap();
            let mut greeting = [0; 3];
            peer.read_exact(&mut greeting).unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            peer.write_all(&[5, 0]).unwrap();
            let mut request = [0; 5];
            peer.read_exact(&mut request).unwrap();
            assert_eq!(request, [5, 1, 0, 3, 11]);
            let mut destination = [0; 13];
            peer.read_exact(&mut destination).unwrap();
            assert_eq!(&destination, b"example.org\x01\xbb");
            peer.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
            let mut payload = [0; 4];
            peer.read_exact(&mut payload).unwrap();
            assert_eq!(&payload, b"ping");
            peer.write_all(b"pong").unwrap();
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server =
            thread::spawn(move || serve(listener.accept().unwrap().0, socks_port, None).unwrap());
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .write_all(b"CONNECT example.org:443 HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut response = [0; 39];
        client.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"HTTP/1.1 200 Connection Established\r\n\r\n");
        client.write_all(b"ping").unwrap();
        let mut payload = [0; 4];
        client.read_exact(&mut payload).unwrap();
        assert_eq!(&payload, b"pong");
        client.shutdown(std::net::Shutdown::Write).unwrap();
        drop(client);
        server.join().unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn http_request_uses_remote_dns_and_origin_form() {
        let socks = TcpListener::bind("127.0.0.1:0").unwrap();
        let socks_port = socks.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut peer, _) = socks.accept().unwrap();
            let mut greeting = [0; 3];
            peer.read_exact(&mut greeting).unwrap();
            peer.write_all(&[5, 0]).unwrap();
            let mut request = [0; 5];
            peer.read_exact(&mut request).unwrap();
            assert_eq!(request, [5, 1, 0, 3, 11]);
            let mut destination = [0; 13];
            peer.read_exact(&mut destination).unwrap();
            assert_eq!(&destination, b"example.org\x00\x50");
            peer.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                peer.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = std::str::from_utf8(&request).unwrap();
            assert!(request.starts_with("GET /hello HTTP/1.1\r\n"));
            assert!(request.contains("Host: example.org\r\n"));
            assert!(!request.contains("Proxy-Authorization"));
            peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server =
            thread::spawn(move || serve(listener.accept().unwrap().0, socks_port, None).unwrap());
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.write_all(b"GET http://example.org/hello HTTP/1.1\r\nHost: example.org\r\nProxy-Authorization: secret\r\n\r\n").unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.contains("\r\n\r\nok"));
        server.join().unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn http_get_completes_without_client_half_close() {
        let socks = TcpListener::bind("127.0.0.1:0").unwrap();
        let socks_port = socks.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut peer, _) = socks.accept().unwrap();
            let mut greeting = [0; 3];
            peer.read_exact(&mut greeting).unwrap();
            peer.write_all(&[5, 0]).unwrap();
            let mut request = [0; 5];
            peer.read_exact(&mut request).unwrap();
            let mut destination = vec![0; request[4] as usize + 2];
            peer.read_exact(&mut destination).unwrap();
            peer.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
            let mut req = Vec::new();
            let mut byte = [0];
            while !req.ends_with(b"\r\n\r\n") {
                peer.read_exact(&mut byte).unwrap();
                req.push(byte[0]);
            }
            peer.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\ndone")
                .unwrap();
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server =
            thread::spawn(move || serve(listener.accept().unwrap().0, socks_port, None).unwrap());
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        // Real HTTP/1.1 clients (browsers, curl) do NOT half-close before reading the response.
        client
            .write_all(b"GET http://example.org/test HTTP/1.1\r\nHost: example.org\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.ends_with("\r\n\r\ndone"));
        server.join().unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn connect_propagates_eof_when_remote_closes_first() {
        let socks = TcpListener::bind("127.0.0.1:0").unwrap();
        let socks_port = socks.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut peer, _) = socks.accept().unwrap();
            let mut greeting = [0; 3];
            peer.read_exact(&mut greeting).unwrap();
            peer.write_all(&[5, 0]).unwrap();
            let mut request = [0; 5];
            peer.read_exact(&mut request).unwrap();
            let mut destination = vec![0; request[4] as usize + 2];
            peer.read_exact(&mut destination).unwrap();
            peer.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
            peer.write_all(b"hello").unwrap();
            // Remote closes immediately without waiting for client
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server =
            thread::spawn(move || serve(listener.accept().unwrap().0, socks_port, None).unwrap());
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client
            .write_all(b"CONNECT example.org:443 HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut header = [0; 39];
        client.read_exact(&mut header).unwrap();
        assert_eq!(&header, b"HTTP/1.1 200 Connection Established\r\n\r\n");
        let mut body = String::new();
        client.read_to_string(&mut body).unwrap();
        assert_eq!(body, "hello");
        drop(client);
        server.join().unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn http_proxy_starts_and_drops_promptly() {
        let proxy = HttpProxy::start(0, 1080).unwrap();
        assert_ne!(proxy.port(), 0);
        let started = std::time::Instant::now();
        drop(proxy);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn parses_http_targets() {
        assert_eq!(
            parse_authority("example.org", 80).unwrap(),
            ("example.org", 80)
        );
        assert_eq!(parse_authority("[::1]:8443", 443).unwrap(), ("::1", 8443));
        assert!(parse_authority("user@example.org", 80).is_err());
        assert!(parse_authority("example.org:0", 80).is_err());
    }

    #[test]
    fn ipv4_target_uses_socks_atyp_ipv4_and_logs_events() {
        let socks = TcpListener::bind("127.0.0.1:0").unwrap();
        let socks_port = socks.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let (mut peer, _) = socks.accept().unwrap();
            let mut greeting = [0; 3];
            peer.read_exact(&mut greeting).unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            peer.write_all(&[5, 0]).unwrap();
            let mut request = [0; 10];
            peer.read_exact(&mut request).unwrap();
            assert_eq!(request, [5, 1, 0, 1, 1, 1, 1, 1, 1, 0xbb]);
            peer.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
        });
        let log = Arc::new(Mutex::new(crate::tui::ProxyStderrLog::default()));
        let proxy = HttpProxy::start_with_log(0, socks_port, Some(Arc::clone(&log))).unwrap();
        test_http_connect(proxy.port(), "1.1.1.1", 443).unwrap();
        worker.join().unwrap();
        drop(proxy);
        let lines = log.lock().unwrap().snapshot();
        assert!(lines.iter().any(|l| l.contains("[http] CONNECT 1.1.1.1:443")));
        assert!(lines.iter().any(|l| l.contains("[socks] CONNECTED 1.1.1.1:443")));
    }
}
