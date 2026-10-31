use super::*;
use crate::definitions::tests::{session, target};
use crate::definitions::Definition;
use crate::handler::{handle, Context};
use crate::hub::{self, tests::context};
use crate::lock::Lock;
use crate::proto::Response as Answered;
use crate::replica::{Action, Entry};
use crate::sync_api::{Http, LogPage, Snapshot, SyncStatus};
use crate::syncing::{Connector, Syncer};
use std::time::{Instant, SystemTime};

const TOKEN: &str = "0123456789abcdef";

fn api(name: &str) -> Api {
    let dir = hub::tests::dir(&format!("http-{name}"));
    Api::new(Arc::new(Hub::new(context(&dir), Some(dir))), TOKEN)
}

fn bearer() -> Option<&'static str> {
    Some("Bearer 0123456789abcdef")
}

fn entry(action: Action) -> String {
    Entry { id: crate::replica::new_id(), device: "desk".into(), at: crate::state::to_unix(SystemTime::now()).unwrap(), action }.text()
}

#[test]
fn every_path_but_health_needs_the_token() {
    let api = api("token");
    assert_eq!(answer(&api, &Method::Get, "/api/health", None, b""), Answer { status: 200, body: "ok".into() });
    for authorization in [None, Some("Bearer wrong"), Some("0123456789abcdef"), Some("Bearer 0123456789abcdefg"), Some("Bearer ")] {
        assert_eq!(answer(&api, &Method::Get, "/api/snapshot", authorization, b"").status, 401, "{authorization:?}");
    }
    assert_eq!(answer(&api, &Method::Get, "/api/snapshot", Some(" Bearer 0123456789abcdef "), b"").status, 200);
    assert!(same(b"abc", b"abc"));
    assert!(!same(b"abc", b"abd") && !same(b"abc", b"ab") && !same(b"", b"a"));
}

