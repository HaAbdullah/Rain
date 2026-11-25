//! The hardware module on a real backend (feature section 05): `GET /hardware` and the open
//! route, with a fake PC. The one test that reads the real PC is marked `#[ignore]`.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::TestBackend;
use common::fakes::{FakeHardware, FakeOpener};
use hyper::StatusCode;
use rain_backend::core::launch_contract::OpenRequest;
use rain_backend::modules::disk::{DriveInfo, DriveSource};
use rain_backend::modules::hardware::{
    Battery, DEVICE_LIMIT, DeviceKind, DeviceLimit, Endpoint, InputDevice, InputKind, Inputs,
    Reading, Source, SystemSource,
};
use rain_backend::platform::hid::Bus;
use rain_backend::platform::registry::UninstallEntry;
use rain_types::{Drive, HardwareKind, HardwareLink, HardwareOpenKind, HardwareTile};
use serde_json::Value;

// ---- fakes ----

struct FakeDrives(Vec<DriveInfo>);

impl DriveSource for FakeDrives {
    fn list(&self) -> Vec<DriveInfo> {
        self.0.clone()
    }
}

fn drive(letter: char, total: u64, used: u64) -> DriveInfo {
    DriveInfo {
        drive: Drive {
            id: format!("drive-{}", letter.to_ascii_lowercase()),
            letter: letter.to_string(),
            label: "Local Disk".to_owned(),
            total_bytes: total,
            used_bytes: used,
        },
        root: PathBuf::from(format!("{letter}:\\")),
        root_name: format!("{letter}:"),
    }
}

/// A drive source that takes too long.
struct SlowDrives;

impl DriveSource for SlowDrives {
    fn list(&self) -> Vec<DriveInfo> {
        let end = Instant::now() + Duration::from_millis(600);
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            std::thread::park_timeout(left);
        }
        vec![drive('C', 100, 50)]
    }
}

const RECEIVER: &str = r"\\?\hid#receiver";
const LGHUB: &str = r"C:\Program Files\LGHUB\lghub.exe";

fn logitech_endpoint() -> Endpoint {
    Endpoint {
        path: RECEIVER.to_owned(),
        vendor_id: 0x046D,
        bus: Bus::Usb,
    }
}

fn mouse_reading() -> Reading {
    Reading {
        name: Some("G PRO X".to_owned()),
        kind: DeviceKind::Mouse,
        battery: Some(Battery {
            percent: 87,
            charging: Some(false),
            estimated: false,
        }),
        receiver: true,
    }
}

fn via_keyboard() -> InputDevice {
    InputDevice {
        kind: InputKind::Keyboard,
        vendor_id: 0x320F,
        product: Some("Fancy Board".to_owned()),
        link: Some(HardwareLink::Usb),
        via: true,
    }
}

fn g_hub_entry() -> UninstallEntry {
    UninstallEntry {
        display_name: "Logitech G HUB".to_owned(),
        display_icon: LGHUB.to_owned(),
        install_location: String::new(),
    }
}

struct Setup {
    hardware: Arc<FakeHardware>,
    opener: Arc<FakeOpener>,
    toml: Option<&'static str>,
    limit: Duration,
    drives: Arc<dyn DriveSource>,
}

impl Setup {
    fn new() -> Self {
        Self {
            hardware: Arc::new(FakeHardware::default()),
            opener: Arc::new(FakeOpener::default()),
            toml: None,
            limit: DEVICE_LIMIT,
            drives: Arc::new(FakeDrives(vec![drive('C', 1000, 400)])),
        }
    }

    async fn start(&self) -> TestBackend {
        let mut builder = TestBackend::builder()
            .with::<dyn Source>(self.hardware.clone())
            .with::<DeviceLimit>(Arc::new(DeviceLimit(self.limit)))
            .with::<dyn DriveSource>(self.drives.clone())
            .opener(self.opener.clone());
        if let Some(text) = self.toml {
            builder = builder.file("hardware.toml", text);
        }
        builder.start().await
    }
}

async fn tiles(backend: &TestBackend) -> Vec<HardwareTile> {
    backend.get("/hardware").await
}

fn tile<'a>(tiles: &'a [HardwareTile], id: &str) -> &'a HardwareTile {
    tiles.iter().find(|tile| tile.id == id).unwrap_or_else(|| {
        panic!(
            "no tile {id} in {:?}",
            tiles.iter().map(|t| &t.id).collect::<Vec<_>>()
        )
    })
}

