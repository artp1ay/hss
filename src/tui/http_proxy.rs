use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct HttpProxy {
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl HttpProxy {
    pub fn start(port: u16, socks_port: u16) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .with_context(|| format!("Cannot bind HTTP proxy to 127.0.0.1:{port}"))?;
        listener.set_nonblocking(true)?;
        let (stop, rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            while rx.try_recv().is_err() {
                match listener.accept() {
                    Ok((stream, _)) => {
                        thread::spawn(move || {
                            let _ = serve(stream, socks_port);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for HttpProxy {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(mut client: TcpStream, socks_port: u16) -> Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut header = Vec::new();
    let mut byte = [0];
    while header.len() < 16 * 1024 && !header.ends_with(b"\r\n\r\n") {
        if client.read(&mut byte)? == 0 {
            return Ok(());
        }
        header.push(byte[0]);
    }
    anyhow::ensure!(header.ends_with(b"\r\n\r\n"), "HTTP headers too large");
    let text = std::str::from_utf8(&header)?;
    let first = text.lines().next().context("Empty HTTP request")?;
    let mut parts = first.split_whitespace();
    let method = parts.next().context("Missing HTTP method")?;
    let target = parts.next().context("Missing HTTP target")?;
    let version = parts.next().context("Missing HTTP version")?;
    let connect = method.eq_ignore_ascii_case("CONNECT");
    let (authority, path) = if connect {
        (target, "")
    } else {
        let url = target
            .strip_prefix("http://")
            .context("Only HTTP URLs or CONNECT are supported")?;
        let split = url.find(['/', '?']).unwrap_or(url.len());
        (
            &url[..split],
            if split == url.len() {
                "/"
            } else {
                &url[split..]
            },
        )
    };
    let (host, port) = parse_authority(authority, if connect { 443 } else { 80 })?;
    let mut remote = match socks_connect(socks_port, host, port) {
        Ok(stream) => stream,
        Err(err) => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
            return Err(err);
        }
    };
    if connect {
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    }
    if !connect {
        let path = if path.starts_with('?') {
            format!("/{path}")
        } else {
            path.to_string()
        };
        remote.write_all(format!("{method} {path} {version}\r\n").as_bytes())?;
        for line in text
            .split("\r\n")
            .skip(1)
            .take_while(|line| !line.is_empty())
        {
            if !line.to_ascii_lowercase().starts_with("proxy-connection:")
                && !line
                    .to_ascii_lowercase()
                    .starts_with("proxy-authorization:")
            {
                remote.write_all(line.as_bytes())?;
                remote.write_all(b"\r\n")?;
            }
        }
        remote.write_all(b"\r\n")?;
    }
    client.set_read_timeout(None)?;
    let mut upstream = remote.try_clone()?;
    let mut downstream = client.try_clone()?;
    let upload = thread::spawn(move || {
        let _ = std::io::copy(&mut downstream, &mut upstream);
        let _ = upstream.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut remote, &mut client);
    let _ = client.shutdown(std::net::Shutdown::Read);
    let _ = upload.join();
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
        !host.is_empty() && !host.contains(['@', '/', '?', '#', ' ']),
        "Invalid proxy target"
    );
    let port = port.map(str::parse).transpose()?.unwrap_or(default_port);
    anyhow::ensure!(port != 0, "Invalid proxy port");
    Ok((host, port))
}

fn socks_connect(socks_port: u16, host: &str, port: u16) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(("127.0.0.1", socks_port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(&[5, 1, 0])?;
    let mut response = [0; 2];
    stream.read_exact(&mut response)?;
    anyhow::ensure!(response == [5, 0], "SOCKS authentication failed");
    anyhow::ensure!(host.len() <= 255, "Host name too long");
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request)?;
    let mut reply = [0; 4];
    stream.read_exact(&mut reply)?;
    anyhow::ensure!(
        reply[0] == 5 && reply[1] == 0,
        "SOCKS connection failed: {}",
        reply[1]
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
            thread::spawn(move || serve(listener.accept().unwrap().0, socks_port).unwrap());
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
            thread::spawn(move || serve(listener.accept().unwrap().0, socks_port).unwrap());
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
    fn parses_http_targets() {
        assert_eq!(
            parse_authority("example.org", 80).unwrap(),
            ("example.org", 80)
        );
        assert_eq!(parse_authority("[::1]:8443", 443).unwrap(), ("::1", 8443));
        assert!(parse_authority("user@example.org", 80).is_err());
        assert!(parse_authority("example.org:0", 80).is_err());
    }
}
