//! The sync server's HTTP side, kept thin: each request is read into `answer`, which routes it to the
//! hub (`sync_api.rs` has the table of paths). Every path but `/api/health` needs the token as
//! `Authorization: Bearer <token>`, compared in constant time. Each request has a thread of its own (a
//! log request waits up to `MAX_WAIT` seconds), at most `MAX_THREADS` at once.

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::{de::DeserializeOwned, Serialize};
use tiny_http::{Header, Method, Response, Server};

use crate::hub::Hub;
use crate::proto::Request;
use crate::sync_api::{DeviceReport, Failure, Posted, Transport, MAX_WAIT};

/// The largest body taken.
pub const MAX_BODY: usize = 32 * 1024 * 1024;
/// The most requests handled at once; more are told to come back (503).
pub const MAX_THREADS: usize = 256;

pub struct Api {
    hub: Arc<Hub>,
    token: String,
    busy: AtomicUsize,
}

impl Api {
    pub fn new(hub: Arc<Hub>, token: &str) -> Api {
        Api { hub, token: token.to_string(), busy: AtomicUsize::new(0) }
    }
}

/// An answer: its status and its body (JSON, but for `/api/health`'s `ok` and a 204's nothing).
#[derive(Debug, PartialEq)]
pub struct Answer {
    pub status: u16,
    pub body: String,
}

fn json<T: Serialize>(value: &T) -> Answer {
    match serde_json::to_string(value) {
        Ok(body) => Answer { status: 200, body },
        Err(e) => error(500, &e.to_string()),
    }
}

fn error(status: u16, why: &str) -> Answer {
    Answer { status, body: serde_json::json!({ "error": why }).to_string() }
}

fn failed(failure: Failure) -> Answer {
    match failure {
        Failure::Gone => error(410, &failure.words()),
        Failure::Unauthorized => error(401, &failure.words()),
        Failure::Other(why) => error(400, &why),
    }
}

fn read<T: DeserializeOwned>(body: &[u8]) -> Result<T, Answer> {
    serde_json::from_slice(body).map_err(|e| error(400, &format!("The body cannot be read: {e}")))
}

/// Whether `given` is `wanted`, in a time that does not tell how much of it was right.
fn same(given: &[u8], wanted: &[u8]) -> bool {
    let mut differ = given.len() ^ wanted.len();
    for i in 0..given.len().max(wanted.len()) {
        differ |= usize::from(given.get(i).copied().unwrap_or(0) ^ wanted.get(i).copied().unwrap_or(0));
    }
    differ == 0
}

/// The value of `name` in a query string (`after=3&wait=25`).
fn query<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| pair.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')))
}

/// The answer to one request: `method` and `url` (path and query), its `Authorization` header and body.
pub fn answer(api: &Api, method: &Method, url: &str, authorization: Option<&str>, body: &[u8]) -> Answer {
    let (path, query_string) = url.split_once('?').unwrap_or((url, ""));
    if path == "/api/health" {
        return Answer { status: 200, body: "ok".into() };
    }
    let token = authorization.and_then(|value| value.trim().strip_prefix("Bearer ")).unwrap_or("");
    if !same(token.trim().as_bytes(), api.token.as_bytes()) {
        return failed(Failure::Unauthorized);
    }
    let hub = api.hub.as_ref();
    let done = |result: Result<Answer, Answer>| result.unwrap_or_else(|answer| answer);
    match (method, path) {
        (Method::Get, "/api/snapshot") => hub.snapshot().map_or_else(failed, |snapshot| json(&snapshot)),
        (Method::Get, "/api/log") => {
            let number = |name: &str| {
                query(query_string, name).map(|value| value.parse::<u64>().map_err(|_| error(400, &format!("{name} is not a number"))))
            };
            done((|| {
                let after = number("after").unwrap_or(Ok(0))?;
                let wait = number("wait").unwrap_or(Ok(0))?.min(MAX_WAIT);
                Ok(hub.log(after, Duration::from_secs(wait)).map_or_else(failed, |page| json(&page)))
            })())
        }
        (Method::Post, "/api/log") => {
            done(read::<Posted>(body).map(|posted| hub.post(posted).map_or_else(failed, |appended| json(&appended))))
        }
        (Method::Post, "/api/request") => {
            done(read::<Request>(body).map(|request| hub.request(&request).map_or_else(failed, |response| json(&response))))
        }
        (Method::Get, "/api/devices") => hub.devices().map_or_else(failed, |devices| json(&devices)),
        (Method::Put, path) if path.starts_with("/api/devices/") => {
            let device = &path["/api/devices/".len()..];
            done(read::<DeviceReport>(body).map(|report| match hub.report(device, report) {
                Ok(()) => Answer { status: 204, body: String::new() },
                Err(failure) => failed(failure),
            }))
        }
        (_, "/api/snapshot" | "/api/log" | "/api/request" | "/api/devices") => error(405, "Not allowed for this path"),
        _ => error(404, "No such path"),
    }
}

pub fn listen(address: &str) -> Result<Server, String> {
    Server::http(address).map_err(|e| e.to_string())
}

/// Serves requests until the server is dropped, each on a thread of its own.
pub fn serve(api: Arc<Api>, server: Server) {
    for request in server.incoming_requests() {
        if api.busy.fetch_add(1, Ordering::SeqCst) >= MAX_THREADS {
            api.busy.fetch_sub(1, Ordering::SeqCst);
            let _ = request.respond(Response::from_string("busy").with_status_code(503));
            continue;
        }
        let api = api.clone();
        let spawned = thread::Builder::new().name("request".into()).spawn({
            let api = api.clone();
            move || {
                respond(&api, request);
                api.busy.fetch_sub(1, Ordering::SeqCst);
            }
        });
        if spawned.is_err() {
            api.busy.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn respond(api: &Api, mut request: tiny_http::Request) {
    let authorization = request.headers().iter().find(|h| h.field.equiv("Authorization")).map(|h| h.value.as_str().to_string());
    let mut body = Vec::new();
    let read = request.as_reader().take(MAX_BODY as u64 + 1).read_to_end(&mut body);
    let answer = match read {
        Err(e) => error(400, &e.to_string()),
        Ok(_) if body.len() > MAX_BODY => error(413, "The body is too large"),
        Ok(_) => answer(api, request.method(), request.url(), authorization.as_deref(), &body),
    };
    let kind = if answer.status == 200 && request.url().starts_with("/api/health") { "text/plain" } else { "application/json" };
    let header = Header::from_bytes("Content-Type", kind).expect("a valid header");
    let _ = request.respond(Response::from_string(answer.body).with_status_code(answer.status).with_header(header));
}

#[cfg(test)]
mod tests;
