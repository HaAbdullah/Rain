//! HID++ 2.0, the way Logitech devices answer questions about themselves. Read only.
//!
//! The protocol is the one that the open-source Solaar project documents. A request is a
//! 20 byte "long" report: `11 <device> <feature index> <function and software id> <16 bytes>`.
//! The answer has the same four first bytes. An error answer starts `11 <device> FF <feature
//! index> <function and software id> <code>`.
//!
//! The only requests that this file builds:
//!
//! | Feature | Function | Meaning |
//! |---|---|---|
//! | `0x0000` root | 0 | the index of a feature (`GetFeature`) |
//! | `0x0000` root | 1 | ping |
//! | `0x0005` device name | 0, 1, 2 | name length, name text, device type |
//! | `0x1004` unified battery | 1 | status |
//! | `0x1001` battery voltage | 0 | voltage |
//! | `0x1000` battery status | 0 | level and charging state |
//!
//! Each one only asks. No request here changes a setting of the device (the battery features
//! also have functions that light an LED or change a threshold; they are never built).

use std::time::{Duration, Instant};

/// The report id of a long report.
pub const LONG_REPORT: u8 = 0x11;
pub const REPORT_LEN: usize = 20;
/// The device number of a device that is not behind a receiver (a cable, Bluetooth).
pub const DIRECT: u8 = 0xFF;
/// The devices that a receiver can serve: numbers 1 to 6.
pub const RECEIVER_SLOTS: std::ops::RangeInclusive<u8> = 1..=6;

/// The software id in the low nibble of the function byte. The device echoes it, which is how an
/// answer to this request is told from a notification. Not zero: zero is what notifications use.
const SOFTWARE_ID: u8 = 0x0A;

const ROOT: u8 = 0x00;
const FEATURE_DEVICE_NAME: u16 = 0x0005;
const FEATURE_BATTERY_STATUS: u16 = 0x1000;
const FEATURE_BATTERY_VOLTAGE: u16 = 0x1001;
const FEATURE_UNIFIED_BATTERY: u16 = 0x1004;

/// How long a device gets to answer one request.
const PING_WAIT: Duration = Duration::from_millis(150);
const ASK_WAIT: Duration = Duration::from_millis(300);
/// The name of a device is never longer than this (the loop must end for a device that lies).
const MAX_NAME: usize = 64;

// ---- what goes over the wire ----

/// One report out, one report in. The real port is a HID channel; the tests use a script.
pub trait Port {
    fn send(&self, report: &[u8; REPORT_LEN]) -> bool;
    /// The next report that arrives, or `None` after `timeout_ms`.
    fn receive(&self, timeout_ms: i32) -> Option<Vec<u8>>;
}

/// The request for `function` of the feature at `feature_index`.
pub fn request(device: u8, feature_index: u8, function: u8, params: &[u8]) -> [u8; REPORT_LEN] {
    let mut report = [0u8; REPORT_LEN];
    report[0] = LONG_REPORT;
    report[1] = device;
    report[2] = feature_index;
    report[3] = (function << 4) | SOFTWARE_ID;
    for (slot, byte) in report[4..].iter_mut().zip(params) {
        *slot = *byte;
    }
    report
}

/// What one incoming report is, for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The answer: the 16 parameter bytes.
    Reply([u8; 16]),
    /// The device refused, with its error code.
    Refused(u8),
    /// Something else (a notification, an answer to another program's request).
    Unrelated,
}

/// Decides what `report` is for the request `(device, feature_index, function)`.
pub fn classify(report: &[u8], device: u8, feature_index: u8, function: u8) -> Answer {
    if report.len() < 6 || !matches!(report[0], 0x10 | LONG_REPORT) || report[1] != device {
        return Answer::Unrelated;
    }
    let function_byte = (function << 4) | SOFTWARE_ID;
    // 0xFF: an error of HID++ 2.0. 0x8F: an error of HID++ 1.0 (an old device or a receiver).
    if matches!(report[2], 0xFF | 0x8F) && report[3] == feature_index && report[4] == function_byte
    {
        return Answer::Refused(report[5]);
    }
    if report[0] == LONG_REPORT
        && report.len() >= REPORT_LEN
        && report[2] == feature_index
        && report[3] == function_byte
    {
        let mut params = [0u8; 16];
        params.copy_from_slice(&report[4..REPORT_LEN]);
        return Answer::Reply(params);
    }
    Answer::Unrelated
}

