//! In-process OTLP/HTTP collector on `127.0.0.1:0` that records every request as text.
//!
//! Include it with `#[path = "support/http_collector.rs"] mod http_collector;`.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex, PoisonError},
    thread,
};

use rustls::{ServerConfig, ServerConnection, StreamOwned};

/// Handle to a running mock; it serves until the test process exits.
pub struct HttpCollector {
    received: Arc<Mutex<Vec<u8>>>,
}

/// Start the mock and return its base URL and a handle. With `tls`, it serves HTTPS
/// (`https://127.0.0.1:<port>`); without, plain HTTP (`http://127.0.0.1:<port>`). It
/// answers the first `failures` requests with 503 and accepts every request after them.
pub fn start(tls: Option<Arc<ServerConfig>>, failures: usize) -> (String, HttpCollector) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind the mock collector");
    let scheme = if tls.is_some() { "https" } else { "http" };
    let base = format!(
        "{scheme}://{}",
        listener.local_addr().expect("mock collector address")
    );
    let received = Arc::new(Mutex::new(Vec::new()));
    let failures = Arc::new(Mutex::new(failures));

    let sink = Arc::clone(&received);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let sink = Arc::clone(&sink);
            let failures = Arc::clone(&failures);
            let tls = tls.clone();
            thread::spawn(move || match tls {
                None => serve(stream, &sink, &failures),
                // a rejected handshake fails the first read, so that connection records nothing
                Some(config) => {
                    if let Ok(session) = ServerConnection::new(config) {
                        serve(StreamOwned::new(session, stream), &sink, &failures);
                    }
                }
            });
        }
    });

    (base, HttpCollector { received })
}

impl HttpCollector {
    /// Every request so far (request line, headers, and body) as lossy UTF-8, rejected ones
    /// included. Protobuf payloads keep span names, log bodies, and metric names as plain
    /// UTF-8 runs.
    pub fn received_text(&self) -> String {
        let received = self.received.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&received).into_owned()
    }
}

// one keep-alive connection: record each request before answering, so an export that
// returned has already been recorded
fn serve(stream: impl Read + Write, sink: &Mutex<Vec<u8>>, failures: &Mutex<usize>) {
    let mut reader = BufReader::new(stream);

    while let Some(request) = read_request(&mut reader) {
        sink.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(&request);
        let fail = {
            let mut left = failures.lock().unwrap_or_else(PoisonError::into_inner);
            let fail = *left > 0;
            *left = left.saturating_sub(1);
            fail
        };
        let answer: &[u8] = if fail {
            b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n"
        } else {
            b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\n\r\n"
        };
        // a request is read in full before the reply, so nothing buffered is skipped
        let reply = reader.get_mut();
        if reply
            .write_all(answer)
            .and_then(|()| reply.flush())
            .is_err()
        {
            return;
        }
    }
}

// one request with a content-length body; None once the client closes the connection
fn read_request(reader: &mut impl BufRead) -> Option<Vec<u8>> {
    let mut request = Vec::new();
    let mut body_len = 0;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            body_len = value.trim().parse().expect("numeric content-length");
        }
        request.extend_from_slice(line.as_bytes());
        if line == "\r\n" {
            break;
        }
    }

    let start = request.len();
    request.resize(start + body_len, 0);
    reader.read_exact(&mut request[start..]).ok()?;
    Some(request)
}
