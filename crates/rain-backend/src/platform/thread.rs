//! Thread priority. Used by the disk scan so that a walk of a whole drive does not slow down
//! what the user is doing (section 05; below-normal priority was not tried in a spike, so the
//! scan measures it).

use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
};

/// Lowers the priority of the calling thread to below normal. Meant for the start handler of a
/// thread pool. A failure is ignored: the walk is then only a little more eager.
pub fn set_below_normal() {
    // SAFETY: `GetCurrentThread` returns a pseudo handle that is always valid for this thread.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Threading::GetThreadPriority;

    /// The priority of the calling thread (`THREAD_PRIORITY_*`).
    fn priority() -> i32 {
        // SAFETY: `GetCurrentThread` returns a pseudo handle that is always valid.
        unsafe { GetThreadPriority(GetCurrentThread()) }
    }

    #[test]
    fn the_priority_of_the_calling_thread_goes_below_normal_and_no_other_thread_changes() {
        let other = std::thread::spawn(priority).join().unwrap();
        std::thread::spawn(|| {
            set_below_normal();
            assert_eq!(priority(), THREAD_PRIORITY_BELOW_NORMAL.0);
        })
        .join()
        .unwrap();
        assert_eq!(std::thread::spawn(priority).join().unwrap(), other);
    }
}