/// Why a request got no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallError {
    /// Nothing came back in time (a device that sleeps or is out of range).
    Silent,
    /// The device answered with an error code.
    Refused(u8),
    /// The report could not be written.
    Port,
}

/// Sends one request and waits for its answer, until `wait` is over or `deadline` passes.
fn call(
    port: &dyn Port,
    device: u8,
    feature_index: u8,
    function: u8,
    params: &[u8],
    wait: Duration,
    deadline: Instant,
) -> Result<[u8; 16], CallError> {
    if !port.send(&request(device, feature_index, function, params)) {
        return Err(CallError::Port);
    }
    let end = (Instant::now() + wait).min(deadline);
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(CallError::Silent);
        }
        let millis = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
        let Some(report) = port.receive(millis) else {
            return Err(CallError::Silent);
        };
        match classify(&report, device, feature_index, function) {
            Answer::Reply(params) => return Ok(params),
            Answer::Refused(code) => return Err(CallError::Refused(code)),
            Answer::Unrelated => {}
        }
    }
}

// ---- decoding the answers (pure) ----

/// The battery as a device reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: Option<bool>,
    pub estimated: bool,
}

/// Feature `0x1004`, function 1: `[charge %, level flags, charging status, external power]`.
/// The level flags are 1 critical, 2 low, 4 good, 8 full. A device that does not know its exact
/// charge sends 0 and only the flags: the percent is then a rough estimate.
pub fn decode_unified_battery(params: &[u8; 16]) -> Option<Battery> {
    let (charge, level, status) = (params[0], params[1], params[2]);
    // 0 discharging, 1 charging, 2 charging slowly, 3 full; above that an error.
    let charging = match status {
        0 | 3 => Some(false),
        1 | 2 => Some(true),
        _ => None,
    };
    if (1..=100).contains(&charge) {
        return Some(Battery {
            percent: charge,
            charging,
            estimated: false,
        });
    }
    let percent = match level {
        l if l & 8 != 0 => 100,
        l if l & 4 != 0 => 60,
        l if l & 2 != 0 => 20,
        l if l & 1 != 0 => 5,
        _ => return None,
    };
    Some(Battery {
        percent,
        charging,
        estimated: true,
    })
}

/// Feature `0x1000`, function 0: `[charge %, next level, charging status]`.
pub fn decode_battery_status(params: &[u8; 16]) -> Option<Battery> {
    let (charge, status) = (params[0], params[2]);
    if charge > 100 {
        return None;
    }
    // 0 discharging, 1 recharging, 2 almost full, 3 charged, 4 charging slowly; above: an error.
    let charging = match status {
        0 | 3 => Some(false),
        1 | 2 | 4 => Some(true),
        _ => None,
    };
    Some(Battery {
        percent: charge,
        charging,
        estimated: false,
    })
}

/// Feature `0x1001`, function 0: `[voltage mV (2 bytes), flags]`. Bit 7 of the flags is set while
/// the device charges. The percent comes from the curve of a lithium cell: an estimate.
pub fn decode_battery_voltage(params: &[u8; 16]) -> Option<Battery> {
    let millivolt = u16::from_be_bytes([params[0], params[1]]);
    if millivolt < 2500 {
        return None;
    }
    Some(Battery {
        percent: percent_of_voltage(millivolt),
        charging: Some(params[2] & 0x80 != 0),
        estimated: true,
    })
}

