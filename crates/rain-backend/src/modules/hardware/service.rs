//! Reading the hardware, and opening what a tile opens.
//!
//! `read` looks at the PC at the moment it is called and keeps nothing but the targets of the
//! tiles it returned (the screen's Enter asks for one of them). No thread, no timer, no cache.
//! Each source of a reading has its own time limit, so that a drive that sleeps or a device
//! that does not answer costs only its own tile.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use rain_types::{
    Drive, HardwareBattery, HardwareKind, HardwareLink, HardwareOpenKind, HardwareOpens,
    HardwareTile,
};
use tracing::warn;

use super::hidpp::{Battery, DeviceKind, Reading};
use super::inputs::{Endpoint, InputKind, Inputs};
use super::model::{self, HardwareFile, OpensRule};
use super::source::Source;
use super::vendor::{AppHint, VIA_APP, VIA_WEB, Vendor, vendor_of};
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::core::launch_contract::OpenRequest;
use crate::core::state::{AppState, WeakAppState};
use crate::platform::hid::Bus;
use crate::platform::registry::UninstallEntry;
use crate::store::toml_file::TomlFile;

const SOURCE: &str = "hardware";

/// How long one source (the drives, the list of devices, one Logitech channel) gets.
pub const DEVICE_LIMIT: Duration = Duration::from_millis(1500);

/// A test replaces the limit with this, through the seam of the state.
pub struct DeviceLimit(pub Duration);

/// What Enter does on one tile.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Disk,
    Program(PathBuf),
    Url(String),
    Nothing,
}

/// A keyboard or a mouse that was found, from either way of finding it.
struct Found {
    kind: HardwareKind,
    product: Option<String>,
    link: Option<HardwareLink>,
    battery: Option<Battery>,
    vendor: Option<&'static Vendor>,
    via: bool,
}

pub struct Service {
    state: WeakAppState,
    file: TomlFile<HardwareFile>,
    source: Arc<dyn Source>,
    limit: Duration,
    /// The targets of the tiles of the newest `read`, by tile id.
    targets: Mutex<HashMap<String, Target>>,
}

