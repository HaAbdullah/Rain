//! Rain's desktop shell.
//!
//! The shell is the window that hosts the frontend, plus the link to the backend. It owns: the
//! global hotkey, showing and hiding the window, remembering the window's size and position,
//! making sure only one copy of Rain runs, and everything between the page and the backend
//! program (see `backend.rs`, `bridge.rs`, `relay.rs`, `media.rs` and `facts.rs`). Every feature
//! lives in the backend or, until it moves there, in the frontend.
//!
//! The frontend learns about show and hide through two events, because it resets the search
//! field on every summon and pauses its timers while hidden.

mod backend;
mod bridge;
mod facts;
mod logging;
mod media;
mod relay;

#[cfg(test)]
mod live_backend_tests;
#[cfg(test)]
mod test_server;

use std::future::Future;
use std::sync::Arc;

use rain_types::limits::GAME_MODE_EXIT_GRACE;
use rain_types::{ApiError, Frame, ShellFacts};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewWindow, WindowEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_window_state::StateFlags;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use backend::{Backend, BackendProgram, StatusSink};
use facts::Facts;
use media::MediaFetcher;

/// Label of the one window, as declared in `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";
/// Added to the app identifier in a sandbox (a debug build or a `RAIN_*` override), so that a
/// development run and the installed Rain do not share the single-instance lock.
const SANDBOX_IDENTIFIER_SUFFIX: &str = ".dev";

/// Sent to the frontend after the window was shown and focused.
const EVENT_SUMMONED: &str = "rain:summoned";
/// Sent to the frontend after the window was hidden from the Rust side.
const EVENT_HIDDEN: &str = "rain:hidden";

/// Start flag of "Start with Windows": the window stays hidden until the user summons it. The
/// backend writes the registry entry with this flag (section 06 6.4); the shell only reads it.
const HIDDEN_FLAG: &str = "--hidden";

/// Ctrl+` summons and dismisses Rain from anywhere in Windows.
fn summon_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL), Code::Backquote)
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(MAIN_WINDOW)
}

fn window_is_visible(app: &AppHandle) -> bool {
    main_window(app)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

fn summon(window: &WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit(EVENT_SUMMONED, ());
    report_visibility(window.app_handle());
}

fn dismiss(window: &WebviewWindow) {
    let _ = window.hide();
    let _ = window.emit(EVENT_HIDDEN, ());
    report_visibility(window.app_handle());
}

/// Tells the backend that the window was shown or hidden, when that is news to it. The backend
/// reads no session files and no notes while nobody can look at the window, and catches up on
/// show. The page hides the window itself too (hide on blur, Escape), so this also runs when the
/// focus changes.
fn report_visibility(app: &AppHandle) {
    let Some(facts) = app.try_state::<Arc<Facts>>() else {
        return;
    };
    let facts = Arc::clone(&facts);
    let app = app.clone();
    in_background(&app.clone(), move |backend| async move {
        // Read inside the task, so the newest of several quick reports tells the truth.
        let visible = window_is_visible(&app);
        if !facts.visibility_changed(visible) {
            return Ok(());
        }
        facts::report(&backend.client(), &facts.snapshot(visible)).await
    });
}

/// Hides the window if the user is looking at it, otherwise brings it forward.
fn toggle(app: &AppHandle) {
    let Some(window) = main_window(app) else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    if visible && focused {
        dismiss(&window);
    } else {
        summon(&window);
        wake_backend(app);
    }
}

/// True when the program was started with `--hidden`.
fn starts_hidden(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|arg| arg == HIDDEN_FLAG)
}

/// Runs `work` with the backend handle in the background and logs a failure. Nothing waits for
/// it: the callers are window events that must stay quick.
fn in_background<F, Fut>(app: &AppHandle, work: F)
where
    F: FnOnce(Backend) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), ApiError>> + Send + 'static,
{
    let Some(backend) = app.try_state::<Backend>() else {
        return;
    };
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = work(backend).await {
            warn!(message = %error.message, "a call to the backend failed");
        }
    });
}

/// A user who summons the window while the backend is down (crashed, or stopped on request) gets
/// it started again, without waiting for the first call from the page (foundation section 2.2).
fn wake_backend(app: &AppHandle) {
    in_background(app, |backend| async move { backend.wake().await });
}

/// A second launch of Rain (Start menu, Windows Search): bring the window forward, and treat it
/// as the user's own wish to have Rain running. That also cancels the exit that game mode has
/// under way (section 06 6.8), and gives a hotkey that another program held a second chance
/// (section 06 6.5).
fn second_launch(app: &AppHandle) {
    if let Some(window) = main_window(app) {
        summon(&window);
    }
    retry_hotkey(app);
    in_background(app, |backend| async move { backend.relaunch().await });
}

/// Registers Ctrl+` and returns the text of a failure for the facts. Another program may own the
/// chord; Rain then still runs, it just cannot be summoned by key.
fn register_hotkey(app: &AppHandle) -> Result<(), String> {
    app.global_shortcut()
        .register(summon_shortcut())
        .map_err(|error| {
            warn!(%error, "could not register Ctrl+`");
            error.to_string()
        })
}

