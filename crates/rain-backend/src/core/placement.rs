//! Putting the window of a launched step into its zone of the screen. Section 02 6.17.
//!
//! Everything here is plain logic that a test drives with a fake `Desktop`: the zone as a
//! rectangle, the choice of the monitor, the correction for the invisible border of a window, the
//! rules that tell which window a started program made, and the wait for that window. The real
//! `Desktop` (windows, monitors, `SetWindowPos`) is `platform::windows_place`.
//!
//! The rules in short:
//! - A snapshot of the visible windows is taken before a step starts. The window of the step is a
//!   window that is not in the snapshot and that belongs to the started program: its process id,
//!   or one of the image names that the plan lists (Windows Terminal and VS Code hand the work to
//!   a process that runs already). A title rule narrows it further.
//! - A window that does not appear within `WAIT` is left alone and reported. Nothing is retried,
//!   and a window that an earlier step of the run placed is never taken again.
//! - A window that is open already counts only when the plan says so (`allow_existing`), and then
//!   only when its title matches.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use rain_types::LayoutZone;
use tokio::time::Instant;

use super::blocking;
use super::launch_contract::{Placement, WindowQuery};

/// How long a step gets to show its window.
pub const WAIT: Duration = Duration::from_secs(8);
/// How often the windows are listed while waiting.
pub const POLL: Duration = Duration::from_millis(150);
/// How long a placed window is left before its position is checked once more. Some programs
/// move their window to the place they remember a moment after they showed it.
pub const SETTLE: Duration = Duration::from_millis(350);

// ---- geometry ----

/// A rectangle in physical pixels: `right` and `bottom` are the first pixel outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// How far a point is from the rectangle: 0 inside it.
    fn distance_to(&self, (x, y): (i32, i32)) -> i64 {
        let dx = i64::from((self.left - x).max(x - (self.right - 1)).max(0));
        let dy = i64::from((self.top - y).max(y - (self.bottom - 1)).max(0));
        dx * dx + dy * dy
    }
}

/// A monitor as the placement sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// The work area: the monitor without the taskbar, in physical pixels of the whole desktop.
    pub work: Rect,
    pub primary: bool,
    /// The scale of this monitor in dots per inch (96 is 100 percent). Zones do not depend on it:
    /// the work area is already in physical pixels.
    pub dpi: u32,
}

/// The monitors in the order that `projects.toml` counts them: from the left edge to the right,
/// and from the top down for monitors that start at the same place. Monitor 1 is the first.
pub fn number_monitors(mut monitors: Vec<MonitorInfo>) -> Vec<MonitorInfo> {
    monitors.sort_by_key(|m| (m.work.left, m.work.top));
    monitors
}

/// The part of `work` that a zone covers. `None` for `Free`: nothing is moved. Neighbouring zones
/// share their edge exactly, also when the size is odd: no gap and no overlap.
pub fn zone_rect(zone: LayoutZone, work: Rect) -> Option<Rect> {
    let (w, h) = (work.width(), work.height());
    // The x or y of the `n`-th of `parts` equal slices, rounded down: slice edges are shared.
    let x = |n: i32, parts: i32| work.left + w * n / parts;
    let y = |n: i32, parts: i32| work.top + h * n / parts;
    let rect = match zone {
        LayoutZone::Free => return None,
        LayoutZone::Full => work,
        LayoutZone::LeftHalf => Rect::new(work.left, work.top, x(1, 2), work.bottom),
        LayoutZone::RightHalf => Rect::new(x(1, 2), work.top, work.right, work.bottom),
        LayoutZone::TopHalf => Rect::new(work.left, work.top, work.right, y(1, 2)),
        LayoutZone::BottomHalf => Rect::new(work.left, y(1, 2), work.right, work.bottom),
        LayoutZone::TopLeft => Rect::new(work.left, work.top, x(1, 2), y(1, 2)),
        LayoutZone::TopRight => Rect::new(x(1, 2), work.top, work.right, y(1, 2)),
        LayoutZone::BottomLeft => Rect::new(work.left, y(1, 2), x(1, 2), work.bottom),
        LayoutZone::BottomRight => Rect::new(x(1, 2), y(1, 2), work.right, work.bottom),
        LayoutZone::LeftThird => Rect::new(work.left, work.top, x(1, 3), work.bottom),
        LayoutZone::MiddleThird => Rect::new(x(1, 3), work.top, x(2, 3), work.bottom),
        LayoutZone::RightThird => Rect::new(x(2, 3), work.top, work.right, work.bottom),
        LayoutZone::LeftTwoThirds => Rect::new(work.left, work.top, x(2, 3), work.bottom),
        LayoutZone::RightTwoThirds => Rect::new(x(1, 3), work.top, work.right, work.bottom),
    };
    Some(rect)
}