/// Millivolt and percent of a lithium cell under a light load, from high to low.
const VOLTAGE_CURVE: [(u16, u8); 12] = [
    (4186, 100),
    (4067, 90),
    (3989, 80),
    (3922, 70),
    (3859, 60),
    (3811, 50),
    (3778, 40),
    (3751, 30),
    (3722, 20),
    (3680, 10),
    (3640, 5),
    (3300, 0),
];

fn percent_of_voltage(millivolt: u16) -> u8 {
    let [(top, _), ..] = VOLTAGE_CURVE;
    if millivolt >= top {
        return 100;
    }
    for pair in VOLTAGE_CURVE.windows(2) {
        let ((high, high_percent), (low, low_percent)) = (pair[0], pair[1]);
        if millivolt >= low {
            let span = u32::from(high - low);
            let into = u32::from(millivolt - low);
            let gain = u32::from(high_percent - low_percent);
            return low_percent + u8::try_from(into * gain / span).unwrap_or(0);
        }
    }
    0
}

/// What kind of device a HID++ device says it is (feature `0x0005`, function 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Keyboard,
    Mouse,
    Other,
}

/// 0 keyboard; 3 mouse, 4 trackpad, 5 trackball. The rest (receivers, headsets, webcams) have no
/// tile.
pub fn decode_device_kind(code: u8) -> DeviceKind {
    match code {
        0 => DeviceKind::Keyboard,
        3..=5 => DeviceKind::Mouse,
        _ => DeviceKind::Other,
    }
}

/// The part of a device name that one answer carries: up to 16 characters, ended by a NUL.
fn name_piece(params: &[u8; 16]) -> &[u8] {
    let end = params.iter().position(|b| *b == 0).unwrap_or(params.len());
    &params[..end]
}

// ---- asking one device ----

/// What Rain learns about one Logitech device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub name: Option<String>,
    pub kind: DeviceKind,
    pub battery: Option<Battery>,
    /// True when the device is behind a receiver (a dongle), false for a cable or Bluetooth.
    pub receiver: bool,
}

fn feature_index(
    port: &dyn Port,
    device: u8,
    feature: u16,
    deadline: Instant,
) -> Result<Option<u8>, CallError> {
    let [high, low] = feature.to_be_bytes();
    let answer = call(port, device, ROOT, 0, &[high, low], ASK_WAIT, deadline)?;
    // Index 0 is the root itself: the device does not have the feature.
    Ok(Some(answer[0]).filter(|index| *index != 0))
}

/// Asks the device behind `port` at number `device` (one that answered the ping) who it is, and
/// its battery.
fn read_device(port: &dyn Port, device: u8, receiver: bool, deadline: Instant) -> Reading {
    let mut name = None;
    let mut kind = DeviceKind::Other;
    if let Ok(Some(index)) = feature_index(port, device, FEATURE_DEVICE_NAME, deadline) {
        name = read_name(port, device, index, deadline);
        if let Ok(answer) = call(port, device, index, 2, &[], ASK_WAIT, deadline) {
            kind = decode_device_kind(answer[0]);
        }
    }
    Reading {
        name,
        kind,
        battery: read_battery(port, device, deadline),
        receiver,
    }
}

fn read_name(port: &dyn Port, device: u8, index: u8, deadline: Instant) -> Option<String> {
    let length =
        usize::from(call(port, device, index, 0, &[], ASK_WAIT, deadline).ok()?[0]).min(MAX_NAME);
    let mut bytes: Vec<u8> = Vec::new();
    while bytes.len() < length {
        let offset = u8::try_from(bytes.len()).ok()?;
        let answer = call(port, device, index, 1, &[offset], ASK_WAIT, deadline).ok()?;
        let piece = name_piece(&answer);
        if piece.is_empty() {
            break;
        }
        bytes.extend_from_slice(piece);
    }
    bytes.truncate(length);
    let text = String::from_utf8_lossy(&bytes).trim().to_owned();
    Some(text).filter(|text| !text.is_empty())
}