impl Service {
    /// Opens `hardware.toml` (and writes the commented example when there is none). Reads no
    /// device.
    pub async fn open(state: &AppState, source: Arc<dyn Source>) -> anyhow::Result<Self> {
        let file = TomlFile::<HardwareFile>::open(&state.hub, model::FILE_NAME, model::validate)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.body.message))?;
        if !state.hub.file_path(model::FILE_NAME).exists() {
            // The only write of this file by Rain: the example, so the owner can find the options.
            let written = file
                .edit(|document| {
                    *document = model::EXAMPLE_FILE
                        .parse()
                        .map_err(|_| ApiError::failed("The example of hardware.toml is broken"))?;
                    Ok(())
                })
                .await;
            if let Err(e) = written {
                warn!(reason = ?e.body.reason, "could not write the example hardware.toml");
            }
        }
        Ok(Self {
            state: state.downgrade(),
            file,
            source,
            limit: state
                .seam::<DeviceLimit>()
                .map_or(DEVICE_LIMIT, |limit| limit.0),
            targets: Mutex::new(HashMap::new()),
        })
    }

    fn state(&self) -> ApiResult<AppState> {
        self.state.upgrade().ok_or_else(|| {
            ApiError::unavailable(SOURCE, "Rain is stopping").with_reason("shuttingDown")
        })
    }

    fn targets(&self) -> MutexGuard<'_, HashMap<String, Target>> {
        self.targets.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // ---- GET /hardware ----

    pub async fn read(&self) -> ApiResult<Vec<HardwareTile>> {
        let state = self.state()?;
        // A hand edit of hardware.toml is picked up here, so it also works without the watcher.
        self.file.refresh().await;
        let rules = self.file.get();

        // The three first readings run side by side, each with its own limit.
        let disk = state.disk();
        let (drives, inputs, installed) = tokio::join!(
            within(self.limit, async { disk.list_drives().await.ok() }),
            within(
                self.limit,
                run_blocking({
                    let source = Arc::clone(&self.source);
                    move || source.inputs()
                })
            ),
            within(
                self.limit,
                run_blocking({
                    let source = Arc::clone(&self.source);
                    move || source.installed()
                })
            ),
        );
        let inputs = inputs.unwrap_or_default();
        let installed = installed.unwrap_or_default();

        // Then every Logitech channel, side by side, each with its own limit.
        // The reader stops a little before the limit, so that what it has read so far is returned
        // and not thrown away with the timeout.
        let deadline = Instant::now() + self.limit.mul_f32(0.85);
        let asked = join_all(inputs.endpoints.iter().map(|endpoint| {
            let source = Arc::clone(&self.source);
            let endpoint = endpoint.clone();
            let limit = self.limit;
            async move {
                let readings = within(
                    limit,
                    run_blocking({
                        let endpoint = endpoint.clone();
                        move || source.ask(&endpoint, deadline)
                    }),
                )
                .await;
                (endpoint, readings)
            }
        }))
        .await;

        let tiles = self.compose(&rules, drives, &inputs, &asked, &installed);
        let mut targets = self.targets();
        targets.clear();
        let mut out = Vec::with_capacity(tiles.len());
        for (tile, target) in tiles {
            targets.insert(tile.id.clone(), target);
            out.push(tile);
        }
        Ok(out)
    }

    /// Turns the readings into tiles, applies `hardware.toml`, and works out what each opens.
    fn compose(
        &self,
        rules: &HardwareFile,
        drives: Option<Vec<Drive>>,
        inputs: &Inputs,
        asked: &[(Endpoint, Option<Vec<Reading>>)],
        installed: &[UninstallEntry],
    ) -> Vec<(HardwareTile, Target)> {
        let exists = |path: &std::path::Path| self.source.exists(path);
        let mut tiles = Vec::new();

        // The disk tile.
        let drive_problem = drives
            .is_none()
            .then(|| "The drives did not answer in time".to_owned());
        let drives = drives.unwrap_or_default();
        tiles.push((
            HardwareTile {
                id: "disk".to_owned(),
                kind: HardwareKind::Disk,
                name: title(HardwareKind::Disk).to_owned(),
                found: !drives.is_empty(),
                product: None,
                connection: None,
                battery: None,
                drives,
                opens: opens(HardwareOpenKind::Disk, Some("Disk")),
                problem: drive_problem,
            },
            Target::Disk,
        ));

        // The keyboards and mice, from the plain interfaces and from the Logitech answers.
        let mut found: Vec<Found> = inputs
            .devices
            .iter()
            .map(|device| Found {
                kind: match device.kind {
                    InputKind::Keyboard => HardwareKind::Keyboard,
                    InputKind::Mouse => HardwareKind::Mouse,
                },
                product: device.product.clone(),
                link: device.link,
                battery: None,
                vendor: vendor_of(device.vendor_id),
                via: device.via,
            })
            .collect();
        // Why a Logitech channel that gave no keyboard or mouse gave none.
        let mut missing: Vec<String> = Vec::new();
        for (endpoint, readings) in asked {
            let vendor = vendor_of(endpoint.vendor_id);
            let name = vendor.map_or("The device", |vendor| vendor.name);
            let mut any = false;
            match readings {
                Some(readings) => {
                    for reading in readings {
                        let kind = match reading.kind {
                            DeviceKind::Keyboard => HardwareKind::Keyboard,
                            DeviceKind::Mouse => HardwareKind::Mouse,
                            DeviceKind::Other => continue,
                        };
                        any = true;
                        found.push(Found {
                            kind,
                            product: reading.name.clone(),
                            link: Some(match (reading.receiver, endpoint.bus) {
                                (true, _) => HardwareLink::Receiver,
                                (false, Bus::Bluetooth) => HardwareLink::Bluetooth,
                                (false, _) => HardwareLink::Usb,
                            }),
                            battery: reading.battery,
                            vendor,
                            via: false,
                        });
                    }
                    if !any {
                        missing.push(format!("{name} found, but no device answered"));
                    }
                }
                None => missing.push(format!("{name} did not answer in time")),
            }
        }
        // The reason goes on the first empty tile, the mouse before the keyboard: a Logitech
        // receiver is far more often a mouse than a keyboard.
        let owner = [HardwareKind::Mouse, HardwareKind::Keyboard]
            .into_iter()
            .find(|kind| !found.iter().any(|device| device.kind == *kind));
        // A duplicate (two channels of one device that both answered) is shown once.
        let mut seen: Vec<(HardwareKind, Option<String>, Option<HardwareLink>)> = Vec::new();
        found.retain(|device| {
            let key = (device.kind, device.product.clone(), device.link);
            let new = !seen.contains(&key);
            seen.push(key);
            new
        });

        for kind in [HardwareKind::Keyboard, HardwareKind::Mouse] {
            let mine: Vec<&Found> = found.iter().filter(|device| device.kind == kind).collect();
            if mine.is_empty() {
                let problem = if owner == Some(kind) {
                    missing.first().cloned()
                } else {
                    None
                };
                tiles.push((
                    HardwareTile {
                        id: id_of(kind, 0),
                        kind,
                        name: title(kind).to_owned(),
                        found: false,
                        product: None,
                        connection: None,
                        battery: None,
                        drives: vec![],
                        opens: opens(HardwareOpenKind::None, None),
                        problem,
                    },
                    Target::Nothing,
                ));
                continue;
            }
            for (number, device) in mine.into_iter().enumerate() {
                let (target, label) = target_of(device, installed, &exists);
                tiles.push((
                    HardwareTile {
                        id: id_of(kind, number),
                        kind,
                        name: title(kind).to_owned(),
                        found: true,
                        product: device.product.clone(),
                        connection: device.link,
                        battery: device.battery.map(|battery| HardwareBattery {
                            percent: battery.percent,
                            charging: battery.charging,
                            estimated: battery.estimated,
                        }),
                        drives: vec![],
                        opens: opens(open_kind(&target), label.as_deref()),
                        problem: None,
                    },
                    target,
                ));
            }
        }

        apply_rules(rules, tiles, &exists)
    }

    // ---- POST /hardware/{id}/open ----

    pub async fn open_tile(&self, id: &str) -> ApiResult<()> {
        let target = self.targets().get(id).cloned().ok_or_else(|| {
            ApiError::conflict(
                "hardwareNotRead",
                "Reload the Hardware screen, then try again.",
            )
        })?;
        let file = match target {
            // The screen shows its own Disk screen: there is nothing for the backend to start.
            Target::Disk => return Ok(()),
            Target::Nothing => {
                return Err(ApiError::conflict(
                    "nothingToOpen",
                    "Rain does not know what opens for this device. Set it in hardware.toml.",
                ));
            }
            Target::Program(path) => path.to_string_lossy().into_owned(),
            Target::Url(url) => url,
        };
        let state = self.state()?;
        let request = OpenRequest::ShellExec {
            file,
            params: None,
            dir: None,
        };
        match state.opener().open(request).await {
            Ok(_) => Ok(()),
            Err(e) => {
                warn!(code = ?e.code, reason = %e.log_reason, "a hardware tile could not be opened");
                Err(ApiError::failed(e.ui_text)
                    .with_source(SOURCE)
                    .with_reason("openFailed"))
            }
        }
    }
}

