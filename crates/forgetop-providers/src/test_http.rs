//! A fake forge for provider tests: a real HTTP/1.1 listener on a loopback port that answers
//! each request path with a canned JSON body and records every request line it saw.
//!
//! Hand-rolled on `std::net` so the tests need no mock-server crate: a provider test wants to
//! know *what the client asked for* (the exact path and query), and that is one request line.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::Value;

pub struct FakeForge {
    base: String,
    routes: Arc<Mutex<HashMap<String, String>>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeForge {
    /// Binds a free loopback port and starts answering. Add bodies with [`route`](Self::route);
    /// a path with none is a `404` with an empty object.
    pub fn start() -> FakeForge {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: Arc<Mutex<HashMap<String, String>>> = Arc::default();
        let requests: Arc<Mutex<Vec<String>>> = Arc::default();
        let (served, log) = (routes.clone(), requests.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut chunk = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&head);
                let mut line = head.lines().next().unwrap_or("").split(' ');
                let (method, target) = (line.next().unwrap_or(""), line.next().unwrap_or(""));
                log.lock().unwrap().push(format!("{method} {target}"));
                let path = target.split('?').next().unwrap_or("");
                let body = served.lock().unwrap().get(path).cloned();
                let (status, body) = match body {
                    Some(body) => ("200 OK", body),
                    None => ("404 Not Found", "{}".to_string()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.flush();
            }
        });
        FakeForge { base, routes, requests }
    }

    /// `http://127.0.0.1:<port>` — what a client's `base` should be.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Answers `path` (as it appears on the request line, percent-encoded, without the query)
    /// with `body` from now on.
    pub fn route(&self, path: &str, body: Value) -> &Self {
        self.routes.lock().unwrap().insert(path.to_string(), body.to_string());
        self
    }

    /// Every request line so far, oldest first: `GET /path?query`.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    /// The request lines whose path starts with `prefix`.
    pub fn requests_to(&self, prefix: &str) -> Vec<String> {
        let prefix = format!("GET {prefix}");
        self.requests().into_iter().filter(|r| r.starts_with(&prefix)).collect()
    }
}