/// Which monitor a zone is on. `numbered` is the list of `number_monitors`.
///
/// `wanted` is the number in the layout (1 is the first). Without one, or when the PC has no such
/// monitor, the monitor of `anchor` is used (a point: Rain's window, else the mouse), else the
/// primary monitor. The second value is true when a number was asked for and does not exist.
pub fn pick_monitor(
    numbered: &[MonitorInfo],
    wanted: Option<u32>,
    anchor: Option<(i32, i32)>,
) -> Option<(usize, bool)> {
    if numbered.is_empty() {
        return None;
    }
    let missing = match wanted {
        Some(number) if number >= 1 && (number as usize) <= numbered.len() => {
            return Some((number as usize - 1, false));
        }
        Some(_) => true,
        None => false,
    };
    let near_anchor = anchor.and_then(|point| {
        numbered
            .iter()
            .enumerate()
            .min_by_key(|(_, m)| m.work.distance_to(point))
            .map(|(index, _)| index)
    });
    let index = near_anchor
        .or_else(|| numbered.iter().position(|m| m.primary))
        .unwrap_or(0);
    Some((index, missing))
}

/// The outer rectangle to give `SetWindowPos` so that the VISIBLE window fills `target`.
///
/// A window of Windows 10 and 11 has an invisible resize border: its outer rectangle (what
/// `GetWindowRect` says) is a few pixels larger than what the eye sees (the extended frame bounds
/// of DWM). Each edge of the outer rectangle is moved by the width of the border on that edge.
pub fn outer_for_visible(target: Rect, outer: Rect, visible: Rect) -> Rect {
    Rect {
        left: target.left - (visible.left - outer.left),
        top: target.top - (visible.top - outer.top),
        right: target.right + (outer.right - visible.right),
        bottom: target.bottom + (outer.bottom - visible.bottom),
    }
}

// ---- which window ----

/// A visible top-level window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenWindow {
    pub hwnd: isize,
    pub pid: u32,
    /// The image name of the owner, lower case, like `code.exe`. Empty when it is not known.
    pub image: String,
    pub title: String,
}

/// Whether a window title names `text` (lower case) as one of its parts. The parts are split at
/// " - ", like "file - folder - Visual Studio Code". A part names the text when it is the text, or
/// starts with it and goes on with a space or a bracket ("web-shop (Workspace)"). A plain
/// substring would take "rain-data" for "rain".
fn title_has(title: &str, text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    title.to_lowercase().split(" - ").any(|part| {
        let part = part.trim().trim_start_matches(['\u{25cf}', '*']).trim();
        part.strip_prefix(text)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '(', '[']))
    })
}

/// The window that a step made, or `None`.
///
/// A candidate belongs to the started program (its process id, or an image name of the query),
/// is not claimed by an earlier step of the run, and has the title text when the query asks for
/// one. A window that the snapshot did not hold wins; the topmost of them when there are several.
/// A window that was there before counts only for `allow_existing` with a title rule.
pub fn find_window(
    before: &[SeenWindow],
    now: &[SeenWindow],
    claimed: &HashSet<isize>,
    query: &WindowQuery,
    started_pid: Option<u32>,
) -> Option<isize> {
    let known: HashSet<isize> = before.iter().map(|w| w.hwnd).collect();
    let candidate = |w: &&SeenWindow| {
        if claimed.contains(&w.hwnd) {
            return false;
        }
        let title_ok = query
            .title_has
            .as_deref()
            .is_none_or(|text| title_has(&w.title, text));
        // The started process owns the window itself: its title needs no rule.
        let own = started_pid.is_some_and(|pid| pid != 0 && pid == w.pid);
        // A query without image names but with a title rule (a store app, whose window belongs to
        // a frame host) is told by the title alone.
        let image = if query.images.is_empty() {
            query.title_has.is_some()
        } else {
            !w.image.is_empty() && query.images.contains(&w.image)
        };
        own || (image && title_ok)
    };
    let mut candidates = now.iter().filter(candidate);
    if let Some(fresh) = candidates.clone().find(|w| !known.contains(&w.hwnd)) {
        return Some(fresh.hwnd);
    }
    if query.allow_existing && query.title_has.is_some() {
        return candidates.next().map(|w| w.hwnd);
    }
    None
}

// ---- the desktop ----