// ---- what the tiles show ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pc_without_devices_still_has_the_three_tiles() {
    let setup = Setup::new();
    let backend = setup.start().await;
    let tiles = tiles(&backend).await;
    let ids: Vec<_> = tiles.iter().map(|tile| tile.id.as_str()).collect();
    assert_eq!(ids, ["disk", "keyboard", "mouse"]);
    assert!(tile(&tiles, "disk").found);
    assert!(!tile(&tiles, "keyboard").found);
    assert!(!tile(&tiles, "mouse").found);
    assert_eq!(tile(&tiles, "mouse").opens.kind, HardwareOpenKind::None);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_disk_tile_has_the_fixed_drives_and_opens_the_disk_screen() {
    let setup = Setup::new();
    let backend = setup.start().await;
    let tiles = tiles(&backend).await;
    let disk = tile(&tiles, "disk");
    assert_eq!(disk.kind, HardwareKind::Disk);
    assert_eq!(disk.drives.len(), 1);
    assert_eq!(disk.drives[0].total_bytes, 1000);
    assert_eq!(disk.drives[0].used_bytes, 400);
    assert_eq!(disk.opens.kind, HardwareOpenKind::Disk);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_logitech_mouse_shows_its_name_connection_and_battery_and_opens_g_hub() {
    let setup = Setup::new();
    setup.hardware.set_inputs(Inputs {
        devices: vec![],
        endpoints: vec![logitech_endpoint()],
    });
    setup
        .hardware
        .answer(RECEIVER, Duration::ZERO, vec![mouse_reading()]);
    *setup.hardware.installed.lock().unwrap() = vec![g_hub_entry()];
    *setup.hardware.files.lock().unwrap() = vec![PathBuf::from(LGHUB)];
    let backend = setup.start().await;

    let tiles = tiles(&backend).await;
    let mouse = tile(&tiles, "mouse");
    assert!(mouse.found);
    assert_eq!(mouse.product.as_deref(), Some("G PRO X"));
    assert_eq!(mouse.connection, Some(HardwareLink::Receiver));
    let battery = mouse.battery.as_ref().expect("a battery");
    assert_eq!(
        (battery.percent, battery.charging, battery.estimated),
        (87, Some(false), false)
    );
    assert_eq!(mouse.opens.kind, HardwareOpenKind::App);
    assert_eq!(mouse.opens.label.as_deref(), Some("Logitech G HUB"));

    // Enter on the tile starts G HUB.
    let (status, _) = backend.send("POST", "/hardware/mouse/open", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        setup.opener.requests(),
        vec![OpenRequest::ShellExec {
            file: LGHUB.to_owned(),
            params: None,
            dir: None
        }]
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_vendor_program_the_mouse_opens_the_vendor_download_page() {
    let setup = Setup::new();
    setup.hardware.set_inputs(Inputs {
        devices: vec![],
        endpoints: vec![logitech_endpoint()],
    });
    setup
        .hardware
        .answer(RECEIVER, Duration::ZERO, vec![mouse_reading()]);
    let backend = setup.start().await;

    let tiles = tiles(&backend).await;
    assert_eq!(tile(&tiles, "mouse").opens.kind, HardwareOpenKind::Url);
    let (status, _) = backend.send("POST", "/hardware/mouse/open", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let requests = setup.opener.requests();
    let [OpenRequest::ShellExec { file, .. }] = requests.as_slice() else {
        panic!("one request, got {requests:?}");
    };
    assert!(file.starts_with("https://"), "{file}");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_via_keyboard_opens_the_via_web_app_or_the_installed_program() {
    let setup = Setup::new();
    setup.hardware.set_inputs(Inputs {
        devices: vec![via_keyboard()],
        endpoints: vec![],
    });
    let backend = setup.start().await;
    let first = tiles(&backend).await;
    let keyboard = tile(&first, "keyboard");
    assert_eq!(keyboard.product.as_deref(), Some("Fancy Board"));
    assert_eq!(keyboard.connection, Some(HardwareLink::Usb));
    assert_eq!(keyboard.opens.label.as_deref(), Some("usevia.app"));
    assert_eq!(keyboard.battery, None);
    backend.send("POST", "/hardware/keyboard/open", None).await;
    assert_eq!(
        setup.opener.requests(),
        vec![OpenRequest::ShellExec {
            file: "https://usevia.app".to_owned(),
            params: None,
            dir: None
        }]
    );
    backend.stop().await;

    // With the program installed, the program opens, not the web page.
    let setup = Setup::new();
    setup.hardware.set_inputs(Inputs {
        devices: vec![via_keyboard()],
        endpoints: vec![],
    });
    let via = r"C:\Users\me\AppData\Local\Programs\via\VIA.exe";
    *setup.hardware.installed.lock().unwrap() = vec![UninstallEntry {
        display_name: "VIA".to_owned(),
        display_icon: via.to_owned(),
        install_location: String::new(),
    }];
    *setup.hardware.files.lock().unwrap() = vec![PathBuf::from(via)];
    let backend = setup.start().await;
    let second = tiles(&backend).await;
    assert_eq!(tile(&second, "keyboard").opens.kind, HardwareOpenKind::App);
    assert_eq!(
        tile(&second, "keyboard").opens.label.as_deref(),
        Some("VIA")
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_device_of_a_kind_gets_a_numbered_tile() {
    let setup = Setup::new();
    setup.hardware.set_inputs(Inputs {
        devices: vec![
            via_keyboard(),
            InputDevice {
                product: Some("Second".to_owned()),
                via: false,
                ..via_keyboard()
            },
        ],
        endpoints: vec![],
    });
    let backend = setup.start().await;
    let tiles = tiles(&backend).await;
    let ids: Vec<_> = tiles.iter().map(|tile| tile.id.as_str()).collect();
    assert_eq!(ids, ["disk", "keyboard", "keyboard-2", "mouse"]);
    assert_eq!(
        tile(&tiles, "keyboard-2").opens.kind,
        HardwareOpenKind::None
    );
    backend.stop().await;
}

// ---- one slow device does not block the others ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_logitech_channel_costs_only_its_own_tile() {
    let mut setup = Setup::new();
    setup.limit = Duration::from_millis(100);
    setup.hardware.set_inputs(Inputs {
        devices: vec![via_keyboard()],
        endpoints: vec![logitech_endpoint()],
    });
    setup
        .hardware
        .answer(RECEIVER, Duration::from_millis(700), vec![mouse_reading()]);
    let backend = setup.start().await;

    let started = Instant::now();
    let tiles = tiles(&backend).await;
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "the answer waited for the slow device: {:?}",
        started.elapsed()
    );
    assert!(tile(&tiles, "keyboard").found, "the keyboard is there");
    assert!(tile(&tiles, "disk").found, "the disk is there");
    let mouse = tile(&tiles, "mouse");
    assert!(!mouse.found);
    assert_eq!(
        mouse.problem.as_deref(),
        Some("Logitech did not answer in time")
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_drives_leave_the_disk_tile_empty_and_the_rest_alone() {
    let mut setup = Setup::new();
    setup.limit = Duration::from_millis(100);
    setup.drives = Arc::new(SlowDrives);
    setup.hardware.set_inputs(Inputs {
        devices: vec![via_keyboard()],
        endpoints: vec![],
    });
    let backend = setup.start().await;
    let started = Instant::now();
    let tiles = tiles(&backend).await;
    assert!(started.elapsed() < Duration::from_millis(500));
    let disk = tile(&tiles, "disk");
    assert!(!disk.found);
    assert!(disk.problem.is_some());
    assert!(tile(&tiles, "keyboard").found);
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_channel_that_answers_with_nothing_says_so_on_the_mouse_tile() {
    let setup = Setup::new();
    setup.hardware.set_inputs(Inputs {
        devices: vec![],
        endpoints: vec![logitech_endpoint()],
    });
    setup.hardware.answer(RECEIVER, Duration::ZERO, vec![]);
    let backend = setup.start().await;
    let tiles = tiles(&backend).await;
    assert_eq!(
        tile(&tiles, "mouse").problem.as_deref(),
        Some("Logitech found, but no device answered")
    );
    backend.stop().await;
}

// ---- nothing runs until the screen asks ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_is_read_at_start_and_every_get_reads_again() {
    let setup = Setup::new();
    let backend = setup.start().await;
    let reads = || {
        setup
            .hardware
            .reads
            .load(std::sync::atomic::Ordering::SeqCst)
    };
    assert_eq!(reads(), 0, "start reads no device");
    tiles(&backend).await;
    assert_eq!(reads(), 1);
    tiles(&backend).await;
    assert_eq!(reads(), 2, "no cache: the PC is read again");
    backend.stop().await;
}

// ---- hardware.toml ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_example_file_is_written_when_there_is_none() {
    let setup = Setup::new();
    let backend = setup.start().await;
    let text = std::fs::read_to_string(backend.data_dir.join("hardware.toml")).expect("a file");
    assert!(text.starts_with("schema_version = 1"));
    assert!(text.contains("[[tile]]"));
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardware_toml_renames_hides_and_sets_what_a_tile_opens() {
    let mut setup = Setup::new();
    setup.toml = Some(
        "[[tile]]\nid = \"mouse\"\nname = \"Desk mouse\"\nopens = \"none\"\n\n\
         [[tile]]\nid = \"disk\"\nhidden = true\n\n\
         [[tile]]\nid = \"keyboard\"\nopens = \"https://example.com/keys\"\n",
    );
    setup.hardware.set_inputs(Inputs {
        devices: vec![via_keyboard()],
        endpoints: vec![logitech_endpoint()],
    });
    setup
        .hardware
        .answer(RECEIVER, Duration::ZERO, vec![mouse_reading()]);
    let backend = setup.start().await;

    let tiles = tiles(&backend).await;
    assert!(
        tiles.iter().all(|tile| tile.id != "disk"),
        "the disk tile is hidden"
    );
    let mouse = tile(&tiles, "mouse");
    assert_eq!(mouse.name, "Desk mouse");
    assert_eq!(mouse.opens.kind, HardwareOpenKind::None);
    let keyboard = tile(&tiles, "keyboard");
    assert_eq!(keyboard.opens.kind, HardwareOpenKind::Url);
    assert_eq!(keyboard.opens.label.as_deref(), Some("example.com"));

    let (status, body) = backend.send("POST", "/hardware/mouse/open", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["reason"], "nothingToOpen");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_program_path_in_hardware_toml_that_does_not_exist_is_reported_not_started() {
    let mut setup = Setup::new();
    setup.toml = Some("[[tile]]\nid = \"keyboard\"\nopens = \"C:\\\\Nope\\\\app.exe\"\n");
    setup.hardware.set_inputs(Inputs {
        devices: vec![via_keyboard()],
        endpoints: vec![],
    });
    let backend = setup.start().await;
    let tiles = tiles(&backend).await;
    let keyboard = tile(&tiles, "keyboard");
    assert_eq!(keyboard.opens.kind, HardwareOpenKind::None);
    assert_eq!(
        keyboard.problem.as_deref(),
        Some("The program in hardware.toml was not found")
    );
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_broken_hardware_toml_keeps_the_screen_working_with_the_defaults() {
    let mut setup = Setup::new();
    setup.toml = Some("[[tile]\nid = ");
    let backend = setup.start().await;
    let tiles = tiles(&backend).await;
    assert_eq!(tiles.len(), 3);
    backend.stop().await;
}

// ---- the open route ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_a_tile_before_the_screen_read_anything_is_a_conflict() {
    let setup = Setup::new();
    let backend = setup.start().await;
    let (status, body) = backend.send("POST", "/hardware/mouse/open", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["reason"], "hardwareNotRead");
    assert!(setup.opener.requests().is_empty());
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_disk_tile_has_nothing_for_the_backend_to_start() {
    let setup = Setup::new();
    let backend = setup.start().await;
    tiles(&backend).await;
    let (status, _) = backend.send("POST", "/hardware/disk/open", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(setup.opener.requests().is_empty());
    backend.stop().await;
}

// ---- the real PC ----

/// Reads this PC for real and prints the answer. Run by hand:
/// `cargo test -p rain-backend --test hardware real_pc -- --ignored --nocapture`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "reads the real devices of this PC"]
async fn real_pc() {
    let backend = TestBackend::builder()
        .with::<dyn Source>(Arc::new(SystemSource))
        .start()
        .await;
    let started = Instant::now();
    let answer: Value = backend.get("/hardware").await;
    println!("took {:?}", started.elapsed());
    println!("{}", serde_json::to_string_pretty(&answer).unwrap());
    assert!(answer.as_array().is_some_and(|tiles| !tiles.is_empty()));
    backend.stop().await;
}
