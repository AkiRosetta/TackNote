//! A small in-memory WebDAV server for tests. It behaves like a real one for
//! the things the app relies on: auth, folders, ETags, If-Match and
//! If-None-Match. Plain HTTP/1.1, one request per connection.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

/// "me:secret" in Basic auth.
const EXPECTED_AUTH: &str = "Basic bWU6c2VjcmV0";

type Hook = Box<dyn FnOnce(&mut State) + Send>;

#[derive(Default)]
pub struct State {
    files: HashMap<String, (String, u64)>,
    folders: HashSet<String>,
    log: Vec<String>,
    version: u64,
    ignore_preconditions: bool,
    like_jianguoyun: bool,
    before_next_put: Option<Hook>,
}

impl State {
    pub fn set_file(&mut self, path: &str, body: &str) {
        self.version += 1;
        self.files.insert(path.to_string(), (body.to_string(), self.version));
    }
}

pub struct FakeWebDav {
    port: u16,
    state: Arc<Mutex<State>>,
}

impl FakeWebDav {
    pub fn start() -> FakeWebDav {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        state.lock().unwrap().folders.insert("/".into());
        let shared = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = shared.clone();
                thread::spawn(move || handle(stream, &state));
            }
        });
        FakeWebDav { port, state }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    pub fn file(&self, path: &str) -> Option<String> {
        self.state.lock().unwrap().files.get(path).map(|(body, _)| body.clone())
    }

    pub fn set_file(&self, path: &str, body: &str) {
        self.state.lock().unwrap().set_file(path, body);
    }

    pub fn add_folder(&self, path: &str) {
        self.state.lock().unwrap().folders.insert(path.into());
    }

    /// Every file and folder there is.
    pub fn paths(&self) -> Vec<String> {
        let state = self.state.lock().unwrap();
        state.files.keys().chain(state.folders.iter()).cloned().collect()
    }

    pub fn log(&self) -> Vec<String> {
        self.state.lock().unwrap().log.clone()
    }

    pub fn clear_log(&self) {
        self.state.lock().unwrap().log.clear();
    }

    pub fn set_honor_preconditions(&self, honor: bool) {
        self.state.lock().unwrap().ignore_preconditions = !honor;
    }

    /// Answers the way Jianguoyun did (2026-11): 409 for a file in a missing
    /// folder, 201 for a folder that exists, 403 for deleting a top-level
    /// folder, and If-None-Match ignored while If-Match is honoured.
    pub fn answer_like_jianguoyun(&self) {
        self.state.lock().unwrap().like_jianguoyun = true;
    }

    /// Runs once, just before the next PUT is handled.
    pub fn before_next_put(&self, hook: impl FnOnce(&mut State) + Send + 'static) {
        self.state.lock().unwrap().before_next_put = Some(Box::new(hook));
    }
}

fn handle(stream: TcpStream, state: &Mutex<State>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());

    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let length: usize = headers.get("content-length").and_then(|value| value.parse().ok()).unwrap_or(0);
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let body = String::from_utf8_lossy(&body).into_owned();

    let (status, etag, response_body) = respond(state, &method, &path, &headers, body);
    let mut out = stream;
    let etag_line = etag.map(|tag| format!("ETag: \"{tag}\"\r\n")).unwrap_or_default();
    let _ = write!(
        out,
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n{etag_line}\r\n{response_body}",
        response_body.len()
    );
}

fn respond(state: &Mutex<State>, method: &str, path: &str, headers: &HashMap<String, String>, body: String) -> (u16, Option<u64>, String) {
    let hook = {
        let mut state = state.lock().unwrap();
        state.log.push(format!("{method} {path}"));
        if headers.get("authorization").map(String::as_str) != Some(EXPECTED_AUTH) {
            return (401, None, String::new());
        }
        if method == "PUT" { state.before_next_put.take() } else { None }
    };
    let mut state = state.lock().unwrap();
    if let Some(hook) = hook {
        hook(&mut state);
    }
    let parent = &path[..path.trim_end_matches('/').rfind('/').map_or(0, |index| index + 1)];
    match method {
        "GET" => match state.files.get(path) {
            Some((content, version)) => (200, Some(*version), content.clone()),
            None if state.like_jianguoyun && !state.folders.contains(parent) => (409, None, String::new()),
            None => (404, None, String::new()),
        },
        "MKCOL" => {
            let created = state.folders.insert(path.to_string());
            (if created || state.like_jianguoyun { 201 } else { 405 }, None, String::new())
        }
        "PUT" => {
            if !state.folders.contains(parent) {
                return (409, None, String::new());
            }
            let current = state.files.get(path).map(|(_, version)| format!("\"{version}\""));
            let if_match = headers.get("if-match");
            let if_none_match = headers.get("if-none-match");
            let blocked = !state.ignore_preconditions
                && ((if_match.is_some() && if_match != current.as_ref())
                    || (!state.like_jianguoyun && if_none_match.map(String::as_str) == Some("*") && current.is_some()));
            if blocked {
                return (412, None, String::new());
            }
            state.set_file(path, &body);
            (201, None, String::new())
        }
        "DELETE" => {
            if state.like_jianguoyun && parent == "/" && path.ends_with('/') {
                return (403, None, String::new());
            }
            let existed = state.files.remove(path).is_some() | state.folders.remove(path);
            if path.ends_with('/') {
                state.files.retain(|file, _| !file.starts_with(path));
                state.folders.retain(|folder| !folder.starts_with(path));
            }
            (if existed { 204 } else { 404 }, None, String::new())
        }
        _ => (405, None, String::new()),
    }
}
