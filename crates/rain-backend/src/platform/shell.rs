//! The Windows shell: starting things, showing a file in Explorer, listing the installed apps.
//! Foundation section 11.6.
//!
//! [`shell_execute`] is the one launch call of the backend: shortcuts, URIs, folders, files with
//! parameters and `shell:AppsFolder\<AUMID>` all go through it (section 01 6.8.1, section 02,
//! section 04). The blocking functions in this file ([`reveal`], [`list_apps`]) must run on the COM
//! thread, so the caller wraps them:
//! `com::run(move || shell::reveal(&path))`.

use std::path::Path;

use windows::Win32::Storage::EnhancedStorage::{
    PKEY_AppUserModel_ID, PKEY_Link_Arguments, PKEY_Link_TargetParsingPath,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    AO_NONE, ApplicationActivationManager, BHID_EnumItems, FOLDERID_AppsFolder,
    IApplicationActivationManager, IEnumShellItems, IShellItem, IShellItem2, KF_FLAG_DEFAULT,
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, SHGetKnownFolderItem,
    SHOpenFolderAndSelectItems, SHParseDisplayName, SIGDN_NORMALDISPLAY, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{Interface, PCWSTR, PWSTR};

use super::com::{self, ComError};

/// `ShellExecuteExW` with `nShow = SW_SHOWNORMAL`. The mask always holds `SEE_MASK_FLAG_NO_UI`
/// (a missing file never shows an error box from a process without a window) and
/// `SEE_MASK_NOASYNC`; `flags` adds more `SEE_MASK_*` bits (pass 0 normally).
///
/// `verb` is usually `open`. `params` and `dir` are optional. `Ok` means that Windows accepted
/// the request, not that a window appeared. `Err` holds the failure code: a `SE_ERR_*` value of
/// 32 or less (2 file not found, 3 path not found, 5 access denied, 31 no program for the file
/// type), else the Win32 error code (740: needs administrator rights, 1223: cancelled).
///
/// The call runs on the COM thread. The caller puts its own time limit around it.
pub async fn shell_execute(
    verb: &str,
    file: &str,
    params: Option<&str>,
    dir: Option<&str>,
    flags: u32,
) -> Result<(), ShellExecuteError> {
    let request = (wide(verb), wide(file), params.map(wide), dir.map(wide));
    com::run(move || Ok(execute_now(&request, flags)))
        .await
        .map_err(ShellExecuteError::Com)?
}

/// Why `shell_execute` did not start anything.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShellExecuteError {
    /// The COM thread did not take or finish the call.
    #[error(transparent)]
    Com(ComError),
    /// Windows refused. See `shell_execute` for the codes.
    #[error("Windows could not start it (code {0})")]
    Code(u32),
}

impl ShellExecuteError {
    /// The code to match on: `None` for a COM thread problem.
    pub fn code(&self) -> Option<u32> {
        match self {
            Self::Code(code) => Some(*code),
            Self::Com(_) => None,
        }
    }
}

type Request = (Vec<u16>, Vec<u16>, Option<Vec<u16>>, Option<Vec<u16>>);

fn execute_now(request: &Request, flags: u32) -> Result<(), ShellExecuteError> {
    let (verb, file, params, dir) = request;
    let pointer =
        |text: &Option<Vec<u16>>| text.as_ref().map_or(PCWSTR::null(), |t| PCWSTR(t.as_ptr()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC | flags,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: pointer(params),
        lpDirectory: pointer(dir),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is fully set, and every string it points to is NUL-terminated and kept
    // alive by `request` for the whole call.
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => Ok(()),
        Err(error) => Err(ShellExecuteError::Code(failure_code(
            info.hInstApp.0 as usize,
            error.code().0,
        ))),
    }
}

/// `hInstApp` holds a `SE_ERR_*` value (32 or less) for the failures that have one; for the
/// others the Win32 error of the call is the answer (the low word of its HRESULT).
fn failure_code(h_inst_app: usize, hresult: i32) -> u32 {
    if (1..=32).contains(&h_inst_app) {
        h_inst_app as u32
    } else {
        (hresult as u32) & 0xFFFF
    }
}

/// Starts a packaged app by its AUMID with `IApplicationActivationManager` (160 to 260 ms) and
/// returns the process id. `args` is the argument text the app receives (`None` for none). Runs
/// on the COM thread. An app that is already running is activated and its id returned.
pub fn activate_application(aumid: &str, args: Option<&str>) -> Result<u32, ComError> {
    let aumid = wide(aumid);
    let args = args.map(wide);
    // SAFETY: the manager is created and used on this COM thread only; both strings are
    // NUL-terminated and outlive the call (an absent argument text is a null pointer).
    unsafe {
        let manager: IApplicationActivationManager =
            CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_INPROC_SERVER)?;
        let arguments = args.as_ref().map_or(PCWSTR::null(), |a| PCWSTR(a.as_ptr()));
        Ok(manager.ActivateApplication(PCWSTR(aumid.as_ptr()), arguments, AO_NONE)?)
    }
}