#[test]
fn the_paths_answer_as_the_api_says() {
    let api = api("paths");
    let ask = |method: Method, url: &str, body: &str| answer(&api, &method, url, bearer(), body.as_bytes());
    let snapshot: Snapshot = serde_json::from_str(&ask(Method::Get, "/api/snapshot", "").body).unwrap();
    assert_eq!(snapshot.head, 0);
    // A computer's entries, then the log.
    let posted = serde_json::json!({ "entries": [entry(Action::SaveTarget { target: target("social", &["a.com"]) })] }).to_string();
    assert_eq!(ask(Method::Post, "/api/log", &posted).body, r#"{"head":1}"#);
    let page: LogPage = serde_json::from_str(&ask(Method::Get, "/api/log?after=0&wait=0", "").body).unwrap();
    assert_eq!((page.head, page.entries.len()), (1, 1));
    assert_eq!(ask(Method::Get, "/api/log", "").status, 200);
    assert_eq!(ask(Method::Get, "/api/log?after=x", "").status, 400);
    assert_eq!(ask(Method::Get, "/api/log?after=0&wait=-1", "").status, 400);
    assert_eq!(ask(Method::Post, "/api/log", "garbage").status, 400);
    let too_many = serde_json::json!({ "entries": vec![""; crate::sync_api::MAX_POST + 1] }).to_string();
    assert_eq!(ask(Method::Post, "/api/log", &too_many).status, 400);
    // A request, under the rules of a computer.
    let start = serde_json::to_string(&Request::Start { id: "nothing".into() }).unwrap();
    let answered: Answered = serde_json::from_str(&ask(Method::Post, "/api/request", &start).body).unwrap();
    assert!(matches!(answered, Answered::Failed(_)));
    let status: Answered = serde_json::from_str(&ask(Method::Post, "/api/request", r#"{"cmd":"status"}"#).body).unwrap();
    assert!(matches!(status, Answered::Status(_)));
    assert_eq!(ask(Method::Post, "/api/request", "{}").status, 400);
    // Computers.
    let report = r#"{"name":"Desk","confirmed":1,"held":[],"running":[]}"#;
    assert_eq!(ask(Method::Put, "/api/devices/d1", report), Answer { status: 204, body: String::new() });
    assert_eq!(ask(Method::Put, "/api/devices/", report).status, 400);
    assert_eq!(ask(Method::Put, "/api/devices/d1", "{").status, 400);
    assert!(ask(Method::Get, "/api/devices", "").body.contains(r#""name":"Desk""#));
    assert_eq!(ask(Method::Delete, "/api/devices", "").status, 405);
    assert_eq!(ask(Method::Get, "/api/request", "").status, 405);
    assert_eq!(ask(Method::Get, "/elsewhere", "").status, 404);
}

/// The server on a port of this computer's, serving on threads of its own; its address.
fn started(name: &str) -> (String, Arc<Hub>) {
    let dir = hub::tests::dir(&format!("http-{name}"));
    let hub = Arc::new(Hub::new(context(&dir), Some(dir)));
    let server = listen("127.0.0.1:0").unwrap();
    let address = format!("http://{}", server.server_addr().to_ip().unwrap());
    let api = Arc::new(Api::new(hub.clone(), TOKEN));
    thread::spawn(move || serve(api, server));
    (address, hub)
}

#[test]
fn the_apps_own_client_talks_to_it_over_http() {
    let (address, hub) = started("client");
    let http = Http::new(&address, TOKEN).unwrap();
    assert_eq!(http.snapshot().unwrap().head, 0);
    assert!(matches!(Http::new(&address, "a wrong token, long enough").unwrap().snapshot(), Err(Failure::Unauthorized)));
    // A log request waits for an entry: on a thread of its own, while another request is answered.
    let waiting = {
        let http = Http::new(&address, TOKEN).unwrap();
        thread::spawn(move || http.log(0, None, Duration::from_secs(10)).unwrap())
    };
    thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    http.post(Posted { entries: vec![entry(Action::AskForUnlock)], ..Posted::default() }).unwrap();
    assert_eq!(waiting.join().unwrap().entries.len(), 1);
    assert!(started.elapsed() < Duration::from_secs(5));
    // Requests, reports.
    assert_eq!(http.request(&Request::SaveTarget { target: target("social", &["a.com"]) }).unwrap(), Answered::Done);
    http.report("d1", DeviceReport { name: "Desk".into(), ..DeviceReport::default() }).unwrap();
    assert_eq!(http.devices().unwrap()[0].report.name, "Desk");
    assert_eq!(hub.head(), 2);
    // A computer further than this log goes, or at another entry: not the log it synced with (409).
    assert!(matches!(http.log(9, None, Duration::ZERO), Err(Failure::Diverged)));
    assert!(matches!(http.log(1, Some("0123abcd"), Duration::ZERO), Err(Failure::Diverged)));
    let posted = Posted { entries: vec![entry(Action::AskForUnlock)], after: Some(1), last: Some("0123abcd".into()) };
    assert!(matches!(http.post(posted), Err(Failure::Diverged)));
    assert!(http.log(1, Some("not an id!"), Duration::ZERO).is_ok(), "what is not an id is not sent");
    // A body over the limit is refused.
    let big =
        ureq::post(&format!("{address}/api/log")).set("Authorization", "Bearer 0123456789abcdef").send_bytes(&vec![b' '; MAX_BODY + 1]);
    assert!(matches!(big, Err(ureq::Error::Status(413, _))));
    let health = ureq::get(&format!("{address}/api/health")).call().unwrap();
    assert_eq!(health.into_string().unwrap(), "ok");
}

#[test]
fn a_computer_missing_too_much_is_told_so_over_http() {
    let (address, _) = started("gone");
    let entries: Vec<String> = (0..hub::KEEP + 5).map(|_| entry(Action::AskToStop { id: "x".into() })).collect();
    let http = Http::new(&address, TOKEN).unwrap();
    for chunk in entries.chunks(crate::sync_api::MAX_POST) {
        http.post(Posted { entries: chunk.to_vec(), ..Posted::default() }).unwrap();
    }
    assert!(matches!(http.log(0, None, Duration::ZERO), Err(Failure::Gone)));
}

/// A computer syncing with the server over HTTP.
struct Computer {
    context: Arc<Context>,
    syncer: Arc<Syncer>,
}

fn computer(name: &str) -> Computer {
    let dir = hub::tests::dir(&format!("http-{name}"));
    let connector: Connector = Box::new(|url, token| Ok(Arc::new(Http::new(url, token)?) as Arc<dyn Transport>));
    let syncer = Arc::new(Syncer::at(Some(dir.clone()), connector, name.into()));
    Computer { context: Arc::new(context(&dir).with_syncer(syncer.clone())), syncer }
}

impl Computer {
    fn ask(&self, request: Request) -> Answered {
        handle(&self.context, 0, request).0
    }

    fn runs(&self, id: &str) -> bool {
        self.context.sessions().running_ids().iter().any(|r| r == id)
    }

    fn status(&self) -> SyncStatus {
        match self.ask(Request::SyncStatus) {
            Answered::Sync(status) => *status,
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn two_computers_sync_over_http_and_each_can_always_get_out() {
    let (address, hub) = started("computers");
    let http = Http::new(&address, TOKEN).unwrap();
    http.request(&Request::SaveTarget { target: target("social", &["a.com"]) }).unwrap();
    http.request(&Request::SaveSession { session: Definition { lock: Lock::UntilRestart, ..session("work", &["social"]) } }).unwrap();
    let (a, b) = (computer("http-a"), computer("http-b"));
    let connect = |c: &Computer| c.ask(Request::SyncConnect { url: address.clone(), token: TOKEN.into() });
    assert_eq!(connect(&a), Answered::Done);
    assert_eq!(connect(&b), Answered::Done);
    // Started on A, and with the sync threads running, B has it soon.
    a.syncer.spawn(a.context.clone());
    b.syncer.spawn(b.context.clone());
    assert_eq!(a.ask(Request::Start { id: "work".into() }), Answered::Done);
    let started = Instant::now();
    while !b.runs("work") && started.elapsed() < Duration::from_secs(10) {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(b.runs("work"));
    assert_eq!(crate::sync_api::Transport::snapshot(hub.as_ref()).unwrap().runs.len(), 1);
    // Locked everywhere, the server's API too.
    assert!(matches!(http.request(&Request::Stop { id: "work".into(), password: None, text: None }).unwrap(), Answered::Failed(_)));
    assert!(matches!(b.ask(Request::Stop { id: "work".into(), password: None, text: None }), Answered::Failed(_)));
    // Disconnecting always works, and the way out is there.
    assert_eq!(b.ask(Request::SyncDisconnect), Answered::Done);
    assert!(!b.status().connected);
    b.ask(Request::AskForUnlock);
    b.context.sessions().move_to(b.context.sessions().now() + Duration::from_secs(crate::unlock::MAX_DELAY + 1));
    assert_eq!(b.ask(Request::Unlock), Answered::Done);
    assert!(!b.runs("work"));
    // A, still connected, is not touched by what B did after it left.
    thread::sleep(Duration::from_millis(300));
    assert!(a.runs("work"));
}

#[test]
fn the_windows_commands_are_answered_as_the_app_answers_them() {
    let api = api("invoke");
    let invoke = |body: serde_json::Value| answer(&api, &Method::Post, "/api/invoke", bearer(), body.to_string().as_bytes());
    let saved = invoke(serde_json::json!({ "cmd": "save_target", "args": { "target": target("social", &["a.com"]) } }));
    assert_eq!(saved, Answer { status: 200, body: "null".into() });
    // It went into the log, as a request does.
    assert_eq!(api.hub.head(), 1);
    let status = invoke(serde_json::json!({ "cmd": "status", "zone": 120 }));
    let overview: crate::proto::Overview = serde_json::from_str(&status.body).unwrap();
    assert_eq!(overview.targets.len(), 1);
    // A refusal is `{"error": ...}`, as are a computer's own commands and arguments that cannot be read.
    let refused = invoke(serde_json::json!({ "cmd": "start_session", "args": { "id": "nothing" } }));
    assert_eq!(refused.status, 400);
    assert!(refused.body.starts_with(r#"{"error":"#));
    let drives = invoke(serde_json::json!({ "cmd": "connected_drives" }));
    assert_eq!(drives.body, serde_json::json!({ "error": crate::strings::t("web.computerOnly", &[]) }).to_string());
    assert_eq!(answer(&api, &Method::Post, "/api/invoke", bearer(), b"garbage").status, 400);
    assert_eq!(answer(&api, &Method::Post, "/api/invoke", None, b"{}").status, 401);
    assert_eq!(answer(&api, &Method::Get, "/api/invoke", bearer(), b"").status, 405);
}

#[test]
fn the_windows_page_is_served_without_the_token_and_nothing_else() {
    let page = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src");
    let (kind, bytes) = file(&page, "/").unwrap();
    assert_eq!(kind, "text/html; charset=utf-8");
    assert!(String::from_utf8(bytes).unwrap().contains("js/web.js"));
    assert_eq!(file(&page, "/js/web.js?v=1").unwrap().0, "text/javascript; charset=utf-8");
    assert_eq!(file(&page, "/app.css").unwrap().0, "text/css; charset=utf-8");
    assert_eq!(file(&page, "/vendor/carbon.js").unwrap().0, "text/javascript; charset=utf-8");
    // A phone's home screen: the manifest and the icons, whatever `?v=` asks iOS to fetch them again.
    assert_eq!(file(&page, "/manifest.webmanifest?v=1").unwrap().0, "application/manifest+json");
    assert_eq!(file(&page, "/icons/icon-180.png?v=1").unwrap().0, "image/png");
    for url in [
        "/../server/Cargo.toml",
        "/js/../index.html",
        "/.git/config",
        "//etc/passwd",
        "/api/invoke",
        "/api",
        "/nothing.js",
        "/README",
        "/js",
    ] {
        assert!(file(&page, url).is_none(), "{url}");
    }
    let fonts = page.join("vendor/fonts");
    if let Some(font) = std::fs::read_dir(&fonts).ok().and_then(|mut d| d.find_map(|e| e.ok())).map(|e| e.file_name()) {
        let name = font.to_string_lossy().into_owned();
        if name.ends_with(".woff2") {
            assert_eq!(file(&page, &format!("/vendor/fonts/{name}")).unwrap().0, "font/woff2");
        }
    }

    // Over HTTP: the page with no token, the API still with it.
    let dir = hub::tests::dir("http-page");
    let api = Arc::new(Api::new(Arc::new(Hub::new(context(&dir), Some(dir))), TOKEN).with_page(page));
    let server = listen("127.0.0.1:0").unwrap();
    let address = format!("http://{}", server.server_addr().to_ip().unwrap());
    thread::spawn(move || serve(api, server));
    let index = ureq::get(&format!("{address}/")).call().unwrap();
    assert_eq!(index.header("Content-Type"), Some("text/html; charset=utf-8"));
    assert_eq!(index.header("Cache-Control"), Some("no-cache"));
    assert!(index.into_string().unwrap().contains("<html"));
    assert!(matches!(ureq::get(&format!("{address}/nothing.html")).call(), Err(ureq::Error::Status(404, _))));
    assert!(matches!(ureq::post(&format!("{address}/api/invoke")).send_string("{}"), Err(ureq::Error::Status(401, _))));
}
