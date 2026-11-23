//! The Windows registry: reading values, and the "start with Windows" entry.
//! Foundation 2.9 and section 06 6.4; the `winreg` calls are the ones proven in the spike
//! `winapi` (`registry.rs`).

use std::io;

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, REG_BINARY};
use winreg::types::FromRegValue;

/// The two hives that Rain reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

impl Hive {
    fn root(self) -> RegKey {
        match self {
            Self::CurrentUser => RegKey::predef(HKEY_CURRENT_USER),
            Self::LocalMachine => RegKey::predef(HKEY_LOCAL_MACHINE),
        }
    }
}

/// A text value, or `None` when the key or the value does not exist. Any other failure (access
/// denied, a value of another type) is an error.
pub fn read_string(hive: Hive, key: &str, name: &str) -> io::Result<Option<String>> {
    read_value(hive, key, name)
}

/// A number value (`REG_DWORD`), or `None` when the key or the value does not exist.
pub fn read_u32(hive: Hive, key: &str, name: &str) -> io::Result<Option<u32>> {
    read_value(hive, key, name)
}

fn read_value<T: FromRegValue>(hive: Hive, key: &str, name: &str) -> io::Result<Option<T>> {
    let key = match hive.root().open_subkey_with_flags(key, KEY_READ) {
        Ok(key) => key,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    match key.get_value(name) {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

// ---- start with Windows ----

/// `HKCU\...\Run`: a value here is started at logon.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// `HKCU\...\StartupApproved\Run`: where Task Manager and Settings keep "disabled by the user".
const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
/// The name of Rain's value in both keys.
pub const RUN_VALUE_NAME: &str = "Rain";

/// What the registry says about Rain's autostart entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunState {
    /// The command in the Run value, or `None` when there is no value.
    pub command: Option<String>,
    /// Windows (Task Manager, Settings) has marked the entry as disabled.
    pub disabled_by_windows: bool,
}

impl RunState {
    /// The switch the user sees: an entry exists and Windows has not disabled it.
    pub fn is_on(&self) -> bool {
        self.command.is_some() && !self.disabled_by_windows
    }
}

/// The autostart entry, behind a trait so that no test touches the real one.
pub trait RunKey: Send + Sync {
    /// A missing value is not an error: it is a state with no command.
    fn read(&self) -> io::Result<RunState>;
    /// Writes the value and removes a "disabled" mark of Windows, so that the entry works.
    fn enable(&self, command: &str) -> io::Result<()>;
    /// Removes the value. A value that is not there is a success.
    fn disable(&self) -> io::Result<()>;
}

/// The real entry of the current user: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Rain`.
/// The settings module refuses to use it in a sandbox run (foundation 2.7).
pub struct WindowsRunKey {
    run_key: String,
    approved_key: String,
    name: String,
}

impl WindowsRunKey {
    /// The entry that Windows reads at logon.
    pub fn real() -> Self {
        Self::at(RUN_KEY, APPROVED_KEY, RUN_VALUE_NAME)
    }

    /// The same code on other keys. The tests use a throwaway key of their own, so they never
    /// touch the real entry or anything that Windows starts at logon.
    pub fn at(run_key: &str, approved_key: &str, name: &str) -> Self {
        Self {
            run_key: run_key.to_owned(),
            approved_key: approved_key.to_owned(),
            name: name.to_owned(),
        }
    }
}

impl RunKey for WindowsRunKey {
    fn read(&self) -> io::Result<RunState> {
        let command = read_string(Hive::CurrentUser, &self.run_key, &self.name)?;
        // The marker is a courtesy of Windows. Without it the entry counts as enabled.
        let approved = read_binary(&self.approved_key, &self.name).unwrap_or_default();
        Ok(RunState {
            command,
            disabled_by_windows: approved.is_some_and(|bytes| is_disabled_marker(&bytes)),
        })
    }

    fn enable(&self, command: &str) -> io::Result<()> {
        let (run, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(&self.run_key)?;
        run.set_value(&self.name, &command)?;
        // A mark "disabled" would keep the entry from starting although the value is there.
        let approved = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(&self.approved_key, KEY_SET_VALUE);
        match approved {
            Ok(key) => ignore_missing(key.delete_value(&self.name)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn disable(&self) -> io::Result<()> {
        let run =
            RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(&self.run_key, KEY_SET_VALUE);
        match run {
            Ok(key) => ignore_missing(key.delete_value(&self.name)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

fn ignore_missing(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn read_binary(key: &str, name: &str) -> io::Result<Option<Vec<u8>>> {
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(key, KEY_READ) {
        Ok(key) => key,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    match key.get_raw_value(name) {
        Ok(value) if value.vtype == REG_BINARY => Ok(Some(value.bytes.into_owned())),
        // Any other type is not a marker that Windows wrote.
        Ok(_) => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The rule for the value of `StartupApproved\Run`: the first byte is odd when the entry is
/// disabled (`0x03`), and even when it is enabled (`0x02`, `0x06`). Any other shape (a value that
/// is empty) means enabled. This comes from how Windows behaves and was not proven in a spike
/// (foundation 6.4); a unit test states the rule and acceptance check S6 checks it by hand.
pub fn is_disabled_marker(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|first| first % 2 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_byte_of_the_marker_decides() {
        assert!(is_disabled_marker(&[0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!is_disabled_marker(&[0x02, 0, 0, 0]));
        assert!(!is_disabled_marker(&[0x06, 0, 0, 0]));
        assert!(!is_disabled_marker(&[]), "an empty value counts as enabled");
    }

    #[test]
    fn a_missing_key_or_value_reads_as_none() {
        assert_eq!(
            read_string(Hive::CurrentUser, r"Software\RainTestNoSuchKey", "x").unwrap(),
            None
        );
        let existing = r"Software\Microsoft\Windows\CurrentVersion\Explorer";
        assert_eq!(
            read_string(Hive::CurrentUser, existing, "RainTestNoSuchValue").unwrap(),
            None
        );
        assert_eq!(
            read_u32(Hive::LocalMachine, r"SOFTWARE\RainTestNoSuchKey", "x").unwrap(),
            None
        );
    }

    #[test]
    fn a_text_value_that_windows_has_is_read() {
        let version = read_string(
            Hive::LocalMachine,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "ProductName",
        )
        .unwrap();
        assert!(version.is_some_and(|name| name.contains("Windows")));
    }

    /// A throwaway place in HKCU that Windows never starts anything from.
    struct Scratch {
        root: String,
    }

    impl Scratch {
        fn new() -> Self {
            // One key for each test: the tests run in parallel and each one deletes its key.
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let number = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let root = format!(r"Software\RainTest-{}-{number}", std::process::id());
            Self { root }
        }

        fn entry(&self) -> WindowsRunKey {
            WindowsRunKey::at(
                &format!(r"{}\Run", self.root),
                &format!(r"{}\StartupApproved", self.root),
                "RainTestValue",
            )
        }

        fn mark(&self, bytes: &[u8]) {
            let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(format!(r"{}\StartupApproved", self.root))
                .unwrap();
            key.set_raw_value(
                "RainTestValue",
                &winreg::RegValue {
                    bytes: bytes.to_vec().into(),
                    vtype: REG_BINARY,
                },
            )
            .unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.root);
        }
    }

    #[test]
    fn enable_writes_the_command_and_disable_removes_it() {
        let scratch = Scratch::new();
        let entry = scratch.entry();
        assert_eq!(entry.read().unwrap(), RunState::default());

        entry.enable(r#""C:\Fake\rain.exe" --hidden"#).unwrap();
        let state = entry.read().unwrap();
        assert_eq!(
            state.command.as_deref(),
            Some(r#""C:\Fake\rain.exe" --hidden"#)
        );
        assert!(state.is_on());

        entry.disable().unwrap();
        assert_eq!(entry.read().unwrap(), RunState::default());
        entry.disable().unwrap(); // removing what is not there is fine
    }

    #[test]
    fn an_entry_that_windows_disabled_reads_as_off_and_enable_clears_the_mark() {
        let scratch = Scratch::new();
        let entry = scratch.entry();
        entry.enable("cmd").unwrap();
        scratch.mark(&[0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

        let state = entry.read().unwrap();
        assert!(state.disabled_by_windows);
        assert!(
            !state.is_on(),
            "the switch shows off while Windows has it disabled"
        );

        entry.enable("cmd").unwrap();
        let state = entry.read().unwrap();
        assert!(
            !state.disabled_by_windows,
            "switching on removes the disabled mark"
        );
        assert!(state.is_on());
    }

    #[test]
    fn an_enabled_marker_does_not_switch_the_entry_off() {
        let scratch = Scratch::new();
        let entry = scratch.entry();
        entry.enable("cmd").unwrap();
        scratch.mark(&[0x02, 0, 0, 0]);
        assert!(entry.read().unwrap().is_on());
    }

    #[test]
    fn the_real_entry_is_named_rain_in_the_run_key() {
        // Only the names are checked here. No test reads or writes the real entry.
        let real = WindowsRunKey::real();
        assert_eq!(real.name, "Rain");
        assert!(real.run_key.ends_with(r"CurrentVersion\Run"));
        assert!(real.approved_key.ends_with(r"StartupApproved\Run"));
    }
}
