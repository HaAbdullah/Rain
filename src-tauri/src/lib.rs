//! Rain's desktop shell.
//!
//! This is only the window that hosts the frontend. It owns four things:
//! the global hotkey, showing and hiding the window, remembering the window's
//! size and position, and making sure only one copy of Rain runs.
//! Every feature lives in the frontend for now.
//!
//! The frontend learns about show and hide through two events, because it
//! resets the search field on every summon and pauses its timers while hidden.

use tauri::{AppHandle, Emitter, Manager, WebviewWindow, WindowEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_window_state::StateFlags;

/// Label of the one window, as declared in `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";

/// Sent to the frontend after the window was shown and focused.
const EVENT_SUMMONED: &str = "rain:summoned";
/// Sent to the frontend after the window was hidden from the Rust side.
const EVENT_HIDDEN: &str = "rain:hidden";

/// Ctrl+` summons and dismisses Rain from anywhere in Windows.
fn summon_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL), Code::Backquote)
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(MAIN_WINDOW)
}

fn summon(window: &WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit(EVENT_SUMMONED, ());
}

fn dismiss(window: &WebviewWindow) {
    let _ = window.hide();
    let _ = window.emit(EVENT_HIDDEN, ());
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
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be the first plugin: a second launch (Start menu, Windows
        // Search) only brings the running window forward and then exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = main_window(app) {
                summon(&window);
            }
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
        .setup(|app| {
            // Another program may already own the hotkey. Rain still starts;
            // it just cannot be summoned by key until that is resolved.
            if let Err(error) = app.global_shortcut().register(summon_shortcut()) {
                eprintln!("could not register Ctrl+`: {error}");
            }
            // The window is created hidden so its saved size is applied
            // before anything is drawn. A launch by the user shows it.
            if let Some(window) = main_window(app.handle()) {
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
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Rain");
}
