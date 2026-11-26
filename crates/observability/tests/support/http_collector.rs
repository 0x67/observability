//! In-process OTLP/HTTP collector on `127.0.0.1:0` that records every request as text.
//!
//! Include it with `#[path = "support/http_collector.rs"] mod http_collector;`.

use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, PoisonError},
    thread,
};

/// Handle to a running mock; it serves until the test process exits.
pub struct HttpCollector {
    received: Arc<Mutex<Vec<u8>>>,
}

/// Start the mock and return its base URL (`http://127.0.0.1:<port>`) and a handle.
pub fn start() -> (String, HttpCollector) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind the mock collector");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("mock collector address")
    );
    let received = Arc::new(Mutex::new(Vec::new()));

    let sink = Arc::clone(&received);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let sink = Arc::clone(&sink);
            thread::spawn(move || serve(stream, &sink));
        }
    });

    (base, HttpCollector { received })
}

impl HttpCollector {
    /// Every request so far (request line, headers, and body) as lossy UTF-8. Protobuf
    /// payloads keep span names, log bodies, and metric names as plain UTF-8 runs.
    pub fn received_text(&self) -> String {
        let received = self.received.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&received).into_owned()
    }
}

// one keep-alive connection: record each request before answering 200, so an export that
// returned has already been recorded
fn serve(stream: TcpStream, sink: &Mutex<Vec<u8>>) {
    let mut reply = stream.try_clone().expect("clone the mock stream");
    let mut reader = BufReader::new(stream);

    while let Some(request) = read_request(&mut reader) {
        sink.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(&request);
        let ok =
            b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\n\r\n";
        if reply.write_all(ok).is_err() {
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