/// What the placement needs from Windows. Every method blocks for a few milliseconds.
pub trait Desktop: Send + Sync {
    /// The visible top-level windows that a person would call windows, the topmost first.
    fn windows(&self) -> Vec<SeenWindow>;
    /// The monitors, in any order.
    fn monitors(&self) -> Vec<MonitorInfo>;
    /// A point on the screen that tells which monitor "this one" is: the middle of Rain's window,
    /// else the mouse.
    fn anchor(&self) -> Option<(i32, i32)>;
    /// Restores the window when it is maximised or minimised, and moves it so that its visible
    /// part fills `visible`.
    fn place(&self, hwnd: isize, visible: Rect) -> Result<(), String>;
    /// The visible part of the window now, `None` when the window is gone.
    fn visible_frame(&self, hwnd: isize) -> Option<Rect>;
}

/// Puts the windows of the steps of one run in their zones.
pub struct Placer {
    desktop: Arc<dyn Desktop>,
    wait: Duration,
    poll: Duration,
    settle: Duration,
}

impl Placer {
    pub fn new(desktop: Arc<dyn Desktop>) -> Self {
        Self::with_timing(desktop, WAIT, POLL, SETTLE)
    }

    /// The same with other times, so that a test does not wait for seconds.
    pub fn with_timing(
        desktop: Arc<dyn Desktop>,
        wait: Duration,
        poll: Duration,
        settle: Duration,
    ) -> Self {
        Self {
            desktop,
            wait,
            poll,
            settle,
        }
    }

    /// How many monitors the PC has now.
    pub async fn monitor_count(&self) -> u32 {
        let desktop = Arc::clone(&self.desktop);
        let monitors = blocking::run(move || desktop.monitors())
            .await
            .unwrap_or_default();
        u32::try_from(monitors.len()).unwrap_or(u32::MAX)
    }

    /// Starts a run. The point that says which monitor "this one" is is taken now, before any
    /// window of the run changes where the mouse or the focus is.
    pub async fn begin(&self) -> PlacementRun {
        let desktop = Arc::clone(&self.desktop);
        let anchor = blocking::run(move || desktop.anchor())
            .await
            .unwrap_or_default();
        PlacementRun {
            desktop: Arc::clone(&self.desktop),
            anchor,
            claimed: HashSet::new(),
            wait: self.wait,
            poll: self.poll,
            settle: self.settle,
        }
    }
}

/// The windows of one run: the anchor, and the windows that are placed already.
pub struct PlacementRun {
    desktop: Arc<dyn Desktop>,
    anchor: Option<(i32, i32)>,
    claimed: HashSet<isize>,
    wait: Duration,
    poll: Duration,
    settle: Duration,
}

impl PlacementRun {
    /// The visible windows now. Call it right before a step starts.
    pub async fn snapshot(&self) -> Vec<SeenWindow> {
        let desktop = Arc::clone(&self.desktop);
        blocking::run(move || desktop.windows())
            .await
            .unwrap_or_default()
    }

    /// Waits for the window of a step that started, and moves it. `None` when the window is in
    /// its zone; else the note for the launch result: what was not done, in plain words.
    pub async fn place(
        &mut self,
        before: &[SeenWindow],
        placement: &Placement,
        started_pid: Option<u32>,
    ) -> Option<String> {
        let desktop = Arc::clone(&self.desktop);
        let monitors = blocking::run(move || desktop.monitors())
            .await
            .unwrap_or_default();
        let numbered = number_monitors(monitors);
        let Some((index, missing)) = pick_monitor(&numbered, placement.monitor, self.anchor) else {
            return Some("No monitor found".to_owned());
        };
        let target = zone_rect(placement.zone, numbered[index].work)?;

        let Some(hwnd) = self.wait_for(before, placement, started_pid).await else {
            return Some("Its window did not show up, so it was left where it is".to_owned());
        };
        self.claimed.insert(hwnd);
        if let Err(text) = self.move_to(hwnd, target).await {
            return Some(format!("Could not move its window: {text}"));
        }
        // Some programs put their window back to the place they remember. Look once more.
        tokio::time::sleep(self.settle).await;
        let desktop = Arc::clone(&self.desktop);
        let frame = blocking::run(move || desktop.visible_frame(hwnd))
            .await
            .ok()
            .flatten();
        if frame.is_some_and(|frame| frame != target) {
            let _ = self.move_to(hwnd, target).await;
        }
        missing.then(|| {
            let wanted = placement.monitor.unwrap_or_default();
            format!("Monitor {wanted} is not connected, so it went to this screen")
        })
    }

    async fn wait_for(
        &self,
        before: &[SeenWindow],
        placement: &Placement,
        started_pid: Option<u32>,
    ) -> Option<isize> {
        let deadline = Instant::now() + self.wait;
        loop {
            let now = self.snapshot().await;
            if let Some(hwnd) =
                find_window(before, &now, &self.claimed, &placement.find, started_pid)
            {
                return Some(hwnd);
            }
            if Instant::now() + self.poll > deadline {
                return None;
            }
            tokio::time::sleep(self.poll).await;
        }
    }