/// `blocking::run` as a future of an option (`None` when the task ended abnormally), for `within`.
async fn run_blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    blocking::run(work).await.ok()
}

/// Runs `work` for at most `limit`. `None` when it ends without a value or runs out of time.
async fn within<T>(limit: Duration, work: impl Future<Output = Option<T>>) -> Option<T> {
    tokio::time::timeout(limit, work).await.ok().flatten()
}

fn title(kind: HardwareKind) -> &'static str {
    match kind {
        HardwareKind::Disk => "Disk",
        HardwareKind::Keyboard => "Keyboard",
        HardwareKind::Mouse => "Mouse",
    }
}

/// `keyboard` for the first device of a kind, `keyboard-2` for the second.
fn id_of(kind: HardwareKind, number: usize) -> String {
    let base = match kind {
        HardwareKind::Disk => "disk",
        HardwareKind::Keyboard => "keyboard",
        HardwareKind::Mouse => "mouse",
    };
    if number == 0 {
        base.to_owned()
    } else {
        format!("{base}-{}", number + 1)
    }
}

fn opens(kind: HardwareOpenKind, label: Option<&str>) -> HardwareOpens {
    HardwareOpens {
        kind,
        label: label.map(str::to_owned),
    }
}

fn open_kind(target: &Target) -> HardwareOpenKind {
    match target {
        Target::Disk => HardwareOpenKind::Disk,
        Target::Program(_) => HardwareOpenKind::App,
        Target::Url(_) => HardwareOpenKind::Url,
        Target::Nothing => HardwareOpenKind::None,
    }
}

