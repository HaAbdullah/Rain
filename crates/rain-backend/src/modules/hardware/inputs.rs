//! Sorting the HID devices of the PC into keyboards, mice and the Logitech channels to ask.
//!
//! Pure: the input is the list that `platform::hid::list` gives, the output says what to show and
//! what to ask. The class of a device comes from what it says it is (HID usages), never from
//! its name or its ids.
//!
//! - A Logitech device (a vendor with `hidpp` in the table) is not listed from its plain
//!   interfaces. Its HID++ channel is asked instead, and the answer says whether it is a mouse
//!   or a keyboard. A receiver has a keyboard interface and a mouse interface whatever is
//!   paired, so the plain interfaces would show a keyboard that does not exist.
//! - Every other device is listed from its plain interfaces. A keyboard often has a mouse
//!   collection too (mouse keys), and a mouse may have a keyboard one (macros), so the
//!   interfaces decide: a collection that has an interface of its own counts, a collection that
//!   shares an interface with the other kind does not.

use std::collections::BTreeMap;

use rain_types::HardwareLink;

use super::vendor::{VIA_USAGE, VIA_USAGE_PAGE, vendor_of};
use crate::platform::hid::{Bus, HidEntry};

const GENERIC_DESKTOP: u16 = 0x01;
const USAGE_MOUSE: u16 = 0x02;
const USAGE_KEYBOARD: u16 = 0x06;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Keyboard,
    Mouse,
}

/// A keyboard or mouse that is listed from its plain interfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDevice {
    pub kind: InputKind,
    pub vendor_id: u16,
    /// The product text of the device.
    pub product: Option<String>,
    /// `None` for a device that is built in (I2C, SPI).
    pub link: Option<HardwareLink>,
    /// A keyboard with the raw HID interface of QMK and VIA.
    pub via: bool,
}

/// One HID++ channel to ask: the long report collection of a Logitech device or receiver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub path: String,
    pub vendor_id: u16,
    pub bus: Bus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inputs {
    pub devices: Vec<InputDevice>,
    pub endpoints: Vec<Endpoint>,
}

/// Usage page and usage of the long HID++ report, as USB and receivers have it (`0xFF00`, 2) and
/// as Bluetooth LE devices have it (`0xFF43`, `0x0202`).
fn is_hidpp_long(entry: &HidEntry) -> bool {
    matches!(
        (entry.usage_page, entry.usage),
        (0xFF00, 0x0002) | (0xFF43, 0x0202)
    )
}

fn link_of(bus: Bus) -> Option<HardwareLink> {
    match bus {
        Bus::Usb => Some(HardwareLink::Usb),
        Bus::Bluetooth => Some(HardwareLink::Bluetooth),
        Bus::Other => None,
    }
}

fn bus_number(bus: Bus) -> u8 {
    match bus {
        Bus::Usb => 0,
        Bus::Bluetooth => 1,
        Bus::Other => 2,
    }
}

pub fn classify(entries: &[HidEntry]) -> Inputs {
    let mut inputs = Inputs::default();
    let mut groups: BTreeMap<(u16, u16, u8), Vec<&HidEntry>> = BTreeMap::new();
    for entry in entries {
        if vendor_of(entry.vendor_id).is_some_and(|vendor| vendor.hidpp) {
            if is_hidpp_long(entry) {
                inputs.endpoints.push(Endpoint {
                    path: entry.path.clone(),
                    vendor_id: entry.vendor_id,
                    bus: entry.bus,
                });
            }
            continue;
        }
        groups
            .entry((entry.vendor_id, entry.product_id, bus_number(entry.bus)))
            .or_default()
            .push(entry);
    }
    for group in groups.values() {
        inputs.devices.extend(devices_of(group));
    }
    inputs
}

