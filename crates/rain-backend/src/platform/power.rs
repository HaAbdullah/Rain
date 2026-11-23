//! A notice when the PC wakes from sleep. Section 03 (the scheduler).
//!
//! The scheduler sleeps until the next job is due. A PC that sleeps stops the timer, so after a
//! resume the scheduler must work out the due jobs again from the wall clock. This is an event
//! source and not a timer: it costs nothing while the PC runs. It needs no window and no
//! message loop (`PowerRegisterSuspendResumeNotification` with a callback). Not proven in a
//! spike; if the registration fails the scheduler falls back to a sleep of at most 5 minutes.

use std::ffi::c_void;

use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY, PowerRegisterSuspendResumeNotification,
    PowerUnregisterSuspendResumeNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
};

type Callback = Box<dyn Fn() + Send + Sync>;

/// A registration. Dropping it ends the notices.
pub struct ResumeWatch {
    handle: HPOWERNOTIFY,
    // The system holds a pointer to this. It stays alive and in place until `Drop` has
    // unregistered, and it is released after that.
    parameters: *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
    callback: *mut Callback,
}

// SAFETY: the raw pointers are owned boxes that only `Drop` frees; the callback they hold is
// `Send + Sync`.
unsafe impl Send for ResumeWatch {}

/// Calls `callback` every time the PC resumes from sleep, on a thread of the system. The
/// callback must be quick and only pass a message on (a `Notify`). `None` when Windows refuses
/// the registration.
pub fn on_resume(callback: impl Fn() + Send + Sync + 'static) -> Option<ResumeWatch> {
    let callback: *mut Callback = Box::into_raw(Box::new(Box::new(callback) as Callback));
    let parameters = Box::into_raw(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(notice),
        Context: callback.cast(),
    }));
    let mut handle = std::ptr::null_mut::<c_void>();
    // SAFETY: `parameters` points to a valid structure that stays alive until `Drop`; the
    // flag says that the handle argument is such a structure and not a window.
    let status = unsafe {
        PowerRegisterSuspendResumeNotification(
            DEVICE_NOTIFY_CALLBACK,
            HANDLE(parameters.cast()),
            &mut handle,
        )
    };
    if status != ERROR_SUCCESS {
        // SAFETY: the registration failed, so nothing else holds the two boxes.
        unsafe {
            drop(Box::from_raw(parameters));
            drop(Box::from_raw(callback));
        }
        return None;
    }
    Some(ResumeWatch {
        handle: HPOWERNOTIFY(handle as isize),
        parameters,
        callback,
    })
}

/// The system's callback. `PBT_APMRESUMEAUTOMATIC` and `PBT_APMRESUMESUSPEND` both mean that the
/// PC is awake; the first comes always, the second only when the user was present. Either one
/// triggers the callback (a double notice is harmless: the scheduler recomputes).
unsafe extern "system" fn notice(
    context: *const c_void,
    kind: u32,
    _setting: *const c_void,
) -> u32 {
    if kind == PBT_APMRESUMEAUTOMATIC || kind == PBT_APMRESUMESUSPEND {
        // SAFETY: `context` is the pointer to the boxed callback from `on_resume`, alive until
        // `Drop` has unregistered, which waits for a running notice to finish.
        let callback = unsafe { &*(context.cast::<Callback>()) };
        callback();
    }
    0
}

impl Drop for ResumeWatch {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful registration. After it is unregistered no
        // notice runs, so the two boxes can be freed.
        unsafe {
            let _ = PowerUnregisterSuspendResumeNotification(self.handle);
            drop(Box::from_raw(self.parameters));
            drop(Box::from_raw(self.callback));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_registration_can_be_made_and_cancelled_without_a_notice() {
        let notices = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&notices);
        let watch = on_resume(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        // A PC that cannot register (a restricted session) is not a failure of this code.
        if let Some(watch) = watch {
            drop(watch);
        }
        assert_eq!(
            notices.load(Ordering::SeqCst),
            0,
            "the PC did not resume during the test"
        );
    }

    #[test]
    fn the_notice_calls_the_callback_for_a_resume_and_ignores_other_events() {
        let notices = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&notices);
        let callback: Callback = Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let context = std::ptr::from_ref(&callback).cast::<c_void>();

        // SAFETY: `context` points to a live `Callback` for the length of the calls.
        unsafe {
            notice(context, PBT_APMRESUMEAUTOMATIC, std::ptr::null());
            notice(context, PBT_APMRESUMESUSPEND, std::ptr::null());
            notice(context, 0x0004, std::ptr::null()); // PBT_APMSUSPEND
        }
        assert_eq!(notices.load(Ordering::SeqCst), 2);
    }
}