/// The first battery feature that the device has: unified, then voltage, then status.
fn read_battery(port: &dyn Port, device: u8, deadline: Instant) -> Option<Battery> {
    type Decode = fn(&[u8; 16]) -> Option<Battery>;
    let features: [(u16, u8, Decode); 3] = [
        (FEATURE_UNIFIED_BATTERY, 1, decode_unified_battery),
        (FEATURE_BATTERY_VOLTAGE, 0, decode_battery_voltage),
        (FEATURE_BATTERY_STATUS, 0, decode_battery_status),
    ];
    for (feature, function, decode) in features {
        match feature_index(port, device, feature, deadline) {
            Ok(Some(index)) => {
                let answer = call(port, device, index, function, &[], ASK_WAIT, deadline).ok()?;
                return decode(&answer);
            }
            // The device does not have this feature: try the next one.
            Ok(None) => {}
            // Silent or refused: asking more will not help.
            Err(_) => return None,
        }
    }
    None
}

/// Pings every device number at once and collects who answers. A number that nobody uses gives no
/// answer, so asking one by one would wait for each of them: asking all together costs one wait.
fn ping_all(port: &dyn Port, numbers: &[u8], deadline: Instant) -> Vec<u8> {
    let mut alive = Vec::new();
    let mut answered = 0;
    for number in numbers {
        if !port.send(&request(*number, ROOT, 1, &[0, 0, 0x5A])) {
            return alive;
        }
    }
    let end = (Instant::now() + PING_WAIT).min(deadline);
    while answered < numbers.len() {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let millis = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
        let Some(report) = port.receive(millis) else {
            break;
        };
        for number in numbers {
            match classify(&report, *number, ROOT, 1) {
                Answer::Reply(_) => {
                    alive.push(*number);
                    answered += 1;
                }
                Answer::Refused(_) => answered += 1,
                Answer::Unrelated => continue,
            }
            break;
        }
    }
    alive
}