/// What a device opens by default: the program of its vendor when it is installed, else the
/// vendor's download page; for a VIA keyboard the VIA program, else the VIA web app.
fn target_of(
    device: &Found,
    installed: &[UninstallEntry],
    exists: &dyn Fn(&std::path::Path) -> bool,
) -> (Target, Option<String>) {
    let first_app = |apps: &[&AppHint]| {
        apps.iter()
            .find_map(|app| app.find(installed, exists).map(|path| (path, app.label)))
    };
    if device.via {
        return match first_app(&[&VIA_APP]) {
            Some((path, label)) => (Target::Program(path), Some(label.to_owned())),
            None => (
                Target::Url(VIA_WEB.to_owned()),
                Some("usevia.app".to_owned()),
            ),
        };
    }
    if let Some(vendor) = device.vendor {
        let apps: Vec<&AppHint> = vendor.apps.iter().collect();
        return match first_app(&apps) {
            Some((path, label)) => (Target::Program(path), Some(label.to_owned())),
            None => (
                Target::Url(vendor.download.to_owned()),
                Some(format!("{} download", vendor.name)),
            ),
        };
    }
    (Target::Nothing, None)
}

/// Renames, hides and re-targets the tiles by the rules of `hardware.toml`.
fn apply_rules(
    rules: &HardwareFile,
    tiles: Vec<(HardwareTile, Target)>,
    exists: &dyn Fn(&std::path::Path) -> bool,
) -> Vec<(HardwareTile, Target)> {
    let mut out = Vec::with_capacity(tiles.len());
    for (mut tile, mut target) in tiles {
        let kind = match tile.kind {
            HardwareKind::Disk => "disk",
            HardwareKind::Keyboard => "keyboard",
            HardwareKind::Mouse => "mouse",
        };
        if rules
            .setting(&tile.id, kind, |rule| rule.hidden)
            .unwrap_or(false)
        {
            continue;
        }
        if let Some(name) = rules.setting(&tile.id, kind, |rule| rule.name.clone()) {
            tile.name = name.trim().to_owned();
        }
        if let Some(text) = rules.setting(&tile.id, kind, |rule| rule.opens.clone()) {
            let (new_target, label) = match model::parse_opens(&text) {
                OpensRule::Disk => (Target::Disk, Some("Disk".to_owned())),
                OpensRule::Nothing => (Target::Nothing, None),
                OpensRule::Url(url) => {
                    let host = url
                        .split("//")
                        .nth(1)
                        .and_then(|rest| rest.split('/').next())
                        .unwrap_or(&url)
                        .to_owned();
                    (Target::Url(url), Some(host))
                }
                OpensRule::Program(path) => {
                    let path = PathBuf::from(path);
                    let label = path
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned());
                    if exists(&path) {
                        (Target::Program(path), label)
                    } else {
                        tile.problem =
                            Some("The program in hardware.toml was not found".to_owned());
                        (Target::Nothing, None)
                    }
                }
            };
            tile.opens = opens(open_kind(&new_target), label.as_deref());
            target = new_target;
        }
        out.push((tile, target));
    }
    out
}
