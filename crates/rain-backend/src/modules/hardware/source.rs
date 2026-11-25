//! Where the readings come from. The seam of the tests: a fake answers from a script, so that no
//! test touches a real device.
//!
//! Every method blocks, and none of them keeps anything: each call reads the PC as it is now.

use std::path::Path;
use std::time::Instant;

use tracing::debug;

use super::hidpp::{self, Port, REPORT_LEN, Reading};
use super::inputs::{self, Endpoint, Inputs};
use crate::platform::hid::{self, HidChannel};
use crate::platform::registry::{UninstallEntry, uninstall_entries};

pub trait Source: Send + Sync {
    /// The keyboards and mice that are plugged in, and the Logitech channels to ask. Opens no
    /// device.
    fn inputs(&self) -> Inputs;

    /// Asks the Logitech devices behind one channel who they are and how full their battery is.
    /// Stops starting new questions when `deadline` has passed.
    fn ask(&self, endpoint: &Endpoint, deadline: Instant) -> Vec<Reading>;

    /// The programs in the list of installed programs of Windows.
    fn installed(&self) -> Vec<UninstallEntry>;

    /// Whether a program file is there.
    fn exists(&self, path: &Path) -> bool;
}

/// The real PC.
pub struct SystemSource;

impl Source for SystemSource {
    fn inputs(&self) -> Inputs {
        match hid::list() {
            Ok(entries) => inputs::classify(&entries),
            Err(reason) => {
                debug!(%reason, "the HID devices could not be listed");
                Inputs::default()
            }
        }
    }

    fn ask(&self, endpoint: &Endpoint, deadline: Instant) -> Vec<Reading> {
        match hid::open(&endpoint.path) {
            Ok(channel) => hidpp::read_channel(&ChannelPort(channel), deadline),
            Err(reason) => {
                debug!(%reason, "a HID++ channel could not be opened");
                Vec::new()
            }
        }
    }

    fn installed(&self) -> Vec<UninstallEntry> {
        uninstall_entries()
    }

    fn exists(&self, path: &Path) -> bool {
        path.is_file()
    }
}

/// A HID channel as the port of the HID++ conversation.
struct ChannelPort(HidChannel);

impl Port for ChannelPort {
    fn send(&self, report: &[u8; REPORT_LEN]) -> bool {
        self.0.write(report)
    }

    fn receive(&self, timeout_ms: i32) -> Option<Vec<u8>> {
        self.0.read(timeout_ms)
    }
}