/// Finds the Logitech devices behind one HID++ channel: a device that talks directly (a cable or
/// Bluetooth) or the devices on the six slots of a receiver. Stops starting new questions when
/// `deadline` has passed, and returns what it has.
pub fn read_channel(port: &dyn Port, deadline: Instant) -> Vec<Reading> {
    let numbers: Vec<u8> = std::iter::once(DIRECT).chain(RECEIVER_SLOTS).collect();
    let alive = ping_all(port, &numbers, deadline);
    if alive.contains(&DIRECT) {
        return vec![read_device(port, DIRECT, false, deadline)];
    }
    alive
        .into_iter()
        .take_while(|_| Instant::now() < deadline)
        .map(|slot| read_device(port, slot, true, deadline))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    type Responder = dyn Fn(&[u8; REPORT_LEN]) -> Option<Vec<u8>>;

    /// A device that answers from a function. Every answer is preceded by a notification, which
    /// the reader must skip. `requests` records every report that was sent.
    struct Script {
        respond: Box<Responder>,
        queue: RefCell<VecDeque<Vec<u8>>>,
        requests: RefCell<Vec<[u8; REPORT_LEN]>>,
    }

    impl Script {
        fn new(respond: impl Fn(&[u8; REPORT_LEN]) -> Option<Vec<u8>> + 'static) -> Self {
            Self {
                respond: Box::new(respond),
                queue: RefCell::default(),
                requests: RefCell::default(),
            }
        }
    }

    impl Port for Script {
        fn send(&self, report: &[u8; REPORT_LEN]) -> bool {
            self.requests.borrow_mut().push(*report);
            if let Some(reply) = (self.respond)(report) {
                let mut notification = vec![LONG_REPORT, report[1], 0x07, 0x00, 1, 2, 3];
                notification.resize(REPORT_LEN, 0);
                let mut queue = self.queue.borrow_mut();
                queue.push_back(notification);
                queue.push_back(reply);
            }
            true
        }

        fn receive(&self, _timeout_ms: i32) -> Option<Vec<u8>> {
            self.queue.borrow_mut().pop_front()
        }
    }

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(30)
    }

    fn params(bytes: &[u8]) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[..bytes.len()].copy_from_slice(bytes);
        out
    }

    /// The answer to `report`: its first four bytes, then `data`.
    fn answer(report: &[u8; REPORT_LEN], data: &[u8]) -> Option<Vec<u8>> {
        let mut full = vec![LONG_REPORT, report[1], report[2], report[3]];
        full.extend_from_slice(data);
        full.resize(REPORT_LEN, 0);
        Some(full)
    }

    /// A mouse behind a receiver on slot 1: "G PRO X" (7 letters), unified battery at 87 %.
    /// Feature table: 0x0005 at index 3, 0x1004 at index 9.
    fn mouse_on_slot_one(report: &[u8; REPORT_LEN]) -> Option<Vec<u8>> {
        if report[1] != 1 {
            return None; // the other slots are empty: no answer at all
        }
        let (index, function) = (report[2], report[3] >> 4);
        match (index, function) {
            (0x00, 1) => answer(report, &[4, 2, 0x5A]),
            (0x00, 0) => match u16::from_be_bytes([report[4], report[5]]) {
                0x0005 => answer(report, &[3, 0, 1]),
                0x1004 => answer(report, &[9, 0, 1]),
                _ => answer(report, &[0, 0, 0]),
            },
            (3, 0) => answer(report, &[7]),
            (3, 1) => answer(report, b"G PRO X"),
            (3, 2) => answer(report, &[3]),
            (9, 1) => answer(report, &[87, 4, 0, 0]),
            _ => None,
        }
    }

    // ---- requests ----

    #[test]
    fn a_request_is_a_long_report_with_the_function_and_the_software_id() {
        let report = request(1, 0x07, 1, &[9, 8]);
        assert_eq!(report[..6], [0x11, 1, 0x07, 0x1A, 9, 8]);
        assert_eq!(report.len(), 20);
        assert!(report[6..].iter().all(|b| *b == 0));
    }

    // ---- classifying answers, from byte fixtures ----

    #[test]
    fn an_answer_repeats_the_first_four_bytes_of_the_request() {
        let mut bytes = vec![0x11, 1, 0x07, 0x1A, 55, 1, 0];
        bytes.resize(20, 0);
        let Answer::Reply(params) = classify(&bytes, 1, 0x07, 1) else {
            panic!("a reply");
        };
        assert_eq!(params[..3], [55, 1, 0]);
    }

    #[test]
    fn a_notification_or_an_answer_to_another_request_is_unrelated() {
        let mut other = vec![0x11, 1, 0x07, 0x2B, 0];
        other.resize(20, 0);
        assert_eq!(classify(&other, 1, 0x07, 1), Answer::Unrelated);
        let mut notification = vec![0x11, 1, 0x07, 0x00, 0];
        notification.resize(20, 0);
        assert_eq!(classify(&notification, 1, 0x07, 1), Answer::Unrelated);
        let mut elsewhere = vec![0x11, 2, 0x07, 0x1A, 0];
        elsewhere.resize(20, 0);
        assert_eq!(classify(&elsewhere, 1, 0x07, 1), Answer::Unrelated);
        assert_eq!(classify(&[0x11, 1], 1, 0x07, 1), Answer::Unrelated);
    }

    #[test]
    fn errors_of_hidpp_2_and_hidpp_1_are_refusals() {
        let mut two = vec![0x11, 1, 0xFF, 0x07, 0x1A, 0x08];
        two.resize(20, 0);
        assert_eq!(classify(&two, 1, 0x07, 1), Answer::Refused(8));
        // A short report of HID++ 1.0: "invalid sub id" from a device that does not know 2.0.
        let one = [0x10, 1, 0x8F, 0x00, 0x1A, 0x01, 0];
        assert_eq!(classify(&one, 1, 0x00, 1), Answer::Refused(1));
    }

    // ---- decoding batteries, from byte fixtures ----

    #[test]
    fn the_unified_battery_gives_the_charge_and_the_charging_state() {
        let discharging = decode_unified_battery(&params(&[87, 4, 0, 0])).unwrap();
        assert_eq!(
            discharging,
            Battery {
                percent: 87,
                charging: Some(false),
                estimated: false
            }
        );
        let charging = decode_unified_battery(&params(&[40, 2, 1, 1])).unwrap();
        assert_eq!((charging.percent, charging.charging), (40, Some(true)));
        let slowly = decode_unified_battery(&params(&[41, 2, 2, 1])).unwrap();
        assert_eq!(slowly.charging, Some(true));
        let full = decode_unified_battery(&params(&[100, 8, 3, 1])).unwrap();
        assert_eq!((full.percent, full.charging), (100, Some(false)));
        let broken = decode_unified_battery(&params(&[50, 4, 5, 0])).unwrap();
        assert_eq!(broken.charging, None);
    }

    #[test]
    fn a_unified_battery_without_a_charge_falls_back_to_its_level_flag() {
        let low = decode_unified_battery(&params(&[0, 2, 0, 0])).unwrap();
        assert_eq!((low.percent, low.estimated), (20, true));
        let critical = decode_unified_battery(&params(&[0, 1, 0, 0])).unwrap();
        assert_eq!(critical.percent, 5);
        assert!(decode_unified_battery(&params(&[0, 0, 0, 0])).is_none());
    }

    #[test]
    fn the_battery_status_feature_gives_charge_and_state() {
        let battery = decode_battery_status(&params(&[63, 50, 1])).unwrap();
        assert_eq!((battery.percent, battery.charging), (63, Some(true)));
        let idle = decode_battery_status(&params(&[63, 50, 0])).unwrap();
        assert_eq!(idle.charging, Some(false));
        assert!(decode_battery_status(&params(&[200, 0, 0])).is_none());
    }

    #[test]
    fn the_voltage_becomes_an_estimated_percent_and_bit_7_means_charging() {
        // 3989 mV (0x0F95) is the 80 % point of the curve.
        let battery = decode_battery_voltage(&params(&[0x0F, 0x95, 0x00])).unwrap();
        assert_eq!(
            battery,
            Battery {
                percent: 80,
                charging: Some(false),
                estimated: true
            }
        );
        let charging = decode_battery_voltage(&params(&[0x10, 0x62, 0x80])).unwrap();
        assert_eq!(charging.percent, 100);
        assert_eq!(charging.charging, Some(true));
        // 3890 mV (0x0F32) lies between 3922 (70) and 3859 (60): about 65.
        let between = decode_battery_voltage(&params(&[0x0F, 0x32, 0])).unwrap();
        assert!((64..=66).contains(&between.percent), "{}", between.percent);
        assert!(decode_battery_voltage(&params(&[0, 0, 0])).is_none());
    }

    #[test]
    fn the_curve_never_gives_more_than_100_or_less_than_0() {
        assert_eq!(percent_of_voltage(5000), 100);
        assert_eq!(percent_of_voltage(2600), 0);
    }

    // ---- names and kinds ----

    #[test]
    fn the_device_type_decides_whether_there_is_a_tile() {
        assert_eq!(decode_device_kind(0), DeviceKind::Keyboard);
        assert_eq!(decode_device_kind(3), DeviceKind::Mouse);
        assert_eq!(decode_device_kind(5), DeviceKind::Mouse);
        assert_eq!(decode_device_kind(7), DeviceKind::Other);
        assert_eq!(decode_device_kind(8), DeviceKind::Other);
    }

    #[test]
    fn a_name_piece_ends_at_the_first_nul() {
        assert_eq!(name_piece(&params(b"PRO X 2\0junk")), b"PRO X 2");
        assert_eq!(name_piece(&[b'a'; 16]).len(), 16);
    }

    // ---- the whole conversation, with a script ----

    #[test]
    fn a_receiver_with_one_mouse_gives_its_name_kind_and_battery() {
        let port = Script::new(mouse_on_slot_one);
        let readings = read_channel(&port, far());
        assert_eq!(
            readings,
            vec![Reading {
                name: Some("G PRO X".to_owned()),
                kind: DeviceKind::Mouse,
                battery: Some(Battery {
                    percent: 87,
                    charging: Some(false),
                    estimated: false
                }),
                receiver: true,
            }]
        );
    }

    #[test]
    fn only_read_requests_are_sent() {
        let port = Script::new(mouse_on_slot_one);
        read_channel(&port, far());
        for report in port.requests.borrow().iter() {
            let (index, function) = (report[2], report[3] >> 4);
            let allowed = matches!(
                (index, function),
                (0x00, 0 | 1) // root: GetFeature, ping (to every number)
                | (3, 0..=2)  // device name: length, text, type
                | (9, 1) // unified battery: status
            );
            assert!(allowed, "a request that is not on the list: {report:02x?}");
            assert_eq!(report[0], LONG_REPORT);
            assert_eq!(report[3] & 0x0F, SOFTWARE_ID);
        }
    }

    #[test]
    fn a_direct_device_is_found_before_the_receiver_slots_are_tried() {
        let port = Script::new(|report| {
            if report[1] != DIRECT {
                return None;
            }
            match (report[2], report[3] >> 4) {
                (0x00, 1) => answer(report, &[4, 2, 0x5A]),
                // No device name and no battery feature.
                (0x00, 0) => answer(report, &[0, 0, 0]),
                _ => None,
            }
        });
        let readings = read_channel(&port, far());
        assert_eq!(readings.len(), 1);
        assert!(!readings[0].receiver);
        assert_eq!(readings[0].name, None);
        assert_eq!(readings[0].battery, None);
        // Every number is pinged at once, but only the direct device is asked more.
        assert!(
            port.requests
                .borrow()
                .iter()
                .all(|r| r[1] == DIRECT || (r[2], r[3] >> 4) == (ROOT, 1)),
            "no slot of a receiver is asked more than a ping"
        );
    }

    #[test]
    fn a_device_that_refuses_hidpp_2_is_not_a_reading() {
        let port = Script::new(|report| {
            let mut full = vec![0x10, report[1], 0x8F, 0x00, report[3], 0x01];
            full.resize(7, 0);
            Some(full)
        });
        assert!(read_channel(&port, far()).is_empty());
    }

    #[test]
    fn a_silent_port_gives_nothing_and_a_passed_deadline_stops_the_questions() {
        let silent = Script::new(|_| None);
        assert!(read_channel(&silent, far()).is_empty());

        let late = Script::new(mouse_on_slot_one);
        let past = Instant::now() - Duration::from_secs(1);
        assert!(read_channel(&late, past).is_empty());
    }

    #[test]
    fn a_battery_that_fails_leaves_the_rest_of_the_reading() {
        let port = Script::new(|report| {
            if report[1] != 1 {
                return None;
            }
            match (report[2], report[3] >> 4) {
                (0x00, 1) => answer(report, &[4, 2, 0x5A]),
                (0x00, 0) => match u16::from_be_bytes([report[4], report[5]]) {
                    0x0005 => answer(report, &[3, 0, 1]),
                    // Has the battery feature, but the device is asleep: no answer to its status.
                    0x1004 => answer(report, &[9, 0, 1]),
                    _ => answer(report, &[0, 0, 0]),
                },
                (3, 0) => answer(report, &[4]),
                (3, 1) => answer(report, b"Anne"),
                (3, 2) => answer(report, &[0]),
                _ => None,
            }
        });
        let readings = read_channel(&port, far());
        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].name.as_deref(), Some("Anne"));
        assert_eq!(readings[0].kind, DeviceKind::Keyboard);
        assert_eq!(readings[0].battery, None);
    }
}