/// Opens Explorer on the parent folder of `path` with the item selected. Runs on the COM thread.
/// `path` is an absolute path that exists (section 05 6.9 checks that before).
pub fn reveal(path: &Path) -> Result<(), ComError> {
    let wide_path = wide(&path.to_string_lossy());
    let mut item: *mut ITEMIDLIST = std::ptr::null_mut();
    // SAFETY: `wide_path` is NUL-terminated; `item` is a valid place for the result.
    unsafe { SHParseDisplayName(PCWSTR(wide_path.as_ptr()), None, &mut item, 0, None)? };
    // With no children (cidl = 0), the first argument is the item to select; Explorer opens its
    // parent folder.
    // SAFETY: `item` was just returned by `SHParseDisplayName`.
    let shown = unsafe { SHOpenFolderAndSelectItems(item, None, 0) };
    // SAFETY: the item list was allocated by the shell for the caller to free, once.
    unsafe { CoTaskMemFree(Some(item as *const _)) };
    shown.map_err(ComError::from)
}

// ---- the installed apps ----

/// One item of `shell:AppsFolder`. A property that the item does not have is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawApp {
    pub aumid: String,
    pub name: String,
    pub target: String,
    pub args: String,
}

/// Lists `shell:AppsFolder`: every installed app, packaged or not, with its name, its app id
/// (AUMID), the target of its shortcut and the arguments. 190 to 200 ms for 125 items (spike
/// `winapi`). Runs on the COM thread.
pub fn list_apps() -> Result<Vec<RawApp>, ComError> {
    // SAFETY (the whole function): every interface comes from the shell on this COM thread and
    // is used only here; every string that the shell returns is copied and freed by `text_of`.
    let folder: IShellItem =
        unsafe { SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)? };
    let items: IEnumShellItems = unsafe { folder.BindToHandler(None, &BHID_EnumItems)? };
    let mut apps = Vec::new();
    loop {
        let mut next: [Option<IShellItem>; 1] = [None];
        let mut fetched = 0u32;
        unsafe { items.Next(&mut next, Some(&mut fetched))? };
        let Some(item) = next[0].take().filter(|_| fetched > 0) else {
            break;
        };
        apps.push(read_app(&item)?);
    }
    Ok(apps)
}

fn read_app(item: &IShellItem) -> Result<RawApp, ComError> {
    // SAFETY: see `list_apps`.
    unsafe {
        let name = text_of(item.GetDisplayName(SIGDN_NORMALDISPLAY)?);
        let properties: IShellItem2 = item.cast()?;
        let property = |key| properties.GetString(key).map(text_of).unwrap_or_default();
        Ok(RawApp {
            name,
            aumid: property(&PKEY_AppUserModel_ID),
            target: property(&PKEY_Link_TargetParsingPath),
            args: property(&PKEY_Link_Arguments),
        })
    }
}

/// Copies a string that the shell allocated and frees it.
fn text_of(text: PWSTR) -> String {
    // SAFETY: `text` is a valid string from the shell. It is freed exactly once, after the copy.
    unsafe {
        let copy = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0 as *const _));
        copy
    }
}

/// UTF-16 with a closing NUL, for the Windows calls.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn a_file_that_does_not_exist_fails_with_file_not_found_and_shows_no_dialog() {
        let missing = r"C:\no-such-folder-rain-test\no-such-file.exe";
        let error = shell_execute("open", missing, None, None, 0)
            .await
            .unwrap_err();
        // 2 is SE_ERR_FNF and 3 is SE_ERR_PNF: either says "it is not there".
        assert!(matches!(error.code(), Some(2 | 3)), "{error:?}");
    }

    // No test opens a file whose type has no program: Windows then shows the "How do you want to
    // open this file" dialog on the desktop and still answers success, `SEE_MASK_FLAG_NO_UI`
    // notwithstanding (seen once, and the dialog had to be closed by hand). The callers of
    // `shell_execute` must not rely on code 31 for that case.

    #[test]
    fn the_failure_code_is_the_se_err_value_when_there_is_one() {
        assert_eq!(failure_code(2, 0x8007_0002_u32 as i32), 2);
        assert_eq!(failure_code(31, -1), 31);
        // hInstApp 0 or above 32: the Win32 error of the call (the low word of the HRESULT).
        assert_eq!(failure_code(0, 0x8007_02E4_u32 as i32), 0x02E4);
        assert_eq!(failure_code(42, 0x8007_04C7_u32 as i32), 0x04C7);
    }

    #[tokio::test]
    async fn the_apps_folder_lists_apps_with_names_and_ids() {
        let apps = com::run(list_apps).await.unwrap();
        assert!(!apps.is_empty(), "Windows always lists some apps");
        assert!(
            apps.iter()
                .all(|app| !app.name.is_empty() || !app.aumid.is_empty())
        );
        assert!(apps.iter().any(|app| !app.aumid.is_empty()));
    }

    #[tokio::test]
    async fn revealing_a_path_that_does_not_exist_is_an_error() {
        // Only the failing case: the successful one would open an Explorer window on the
        // desktop of whoever runs the tests. The success was proven in the spike `process`.
        let missing = PathBuf::from(r"C:\no-such-folder-rain-test\file.txt");
        assert!(com::run(move || reveal(&missing)).await.is_err());
    }
}
