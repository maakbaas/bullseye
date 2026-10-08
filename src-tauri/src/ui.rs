//! The window: made when it is asked for, hidden (not closed) when it is dismissed so that it
//! comes back at once, and destroyed once it has stayed hidden for a while.
//!
//! The web view behind the window costs about 300 MB and half a dozen extra processes, and the
//! app is only looked at now and then; the blocking never needs the window. So the app starts in
//! the tray with no window at all, and gives the memory back after a quiet spell. A window that is
//! looked at in bursts (check, start a block, look again a minute later) reopens instantly; the
//! first opening after a long pause takes about half a second.

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window};

use crate::sync::{spawn_critical, LockExt};
use crate::watchdog::note;

const LABEL: &str = "main";
/// The page is responsive (Carbon's grid), so the window can be any size from the minimum up.
const WIDTH: f64 = 760.0;
const HEIGHT: f64 = 720.0;
/// Wide enough for an editor's Delete, Cancel and Save to stay on one row (they wrap below 436 px
/// with a 15 px scrollbar, measured; Windows' is 17), the scrollbar's room included
/// (`scrollbar-gutter: stable`).
const MIN_WIDTH: f64 = 440.0;
const MIN_HEIGHT: f64 = 480.0;

/// How long a dismissed window is kept before it is destroyed.
const KEEP_HIDDEN: Duration = Duration::from_secs(5 * 60);

/// Whether, and since when, the window is hidden.
pub struct Ui {
    hidden_since: Mutex<Option<Instant>>,
    keep_hidden: Duration,
}

impl Ui {
    pub fn new() -> Ui {
        // Debug builds can shorten the wait, to try the destroying and reopening.
        #[cfg(debug_assertions)]
        let keep_hidden = env_seconds("BULLSEYE_UI_TIMEOUT_SECS").unwrap_or(KEEP_HIDDEN);
        #[cfg(not(debug_assertions))]
        let keep_hidden = KEEP_HIDDEN;
        Ui { hidden_since: Mutex::new(None), keep_hidden }
    }

    fn hidden(&self, now: Instant) {
        *self.hidden_since.locked() = Some(now);
    }

    fn shown(&self) {
        *self.hidden_since.locked() = None;
    }

    /// Whether the window has been hidden long enough to be destroyed.
    fn expired(&self, now: Instant) -> bool {
        self.hidden_since.locked().is_some_and(|since| now.saturating_duration_since(since) >= self.keep_hidden)
    }
}

/// Opens the window: shows the one there is, or makes a new one. A new one is shown by the page
/// itself, once it has drawn (`window_ready`), so there is no blank window in between.
pub fn show(app: &AppHandle) {
    match app.get_webview_window(LABEL) {
        Some(window) => reveal(&app.state::<Ui>(), &window),
        None => {
            if let Err(e) = create(app) {
                note("app", &format!("cannot open the window: {e}"));
            }
        }
    }
}

/// `show`, from a thread of its own: making a window from inside the event handler that asked for
/// it (a click on the tray icon) can deadlock.
pub fn show_in_background(app: AppHandle) {
    thread::spawn(move || show(&app));
}

fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Bullseye")
        .inner_size(WIDTH, HEIGHT)
        .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
        .resizable(true)
        .visible(false)
        .build()
}

fn reveal(ui: &Ui, window: &WebviewWindow) {
    ui.shown();
    let _ = window.show();
    let _ = window.set_focus();
    // Tells the page to look at the service again (it stops while it is hidden).
    let _ = window.emit("visibility", true);
}

/// Called by the page once it has drawn for the first time.
#[tauri::command]
pub fn window_ready(window: WebviewWindow, ui: State<Ui>) {
    reveal(&ui, &window);
}

/// The window's X: hides it, and starts the clock after which it is destroyed.
pub fn dismiss(window: &Window) {
    let _ = window.hide();
    let _ = window.emit("visibility", false);
    window.app_handle().state::<Ui>().hidden(Instant::now());
}

/// Destroys the window once it has been hidden long enough.
pub fn spawn_reaper(app: AppHandle) {
    let ui = app.state::<Ui>();
    let every = (ui.keep_hidden / 4).clamp(Duration::from_secs(1), Duration::from_secs(10));
    spawn_critical("window reaper", move || loop {
        thread::sleep(every);
        let ui = app.state::<Ui>();
        if ui.expired(Instant::now()) {
            if let Some(window) = app.get_webview_window(LABEL) {
                let _ = window.destroy();
            }
            ui.shown();
        }
    });
}

#[cfg(debug_assertions)]
fn env_seconds(name: &str) -> Option<Duration> {
    std::env::var(name).ok()?.parse().ok().map(Duration::from_secs)
}

/// Development aids, debug builds only. `BULLSEYE_SHOW=1` opens the window at start (otherwise
/// click the tray icon), `BULLSEYE_REOPEN_AFTER_SECS=n` opens it again n seconds later, as another
/// click on the tray icon would, and `BULLSEYE_UI_TIMEOUT_SECS=n` shortens how long a hidden
/// window is kept.
#[cfg(debug_assertions)]
pub fn debug_aids(app: &AppHandle) {
    if std::env::var_os("BULLSEYE_SHOW").is_some() {
        show_in_background(app.clone());
    }
    if let Some(after) = env_seconds("BULLSEYE_REOPEN_AFTER_SECS") {
        let app = app.clone();
        thread::spawn(move || {
            thread::sleep(after);
            show(&app);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_window_is_destroyed_only_after_its_time() {
        let ui = Ui { hidden_since: Mutex::new(None), keep_hidden: Duration::from_secs(300) };
        let start = Instant::now();
        // Never hidden (or shown): nothing to destroy.
        assert!(!ui.expired(start + Duration::from_secs(1000)));

        ui.hidden(start);
        assert!(!ui.expired(start + Duration::from_secs(299)), "reopening within the time is instant");
        assert!(ui.expired(start + Duration::from_secs(300)));
        assert!(ui.expired(start + Duration::from_secs(3000)));

        // Opened again: the clock is gone, and starts over the next time it is dismissed.
        ui.shown();
        assert!(!ui.expired(start + Duration::from_secs(3000)));
        ui.hidden(start + Duration::from_secs(3000));
        assert!(!ui.expired(start + Duration::from_secs(3100)));
        assert!(ui.expired(start + Duration::from_secs(3300)));
    }
}