/// The keyboards and mice in the collections of one physical device.
fn devices_of(group: &[&HidEntry]) -> Vec<InputDevice> {
    // (has a keyboard collection, has a mouse collection), by interface.
    let mut by_interface: BTreeMap<i32, (bool, bool)> = BTreeMap::new();
    for entry in group.iter().filter(|e| e.usage_page == GENERIC_DESKTOP) {
        let slot = by_interface.entry(entry.interface).or_default();
        match entry.usage {
            USAGE_KEYBOARD => slot.0 = true,
            USAGE_MOUSE => slot.1 = true,
            _ => {}
        }
    }
    let keyboard_any = by_interface.values().any(|(k, _)| *k);
    let mouse_any = by_interface.values().any(|(_, m)| *m);
    let keyboard_alone = by_interface.values().any(|(k, m)| *k && !*m);
    let mouse_alone = by_interface.values().any(|(k, m)| *m && !*k);

    let kinds: Vec<InputKind> = match (keyboard_any, mouse_any) {
        (false, false) => vec![],
        (true, false) => vec![InputKind::Keyboard],
        (false, true) => vec![InputKind::Mouse],
        (true, true) => match (keyboard_alone, mouse_alone) {
            (true, true) => vec![InputKind::Keyboard, InputKind::Mouse],
            (false, true) => vec![InputKind::Mouse],
            _ => vec![InputKind::Keyboard],
        },
    };

    let first = group[0];
    let product = group.iter().find_map(|entry| entry.product.clone());
    let via = group
        .iter()
        .any(|entry| (entry.usage_page, entry.usage) == (VIA_USAGE_PAGE, VIA_USAGE));
    kinds
        .into_iter()
        .map(|kind| InputDevice {
            kind,
            vendor_id: first.vendor_id,
            product: product.clone(),
            link: link_of(first.bus),
            via: via && kind == InputKind::Keyboard,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(vid: u16, pid: u16, interface: i32, page: u16, usage: u16, bus: Bus) -> HidEntry {
        HidEntry {
            vendor_id: vid,
            product_id: pid,
            usage_page: page,
            usage,
            interface,
            product: Some(format!("Device {vid:04x}")),
            bus,
            path: format!("\\\\?\\hid#{vid:04x}&{pid:04x}&{interface}&{page:04x}&{usage:04x}"),
        }
    }

    fn usb(vid: u16, pid: u16, interface: i32, page: u16, usage: u16) -> HidEntry {
        entry(vid, pid, interface, page, usage, Bus::Usb)
    }

    /// A VIA keyboard: boot keyboard on interface 0, raw HID on 1, and keys, system, consumer,
    /// vendor and mouse keys together on interface 2.
    fn via_keyboard() -> Vec<HidEntry> {
        vec![
            usb(0x320F, 0x5088, 0, 1, 6),
            usb(0x320F, 0x5088, 1, VIA_USAGE_PAGE, VIA_USAGE),
            usb(0x320F, 0x5088, 2, 1, 6),
            usb(0x320F, 0x5088, 2, 1, 0x80),
            usb(0x320F, 0x5088, 2, 0x0C, 1),
            usb(0x320F, 0x5088, 2, 0xFF00, 1),
            usb(0x320F, 0x5088, 2, 1, 2),
        ]
    }

    /// A Logitech receiver: keyboard, mouse and consumer interfaces, and the HID++ interface.
    fn logitech_receiver() -> Vec<HidEntry> {
        vec![
            usb(0x046D, 0xC53F, 0, 1, 6),
            usb(0x046D, 0xC53F, 1, 1, 2),
            usb(0x046D, 0xC53F, 1, 0x0C, 1),
            usb(0x046D, 0xC53F, 1, 1, 0x80),
            usb(0x046D, 0xC53F, 2, 0xFF00, 1),
            usb(0x046D, 0xC53F, 2, 0xFF00, 2),
            usb(0x046D, 0xC53F, 2, 0xFF00, 4),
        ]
    }

    #[test]
    fn a_keyboard_with_mouse_keys_on_a_shared_interface_is_one_keyboard() {
        let inputs = classify(&via_keyboard());
        assert_eq!(inputs.devices.len(), 1);
        assert_eq!(inputs.devices[0].kind, InputKind::Keyboard);
        assert_eq!(inputs.devices[0].link, Some(HardwareLink::Usb));
        assert!(inputs.devices[0].via, "the raw HID interface says VIA");
        assert!(inputs.endpoints.is_empty());
    }

    #[test]
    fn a_logitech_receiver_is_asked_and_not_listed_from_its_plain_interfaces() {
        let inputs = classify(&logitech_receiver());
        assert!(inputs.devices.is_empty());
        assert_eq!(inputs.endpoints.len(), 1, "only the long report collection");
        assert!(inputs.endpoints[0].path.contains("ff00&0002"));
        assert_eq!(inputs.endpoints[0].bus, Bus::Usb);
    }

    #[test]
    fn a_virtual_logitech_device_without_a_hidpp_channel_is_ignored() {
        let virtual_pair = vec![usb(0x046D, 0xC232, -1, 1, 6), usb(0x046D, 0xC231, -1, 1, 2)];
        assert_eq!(classify(&virtual_pair), Inputs::default());
    }

    #[test]
    fn a_plain_mouse_is_a_mouse_and_a_mouse_with_a_macro_keyboard_interface_too() {
        let plain = vec![usb(0x1111, 1, 0, 1, 2)];
        assert_eq!(classify(&plain).devices[0].kind, InputKind::Mouse);
        let macros = vec![usb(0x2222, 1, 0, 1, 2), usb(0x2222, 1, 1, 1, 6)];
        // Both have an interface of their own: a dongle that serves both, so both are listed.
        let kinds: Vec<_> = classify(&macros).devices.iter().map(|d| d.kind).collect();
        assert_eq!(kinds, vec![InputKind::Keyboard, InputKind::Mouse]);
        // A mouse whose keyboard collection shares the mouse interface is only a mouse.
        let shared = vec![
            usb(0x3333, 1, 0, 1, 2),
            usb(0x3333, 1, 1, 1, 2),
            usb(0x3333, 1, 1, 1, 6),
        ];
        let kinds: Vec<_> = classify(&shared).devices.iter().map(|d| d.kind).collect();
        assert_eq!(kinds, vec![InputKind::Mouse]);
    }

    #[test]
    fn devices_without_a_keyboard_or_mouse_collection_have_no_tile() {
        let headset = vec![
            usb(0x37D3, 1, 3, 0x0C, 1),
            usb(0x37D3, 1, 3, 0xFF00, 1),
            usb(0x37D3, 1, 3, 0x0B, 5),
        ];
        let lighting = vec![usb(0x0B05, 0x19AF, 0, 0xFF72, 0xA1)];
        assert_eq!(classify(&headset), Inputs::default());
        assert_eq!(classify(&lighting), Inputs::default());
    }

    #[test]
    fn the_bus_gives_the_connection() {
        let bluetooth = vec![entry(0x5555, 1, -1, 1, 6, Bus::Bluetooth)];
        assert_eq!(
            classify(&bluetooth).devices[0].link,
            Some(HardwareLink::Bluetooth)
        );
        let built_in = vec![entry(0x6666, 1, -1, 1, 6, Bus::Other)];
        assert_eq!(classify(&built_in).devices[0].link, None);
    }

    #[test]
    fn two_different_devices_stay_two_and_the_product_text_is_kept() {
        let mut entries = via_keyboard();
        entries.push(usb(0x1111, 1, 0, 1, 2));
        let inputs = classify(&entries);
        assert_eq!(inputs.devices.len(), 2);
        assert_eq!(inputs.devices[0].product.as_deref(), Some("Device 1111"));
        assert_eq!(inputs.devices[1].product.as_deref(), Some("Device 320f"));
    }

    #[test]
    fn a_bluetooth_hidpp_channel_is_found_too() {
        let ble = vec![entry(0x046D, 0xB034, -1, 0xFF43, 0x0202, Bus::Bluetooth)];
        let inputs = classify(&ble);
        assert_eq!(inputs.endpoints.len(), 1);
        assert_eq!(inputs.endpoints[0].bus, Bus::Bluetooth);
    }
}
