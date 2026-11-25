//! Hardware: the tiles of the Hardware screen. Feature section 05
//! (`docs/backend/05-disk-machine.md`, the hardware part).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::disk::Drive;

/// What a tile stands for. The screen picks the icon from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HardwareKind {
    Disk,
    Keyboard,
    Mouse,
}

/// How a device is connected to the PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HardwareLink {
    // A cable, or a device that is built in.
    Usb,
    // A 2.4 GHz receiver (a dongle) that talks to the device by radio.
    Receiver,
    Bluetooth,
}

/// What the device says about its battery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct HardwareBattery {
    // 0 to 100.
    pub percent: u8,
    // Absent when the device does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging: Option<bool>,
    // True when the percent was worked out from a voltage, not reported by the device.
    pub estimated: bool,
}

/// What Enter does on a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HardwareOpenKind {
    // The Disk screen of Rain. The screen does this itself.
    Disk,
    // A program on this PC.
    App,
    // A web page in the default browser.
    Url,
    // Nothing is known that fits this device.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct HardwareOpens {
    #[ts(inline)]
    pub kind: HardwareOpenKind,
    // What the action is called: "Logitech G HUB", "VIA", "usevia.app". Absent for `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One tile of the Hardware screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct HardwareTile {
    // "disk", "keyboard", "mouse"; a second device of a kind is "mouse-2".
    pub id: String,
    #[ts(inline)]
    pub kind: HardwareKind,
    // The title: the kind's name, or the name that hardware.toml gives.
    pub name: String,
    // False when no device of this kind was found (the tile stays, with nothing to show).
    pub found: bool,
    // The name the device gives itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(inline)]
    pub connection: Option<HardwareLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<HardwareBattery>,
    // The fixed drives; only on the disk tile.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drives: Vec<Drive>,
    pub opens: HardwareOpens,
    // A short text when this device could not be read in time. The other tiles are not affected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;
    use serde_json::json;

    #[test]
    fn a_tile_round_trips_and_leaves_out_what_it_does_not_have() {
        let tile = HardwareTile {
            id: "mouse".into(),
            kind: HardwareKind::Mouse,
            name: "Mouse".into(),
            found: true,
            product: Some("Pro Wireless".into()),
            connection: Some(HardwareLink::Receiver),
            battery: Some(HardwareBattery {
                percent: 80,
                charging: Some(false),
                estimated: false,
            }),
            drives: vec![],
            opens: HardwareOpens {
                kind: HardwareOpenKind::App,
                label: Some("Logitech G HUB".into()),
            },
            problem: None,
        };
        assert_eq!(
            round_trip(&tile),
            json!({
                "id": "mouse", "kind": "mouse", "name": "Mouse", "found": true,
                "product": "Pro Wireless", "connection": "receiver",
                "battery": { "percent": 80, "charging": false, "estimated": false },
                "opens": { "kind": "app", "label": "Logitech G HUB" }
            })
        );
    }
}