/// Tries the hotkey again when it is not registered, and tells the backend if that changed the facts.
fn retry_hotkey(app: &AppHandle) {
    let Some(facts) = app.try_state::<Arc<Facts>>() else {
        return;
    };
    if !facts.retry_hotkey(|| register_hotkey(app)) {
        return;
    }
    info!("the state of the hotkey changed; telling the backend");
    let facts = Arc::clone(&facts);
    let visible = window_is_visible(app);
    in_background(app, move |backend| async move {
        facts::report(&backend.client(), &facts.snapshot(visible)).await
    });
}

/// What the relay needs from the application: the page to emit to, the window to ask about, and
/// the means to exit.
struct TauriHost {
    app: AppHandle,
    facts: Arc<Facts>,
}

impl relay::Host for TauriHost {
    fn emit_frame(&self, frame: &Frame) {
        if let Err(error) = self.app.emit(relay::EVENT_STREAM, frame) {
            warn!(%error, topic = %frame.topic, "could not pass a frame to the page");
        }
    }

    fn shell_facts(&self) -> ShellFacts {
        self.facts.snapshot(window_is_visible(&self.app))
    }

    fn exit_app(&self) {
        info!("the shell exits");
        self.app.exit(0);
    }
}

/// Creates the handle to the backend, starts the relay of its stream, and starts the backend
/// itself when nothing answers on the pipe. Neither of them delays the window.
fn start_backend(app: &AppHandle, facts: Arc<Facts>) -> Result<(), Box<dyn std::error::Error>> {
    let pipe_name = rain_types::env::pipe_name()?;
    let page = app.clone();
    let tell_page: StatusSink = Arc::new(move |status| {
        if let Err(error) = page.emit(relay::EVENT_BACKEND, status) {
            warn!(%error, "could not tell the page about the backend state");
        }
    });
    let backend = Backend::new(&pipe_name, BackendProgram::beside_shell(), tell_page);
    app.manage(backend.clone());
    app.manage(MediaFetcher::new(backend.client()));

    let host = Arc::new(TauriHost {
        app: app.clone(),
        facts,
    });
    tauri::async_runtime::spawn(relay::run(
        backend.clone(),
        host,
        GAME_MODE_EXIT_GRACE,
        CancellationToken::new(),
    ));
    tauri::async_runtime::spawn(async move {
        if let Err(error) = backend.ensure().await {
            warn!(message = %error.message, "the backend could not be started");
        }
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut log = logging::init(); // written out when the app exits, see below
    let hidden = starts_hidden(std::env::args());
    info!(
        version = env!("CARGO_PKG_VERSION"),
        sandbox = rain_types::env::sandbox(),
        hidden,
        "Rain shell starting"
    );
    let mut context = tauri::generate_context!();
    if rain_types::env::sandbox() {
        // A development run is another app for Windows: its own single-instance lock, window
        // state and WebView2 profile. Without this, starting it while the installed Rain runs
        // would only wake the installed one and exit (foundation section 2.4).
        context
            .config_mut()
            .identifier
            .push_str(SANDBOX_IDENTIFIER_SUFFIX);
    }
    tauri::Builder::default()
        // Must be the first plugin: a second launch (Start menu, Windows
        // Search) only brings the running window forward and then exits.
        // Its arguments are ignored: `--hidden` counts only at the first start.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            second_launch(app);
        }))
        // Remembers size, position and maximised state. Visibility is left
        // out on purpose: Rain decides itself when the window is shown.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .build(),
        )
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if shortcut == &summon_shortcut() && event.state() == ShortcutState::Pressed {
                        toggle(app);
                    }
                })
                .build(),
        )
        // Icons and covers: `http://rain-media.localhost/media/...` (foundation section 6.3).
        .register_asynchronous_uri_scheme_protocol(media::SCHEME, media::handle_scheme)
        .invoke_handler(tauri::generate_handler![bridge::api_request])
        .setup(move |app| {
            let facts = Arc::new(Facts::new());
            facts.set_hotkey(register_hotkey(app.handle()));
            app.manage(Arc::clone(&facts));
            // The facts are complete before the relay can report them.
            start_backend(app.handle(), facts)?;
            // The window is created hidden so its saved size is applied
            // before anything is drawn. A launch by the user shows it;
            // a start with Windows (`--hidden`) leaves it hidden.
            if !hidden && let Some(window) = main_window(app.handle()) {
                summon(&window);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing (Alt+F4) only hides: the page stays loaded so the next
            // summon is instant.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                let _ = window.emit(EVENT_HIDDEN, ());
                report_visibility(window.app_handle());
            }
            if let WindowEvent::Focused(_) = event {
                report_visibility(window.app_handle());
            }
        })
        .build(context)
        .expect("error while building Rain")
        .run(move |_app, event| {
            // The process ends right after this event, so the log is written out here: the last
            // lines (why the shell exits) are the ones that matter most.
            if matches!(event, RunEvent::Exit) {
                log.take();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn the_hidden_flag_is_found_anywhere_in_the_arguments() {
        assert!(starts_hidden(args(&["rain.exe", "--hidden"])));
        assert!(starts_hidden(args(&["rain.exe", "--other", "--hidden"])));
    }

    #[test]
    fn without_the_flag_the_window_is_shown() {
        assert!(!starts_hidden(args(&["rain.exe"])));
        assert!(!starts_hidden(args(&[
            "rain.exe",
            "--hidden-ish",
            "hidden"
        ])));
    }
}
