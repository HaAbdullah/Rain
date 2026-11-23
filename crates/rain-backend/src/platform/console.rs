//! Text output for a program that has no console of its own.
//!
//! `rain-backend.exe` is built for the Windows subsystem, so Windows starts it without a console
//! window and without standard output. The flags `--console`, `--stop` and `--version` are run by
//! a person in a terminal, who wants to see the output. This borrows the terminal's console.

use std::fs::OpenOptions;
use std::os::windows::io::IntoRawHandle;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE,
    STD_OUTPUT_HANDLE, SetStdHandle,
};

/// Connects standard output and standard error to the console of the parent process, if it has
/// one. Does nothing when the program was started without a console (by the shell, at logon) and
/// leaves alone any output that the parent already redirected to a file or a pipe.
pub fn attach_parent_console() {
    // SAFETY: `AttachConsole` takes a constant and has no memory arguments. It fails (returns
    // 0) when the parent has no console; there is then nothing to connect.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    connect_if_missing(STD_OUTPUT_HANDLE);
    connect_if_missing(STD_ERROR_HANDLE);
}

fn connect_if_missing(which: STD_HANDLE) {
    // SAFETY: `GetStdHandle` takes a constant and returns a handle value or an error marker.
    let current: HANDLE = unsafe { GetStdHandle(which) };
    if !current.is_null() && current != INVALID_HANDLE_VALUE {
        return; // already connected, for example redirected to a file by the parent
    }
    // "CONOUT$" is the name of the console's output buffer.
    let Ok(console) = OpenOptions::new().write(true).open("CONOUT$") else {
        return;
    };
    // The handle must stay open for the life of the process, so it is turned into a raw handle
    // that nothing closes.
    let raw = console.into_raw_handle();
    // SAFETY: `raw` is an open handle that this process owns and never closes.
    unsafe { SetStdHandle(which, raw.cast()) };
}
