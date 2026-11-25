//! HID devices: the list of what is plugged in, and a channel to write a report to one and read
//! its answer. Used by the hardware module (section 05) only, and only while the Hardware screen
//! is open.
//!
//! The `hidapi` crate does the Windows calls (its `windows-native` backend: no C compiler, no
//! extra DLL). Listing opens no device. A channel opens one device by its path, with the shared
//! mode that Windows gives every program, so G HUB or VIA keep working next to Rain.

use std::ffi::CString;

use hidapi::{BusType, HidApi, HidDevice};

/// How a HID device is attached, as far as Windows says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Usb,
    Bluetooth,
    Other,
}

/// One top-level collection of a HID device. A keyboard with three interfaces gives several of
/// these, with the same ids and product text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HidEntry {
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
    /// The USB interface number, or -1 when the device has none (Bluetooth).
    pub interface: i32,
    pub product: Option<String>,
    pub bus: Bus,
    /// The device path to open this collection.
    pub path: String,
}

/// Every HID collection that Windows lists now. Takes a few milliseconds.
pub fn list() -> Result<Vec<HidEntry>, String> {
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let mut entries: Vec<HidEntry> = api
        .device_list()
        .map(|info| HidEntry {
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            usage_page: info.usage_page(),
            usage: info.usage(),
            interface: info.interface_number(),
            product: info
                .product_string()
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty()),
            bus: match info.bus_type() {
                BusType::Usb => Bus::Usb,
                BusType::Bluetooth => Bus::Bluetooth,
                _ => Bus::Other,
            },
            path: info.path().to_string_lossy().into_owned(),
        })
        .collect();
    // The same order every time, so that two reads of one PC give the same tile ids.
    entries.sort_by(|a, b| {
        (
            a.vendor_id,
            a.product_id,
            a.interface,
            a.usage_page,
            a.usage,
            &a.path,
        )
            .cmp(&(
                b.vendor_id,
                b.product_id,
                b.interface,
                b.usage_page,
                b.usage,
                &b.path,
            ))
    });
    Ok(entries)
}

/// One open collection: reports go out with `write`, answers come back with `read`.
pub struct HidChannel {
    device: HidDevice,
}

/// Opens the collection at `path`. Fails when the device is gone or Windows refuses.
pub fn open(path: &str) -> Result<HidChannel, String> {
    // This lists the devices once more (a few milliseconds). `new_without_enumerate` would
    // skip that, but it panics when it runs a second time in one process.
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let path = CString::new(path).map_err(|e| e.to_string())?;
    let device = api.open_path(&path).map_err(|e| e.to_string())?;
    Ok(HidChannel { device })
}

impl HidChannel {
    /// Sends one output report. `report` starts with the report id.
    pub fn write(&self, report: &[u8]) -> bool {
        self.device.write(report).is_ok()
    }

    /// The next input report, or `None` when none arrives within `timeout_ms`.
    pub fn read(&self, timeout_ms: i32) -> Option<Vec<u8>> {
        let mut buffer = [0u8; 64];
        match self.device.read_timeout(&mut buffer, timeout_ms) {
            Ok(0) | Err(_) => None,
            Ok(length) => Some(buffer[..length].to_vec()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_of_this_pc_can_be_read_and_is_in_a_fixed_order() {
        let first = list().expect("Windows lists HID devices");
        let second = list().expect("Windows lists HID devices");
        let keys = |entries: &[HidEntry]| {
            entries
                .iter()
                .map(|entry| (entry.vendor_id, entry.product_id, entry.path.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&first), keys(&second));
    }

    #[test]
    fn a_path_that_is_not_a_device_does_not_open() {
        assert!(open(r"\\?\hid#vid_0000&pid_0000#nothing").is_err());
    }
}