    async fn move_to(&self, hwnd: isize, target: Rect) -> Result<(), String> {
        let desktop = Arc::clone(&self.desktop);
        blocking::run(move || desktop.place(hwnd, target))
            .await
            .map_err(|_| "Rain had an internal error".to_owned())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect::new(left, top, right, bottom)
    }

    // ---- zones ----

    /// 1920 x 1080 with the taskbar (40 px) at the bottom, at the top, on the left, on the right.
    const TASKBARS: [Rect; 4] = [
        Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        },
        Rect {
            left: 0,
            top: 40,
            right: 1920,
            bottom: 1080,
        },
        Rect {
            left: 60,
            top: 0,
            right: 1920,
            bottom: 1080,
        },
        Rect {
            left: 0,
            top: 0,
            right: 1860,
            bottom: 1080,
        },
    ];

    #[test]
    fn free_moves_nothing_and_full_is_the_work_area() {
        for work in TASKBARS {
            assert_eq!(zone_rect(LayoutZone::Free, work), None);
            assert_eq!(zone_rect(LayoutZone::Full, work), Some(work));
        }
    }

    #[test]
    fn the_halves_and_quadrants_tile_the_work_area_with_a_taskbar_on_any_side() {
        for work in TASKBARS {
            let zone = |z| zone_rect(z, work).unwrap();
            let (l, rt) = (zone(LayoutZone::LeftHalf), zone(LayoutZone::RightHalf));
            assert_eq!(
                (l.left, l.top, l.bottom),
                (work.left, work.top, work.bottom)
            );
            assert_eq!(l.right, rt.left, "no gap and no overlap");
            assert_eq!(
                (rt.right, rt.top, rt.bottom),
                (work.right, work.top, work.bottom)
            );
            let (t, b) = (zone(LayoutZone::TopHalf), zone(LayoutZone::BottomHalf));
            assert_eq!((t.top, t.left, t.right), (work.top, work.left, work.right));
            assert_eq!(t.bottom, b.top);
            assert_eq!(b.bottom, work.bottom);
            let corners = [
                zone(LayoutZone::TopLeft),
                zone(LayoutZone::TopRight),
                zone(LayoutZone::BottomLeft),
                zone(LayoutZone::BottomRight),
            ];
            let area: i64 = corners
                .iter()
                .map(|c| i64::from(c.width()) * i64::from(c.height()))
                .sum();
            assert_eq!(area, i64::from(work.width()) * i64::from(work.height()));
            assert_eq!(corners[0].right, corners[1].left);
            assert_eq!(corners[0].bottom, corners[2].top);
        }
        // The exact numbers for one of them: taskbar on the left.
        let work = TASKBARS[2];
        assert_eq!(
            zone_rect(LayoutZone::LeftHalf, work),
            Some(r(60, 0, 990, 1080))
        );
        assert_eq!(
            zone_rect(LayoutZone::RightHalf, work),
            Some(r(990, 0, 1920, 1080))
        );
    }

    #[test]
    fn thirds_and_two_thirds_share_their_edges() {
        let work = r(0, 0, 1921, 1080); // an odd width
        let zone = |z| zone_rect(z, work).unwrap();
        let (a, b, c) = (
            zone(LayoutZone::LeftThird),
            zone(LayoutZone::MiddleThird),
            zone(LayoutZone::RightThird),
        );
        assert_eq!(a.left, 0);
        assert_eq!(a.right, b.left);
        assert_eq!(b.right, c.left);
        assert_eq!(c.right, 1921);
        let (two_left, two_right) = (
            zone(LayoutZone::LeftTwoThirds),
            zone(LayoutZone::RightTwoThirds),
        );
        assert_eq!(two_left.right, b.right);
        assert_eq!(two_right.left, a.right);
        assert_eq!(two_right.right, 1921);
    }

    #[test]
    fn an_odd_work_area_and_a_monitor_left_of_the_primary_work() {
        // A second monitor left of the primary one has negative coordinates; 1081 is odd.
        let work = r(-1921, -20, 0, 1061);
        let left = zone_rect(LayoutZone::LeftHalf, work).unwrap();
        let right = zone_rect(LayoutZone::RightHalf, work).unwrap();
        assert_eq!(left.left, -1921);
        assert_eq!(left.right, right.left);
        assert_eq!(right.right, 0);
        let top = zone_rect(LayoutZone::TopHalf, work).unwrap();
        let bottom = zone_rect(LayoutZone::BottomHalf, work).unwrap();
        assert_eq!(
            (top.top, top.bottom, bottom.top, bottom.bottom),
            (-20, top.bottom, top.bottom, 1061)
        );
    }

    #[test]
    fn a_portrait_monitor_keeps_its_own_proportions() {
        let work = r(1920, 0, 3000, 1880); // 1080 x 1880, next to a landscape monitor
        let top = zone_rect(LayoutZone::TopHalf, work).unwrap();
        let bottom = zone_rect(LayoutZone::BottomHalf, work).unwrap();
        assert_eq!((top.width(), top.height()), (1080, 940));
        assert_eq!((bottom.top, bottom.height()), (940, 940));
        let corner = zone_rect(LayoutZone::BottomRight, work).unwrap();
        assert_eq!(corner, r(2460, 940, 3000, 1880));
    }

    // ---- monitors ----

    fn monitor(work: Rect, primary: bool, dpi: u32) -> MonitorInfo {
        MonitorInfo { work, primary, dpi }
    }

    /// A 4K monitor at 200 percent, the primary one, and a full HD monitor at 100 percent to its
    /// left. Both work areas are in physical pixels.
    fn two_monitors() -> Vec<MonitorInfo> {
        vec![
            monitor(r(0, 0, 3840, 2080), true, 192),
            monitor(r(-1920, 200, 0, 1240), false, 96),
        ]
    }

    #[test]
    fn monitors_are_numbered_from_the_left() {
        let numbered = number_monitors(two_monitors());
        assert_eq!(numbered[0].work.left, -1920);
        assert!(numbered[1].primary);
    }

    #[test]
    fn each_monitor_is_tiled_by_its_own_physical_size_whatever_its_scale() {
        let numbered = number_monitors(two_monitors());
        let hd = zone_rect(LayoutZone::RightHalf, numbered[0].work).unwrap();
        let uhd = zone_rect(LayoutZone::RightHalf, numbered[1].work).unwrap();
        assert_eq!(hd, r(-960, 200, 0, 1240));
        assert_eq!(uhd, r(1920, 0, 3840, 2080));
    }

    #[test]
    fn a_numbered_monitor_wins_and_a_missing_one_falls_back_to_this_screen() {
        let numbered = number_monitors(two_monitors());
        assert_eq!(
            pick_monitor(&numbered, Some(1), Some((100, 100))),
            Some((0, false))
        );
        assert_eq!(pick_monitor(&numbered, Some(2), None), Some((1, false)));
        // No monitor 3: the monitor under the anchor, and the caller is told.
        assert_eq!(
            pick_monitor(&numbered, Some(3), Some((-500, 600))),
            Some((0, true))
        );
        assert_eq!(pick_monitor(&numbered, Some(0), None), Some((1, true)));
        assert_eq!(pick_monitor(&[], Some(1), None), None);
    }

    #[test]
    fn without_a_number_the_monitor_is_the_one_under_the_anchor_else_the_primary() {
        let numbered = number_monitors(two_monitors());
        assert_eq!(
            pick_monitor(&numbered, None, Some((-100, 500))),
            Some((0, false))
        );
        assert_eq!(
            pick_monitor(&numbered, None, Some((3000, 100))),
            Some((1, false))
        );
        // On the taskbar, outside the work area: the nearest monitor.
        assert_eq!(
            pick_monitor(&numbered, None, Some((3000, 2100))),
            Some((1, false))
        );
        assert_eq!(pick_monitor(&numbered, None, None), Some((1, false)));
    }

    // ---- the invisible border ----

    #[test]
    fn the_outer_rectangle_grows_by_the_invisible_border_on_each_edge() {
        // A window of Windows 11 at 100 percent: 7 px on the left, right and bottom.
        let outer = r(100, 100, 900, 700);
        let visible = r(107, 100, 893, 693);
        let left_half = r(0, 0, 960, 1040);
        assert_eq!(
            outer_for_visible(left_half, outer, visible),
            r(-7, 0, 967, 1047)
        );
        // At 200 percent the border is twice as wide; the same rule gives the right numbers.
        let outer = r(0, 0, 1000, 800);
        let visible = r(14, 0, 986, 786);
        let right_half = r(1920, 0, 3840, 2080);
        assert_eq!(
            outer_for_visible(right_half, outer, visible),
            r(1906, 0, 3854, 2094)
        );
    }

    #[test]
    fn two_neighbours_touch_after_the_correction() {
        let outer = r(0, 0, 800, 600);
        let visible = r(8, 0, 792, 592);
        let work = r(0, 0, 1920, 1040);
        let left = outer_for_visible(
            zone_rect(LayoutZone::LeftHalf, work).unwrap(),
            outer,
            visible,
        );
        let right = outer_for_visible(
            zone_rect(LayoutZone::RightHalf, work).unwrap(),
            outer,
            visible,
        );
        // The visible edges are the zone edges: outer edge plus the border gives 960 for both.
        assert_eq!(left.right - 8, 960);
        assert_eq!(right.left + 8, 960);
    }

    #[test]
    fn a_window_without_a_border_is_not_changed() {
        let same = r(10, 10, 500, 400);
        assert_eq!(
            outer_for_visible(r(0, 0, 100, 100), same, same),
            r(0, 0, 100, 100)
        );
    }

    // ---- which window ----

    fn window(hwnd: isize, pid: u32, image: &str, title: &str) -> SeenWindow {
        SeenWindow {
            hwnd,
            pid,
            image: image.to_owned(),
            title: title.to_owned(),
        }
    }

    fn query(images: &[&str], title: Option<&str>, existing: bool) -> WindowQuery {
        WindowQuery {
            images: images.iter().map(|i| (*i).to_owned()).collect(),
            title_has: title.map(str::to_owned),
            allow_existing: existing,
        }
    }

    #[test]
    fn a_new_window_of_the_started_process_is_found() {
        let before = [window(1, 10, "explorer.exe", "Downloads")];
        let now = [
            window(2, 500, "notepad.exe", "Untitled - Notepad"),
            window(1, 10, "explorer.exe", "Downloads"),
        ];
        let found = find_window(
            &before,
            &now,
            &HashSet::new(),
            &query(&[], None, false),
            Some(500),
        );
        assert_eq!(found, Some(2));
    }

    #[test]
    fn a_window_of_an_unrelated_program_is_never_taken() {
        let before = [];
        let now = [window(2, 77, "chrome.exe", "News")];
        let found = find_window(
            &before,
            &now,
            &HashSet::new(),
            &query(&[], None, false),
            Some(500),
        );
        assert_eq!(found, None);
        let found = find_window(
            &before,
            &now,
            &HashSet::new(),
            &query(&["notepad.exe"], None, false),
            None,
        );
        assert_eq!(found, None);
    }

    #[test]
    fn a_program_that_hands_off_to_a_running_process_is_found_by_its_image_name() {
        // wt.exe (pid 40) starts and ends; the window belongs to WindowsTerminal.exe (pid 9).
        let before = [window(5, 9, "windowsterminal.exe", "old tab")];
        let now = [
            window(6, 9, "windowsterminal.exe", "web-shop"),
            window(5, 9, "windowsterminal.exe", "old tab"),
        ];
        let q = query(&["windowsterminal.exe", "wt.exe"], None, false);
        assert_eq!(
            find_window(&before, &now, &HashSet::new(), &q, Some(40)),
            Some(6)
        );
        // The user's own terminal window is not new and is never taken.
        assert_eq!(find_window(&now, &now, &HashSet::new(), &q, Some(40)), None);
    }

    #[test]
    fn the_title_rule_picks_the_vs_code_window_of_the_folder() {
        let before = [window(1, 20, "code.exe", "other - Visual Studio Code")];
        let now = [
            window(3, 21, "code.exe", "Visual Studio Code"),
            window(2, 21, "code.exe", "web-shop - Visual Studio Code"),
            window(1, 20, "code.exe", "other - Visual Studio Code"),
        ];
        let q = query(&["code.exe"], Some("web-shop"), true);
        assert_eq!(
            find_window(&before, &now, &HashSet::new(), &q, Some(99)),
            Some(2)
        );
        // Before the title shows the folder, nothing matches yet.
        let early = [window(3, 21, "code.exe", "Visual Studio Code")];
        assert_eq!(
            find_window(&before, &early, &HashSet::new(), &q, Some(99)),
            None
        );
    }

    #[test]
    fn an_open_window_counts_only_with_allow_existing_and_a_title_rule() {
        let open = [window(2, 21, "code.exe", "Web-Shop - Visual Studio Code")];
        let with = query(&["code.exe"], Some("web-shop"), true);
        assert_eq!(
            find_window(&open, &open, &HashSet::new(), &with, Some(99)),
            Some(2)
        );
        let without = query(&["code.exe"], Some("web-shop"), false);
        assert_eq!(
            find_window(&open, &open, &HashSet::new(), &without, Some(99)),
            None
        );
        let no_title = query(&["code.exe"], None, true);
        assert_eq!(
            find_window(&open, &open, &HashSet::new(), &no_title, Some(99)),
            None
        );
        // A new window is preferred to an open one.
        let both = [
            window(7, 21, "code.exe", "web-shop - Visual Studio Code"),
            window(2, 21, "code.exe", "web-shop - Visual Studio Code"),
        ];
        assert_eq!(
            find_window(&open, &both, &HashSet::new(), &with, Some(99)),
            Some(7)
        );
    }

    #[test]
    fn the_title_rule_names_a_part_of_the_title_not_a_piece_of_text() {
        assert!(title_has("lib.rs - rain - Visual Studio Code", "rain"));
        assert!(title_has("rain (Workspace) - Visual Studio Code", "rain"));
        assert!(!title_has("rain-data - Visual Studio Code", "rain"));
        assert!(!title_has("Terrain - Visual Studio Code", "rain"));
        assert!(!title_has("anything", ""));
        // An open window of another folder is never taken for an open project.
        let open = [window(2, 21, "code.exe", "rain-data - Visual Studio Code")];
        let q = query(&["code.exe"], Some("rain"), true);
        assert_eq!(find_window(&open, &open, &HashSet::new(), &q, None), None);
    }

    #[test]
    fn two_items_of_the_same_program_get_two_different_windows() {
        let first = [window(11, 5, "notepad.exe", "a")];
        let q = query(&["notepad.exe"], None, false);
        let mut claimed = HashSet::new();
        assert_eq!(find_window(&[], &first, &claimed, &q, None), Some(11));
        claimed.insert(11);
        // The second item: the snapshot already holds window 11, and a second window is new.
        let now = [
            window(12, 5, "notepad.exe", "b"),
            window(11, 5, "notepad.exe", "a"),
        ];
        assert_eq!(find_window(&first, &now, &claimed, &q, None), Some(12));
        // Even if the snapshot missed window 11, a claimed window is not taken again.
        assert_eq!(find_window(&[], &first, &claimed, &q, None), None);
    }

    // ---- the run, with a fake desktop ----

    struct FakeDesktop {
        windows: Mutex<Vec<SeenWindow>>,
        /// Windows that show up after this many calls of `windows`.
        later: Mutex<Vec<(usize, SeenWindow)>>,
        calls: Mutex<usize>,
        monitors: Vec<MonitorInfo>,
        placed: Mutex<Vec<(isize, Rect)>>,
        refuse: bool,
        /// The first move is undone by the program: it goes back to this rectangle.
        drifts_to: Mutex<Option<Rect>>,
        frame: Mutex<Option<Rect>>,
    }

    impl FakeDesktop {
        fn new(windows: Vec<SeenWindow>) -> Arc<Self> {
            Arc::new(Self {
                windows: Mutex::new(windows),
                later: Mutex::new(Vec::new()),
                calls: Mutex::new(0),
                monitors: two_monitors(),
                placed: Mutex::new(Vec::new()),
                refuse: false,
                drifts_to: Mutex::new(None),
                frame: Mutex::new(None),
            })
        }

        fn placed(&self) -> Vec<(isize, Rect)> {
            self.placed.lock().unwrap().clone()
        }
    }

    impl Desktop for FakeDesktop {
        fn windows(&self) -> Vec<SeenWindow> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            let mut later = self.later.lock().unwrap();
            let mut windows = self.windows.lock().unwrap();
            let due: Vec<_> = later
                .iter()
                .filter(|(at, _)| *at <= *calls)
                .cloned()
                .collect();
            later.retain(|(at, _)| *at > *calls);
            for (_, window) in due {
                windows.insert(0, window);
            }
            windows.clone()
        }

        fn monitors(&self) -> Vec<MonitorInfo> {
            self.monitors.clone()
        }

        fn anchor(&self) -> Option<(i32, i32)> {
            Some((-100, 700)) // on the HD monitor, monitor 1
        }

        fn place(&self, hwnd: isize, visible: Rect) -> Result<(), String> {
            if self.refuse {
                return Err("access denied".to_owned());
            }
            let mut placed = self.placed.lock().unwrap();
            placed.push((hwnd, visible));
            let drift = self.drifts_to.lock().unwrap().take();
            *self.frame.lock().unwrap() = Some(drift.unwrap_or(visible));
            Ok(())
        }

        fn visible_frame(&self, _hwnd: isize) -> Option<Rect> {
            *self.frame.lock().unwrap()
        }
    }

    fn fast(desktop: &Arc<FakeDesktop>) -> Placer {
        Placer::with_timing(
            Arc::clone(desktop) as Arc<dyn Desktop>,
            Duration::from_millis(400),
            Duration::from_millis(5),
            Duration::from_millis(5),
        )
    }

    fn placement(zone: LayoutZone, monitor: Option<u32>, find: WindowQuery) -> Placement {
        Placement {
            zone,
            monitor,
            find,
        }
    }

    #[tokio::test]
    async fn a_window_that_shows_up_late_is_moved_into_its_zone_on_this_monitor() {
        let desktop = FakeDesktop::new(vec![window(1, 10, "explorer.exe", "Downloads")]);
        desktop
            .later
            .lock()
            .unwrap()
            .push((3, window(2, 500, "notepad.exe", "Untitled - Notepad")));
        let placer = fast(&desktop);
        let mut run = placer.begin().await;
        let before = run.snapshot().await;
        let note = run
            .place(
                &before,
                &placement(
                    LayoutZone::RightHalf,
                    None,
                    query(&["notepad.exe"], None, false),
                ),
                Some(500),
            )
            .await;
        assert_eq!(note, None);
        // This monitor is monitor 1 (HD, left): the right half of its work area.
        assert_eq!(desktop.placed(), vec![(2, r(-960, 200, 0, 1240))]);
    }

    #[tokio::test]
    async fn a_window_that_never_appears_is_reported_and_nothing_else_is_moved() {
        let desktop = FakeDesktop::new(vec![window(1, 10, "explorer.exe", "Downloads")]);
        let placer = fast(&desktop);
        let mut run = placer.begin().await;
        let before = run.snapshot().await;
        let note = run
            .place(
                &before,
                &placement(
                    LayoutZone::LeftHalf,
                    Some(2),
                    query(&["notepad.exe"], None, false),
                ),
                Some(500),
            )
            .await;
        assert_eq!(
            note.as_deref(),
            Some("Its window did not show up, so it was left where it is")
        );
        assert!(desktop.placed().is_empty());
    }

    #[tokio::test]
    async fn two_items_of_the_same_program_end_in_two_zones() {
        let desktop = FakeDesktop::new(Vec::new());
        let placer = fast(&desktop);
        let mut run = placer.begin().await;
        let q = query(&["notepad.exe"], None, false);

        let before = run.snapshot().await;
        desktop
            .windows
            .lock()
            .unwrap()
            .push(window(21, 5, "notepad.exe", "a"));
        let first = placement(LayoutZone::TopLeft, Some(2), q.clone());
        assert_eq!(run.place(&before, &first, None).await, None);

        let before = run.snapshot().await;
        desktop
            .windows
            .lock()
            .unwrap()
            .insert(0, window(22, 5, "notepad.exe", "b"));
        let second = placement(LayoutZone::BottomRight, Some(2), q);
        assert_eq!(run.place(&before, &second, None).await, None);

        let placed = desktop.placed();
        assert_eq!(placed[0], (21, r(0, 0, 1920, 1040)));
        assert_eq!(placed[1], (22, r(1920, 1040, 3840, 2080)));
    }

    #[tokio::test]
    async fn a_refused_move_is_reported() {
        let mut fake = FakeDesktop::new(vec![window(2, 500, "notepad.exe", "n")]);
        Arc::get_mut(&mut fake).unwrap().refuse = true;
        let placer = fast(&fake);
        let mut run = placer.begin().await;
        let note = run
            .place(
                &[],
                &placement(LayoutZone::Full, None, query(&[], None, false)),
                Some(500),
            )
            .await;
        assert_eq!(
            note.as_deref(),
            Some("Could not move its window: access denied")
        );
    }

    #[tokio::test]
    async fn a_monitor_that_is_not_connected_is_said_and_this_screen_is_used() {
        let desktop = FakeDesktop::new(vec![window(2, 500, "notepad.exe", "n")]);
        let placer = fast(&desktop);
        let mut run = placer.begin().await;
        let note = run
            .place(
                &[],
                &placement(LayoutZone::LeftHalf, Some(3), query(&[], None, false)),
                Some(500),
            )
            .await;
        assert_eq!(
            note.as_deref(),
            Some("Monitor 3 is not connected, so it went to this screen")
        );
        assert_eq!(desktop.placed()[0].1, r(-1920, 200, -960, 1240));
    }

    #[tokio::test]
    async fn a_window_that_moves_itself_back_is_placed_once_more() {
        let desktop = FakeDesktop::new(vec![window(2, 500, "notepad.exe", "n")]);
        *desktop.drifts_to.lock().unwrap() = Some(r(5, 5, 400, 300));
        let placer = fast(&desktop);
        let mut run = placer.begin().await;
        let note = run
            .place(
                &[],
                &placement(LayoutZone::Full, Some(2), query(&[], None, false)),
                Some(500),
            )
            .await;
        assert_eq!(note, None);
        assert_eq!(desktop.placed().len(), 2);
        assert_eq!(desktop.placed()[1].1, r(0, 0, 3840, 2080));
    }
}
