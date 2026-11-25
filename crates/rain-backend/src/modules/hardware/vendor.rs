//! What Rain knows about makers of input devices, and where their programs are.
//!
//! This is the only place with names of vendors. A new vendor is one more row in `VENDORS`.
//! Nothing here names a device of the owner: a device is found by its class and its vendor id.

use std::path::{Path, PathBuf};

use crate::platform::registry::UninstallEntry;

/// A program that manages a device: found in the list of installed programs.
pub struct AppHint {
    /// What the action is called.
    pub label: &'static str,
    /// The names that the program has in the list of installed programs (lower case). A name
    /// matches when the list entry is equal to it or starts with it and a space.
    pub names: &'static [&'static str],
    /// The file name of the program. The entry only counts when this file exists.
    pub exe: &'static str,
}

pub struct Vendor {
    pub name: &'static str,
    pub vendor_id: u16,
    /// The devices of this vendor answer questions over HID++ (Logitech). Their plain keyboard
    /// and mouse interfaces are then not listed on their own: the answer says what the device is.
    pub hidpp: bool,
    /// The programs of the vendor, the one to prefer first.
    pub apps: &'static [AppHint],
    /// Where to get a program when none is installed.
    pub download: &'static str,
}

pub const VENDORS: &[Vendor] = &[Vendor {
    name: "Logitech",
    vendor_id: 0x046D,
    hidpp: true,
    apps: &[
        AppHint {
            label: "Logitech G HUB",
            names: &["logitech g hub"],
            exe: "lghub.exe",
        },
        AppHint {
            label: "Logi Options+",
            names: &["logi options+"],
            exe: "logioptionsplus.exe",
        },
    ],
    download: "https://www.logitechg.com/en-us/innovation/g-hub.html",
}];

/// A keyboard that has the raw HID interface of QMK and VIA (usage page 0xFF60, usage 0x61) can
/// be set up with VIA.
pub const VIA_USAGE_PAGE: u16 = 0xFF60;
pub const VIA_USAGE: u16 = 0x61;

pub const VIA_APP: AppHint = AppHint {
    label: "VIA",
    names: &["via"],
    exe: "via.exe",
};
pub const VIA_WEB: &str = "https://usevia.app";

pub fn vendor_of(vendor_id: u16) -> Option<&'static Vendor> {
    VENDORS.iter().find(|vendor| vendor.vendor_id == vendor_id)
}

impl AppHint {
    fn matches_name(&self, display_name: &str) -> bool {
        let name = display_name.trim().to_lowercase();
        self.names.iter().any(|known| {
            name == *known
                || name
                    .strip_prefix(known)
                    .is_some_and(|rest| rest.starts_with(' '))
        })
    }

    /// The program file of this app, when it is installed: from the icon path of the list entry,
    /// else from the install folder. `exists` says whether a file is there (a test replaces it).
    pub fn find(
        &self,
        installed: &[UninstallEntry],
        exists: &dyn Fn(&Path) -> bool,
    ) -> Option<PathBuf> {
        installed
            .iter()
            .filter(|entry| self.matches_name(&entry.display_name))
            .flat_map(|entry| {
                let from_icon = exe_of_icon(&entry.display_icon);
                let from_folder = (!entry.install_location.trim().is_empty())
                    .then(|| Path::new(entry.install_location.trim()).join(self.exe));
                from_icon.into_iter().chain(from_folder)
            })
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(self.exe))
                    && exists(path)
            })
    }
}

/// The program path in a `DisplayIcon` value: `"C:\x\a.exe",0` becomes `C:\x\a.exe`. `None` when
/// the value is not a path to a program.
fn exe_of_icon(display_icon: &str) -> Option<PathBuf> {
    let text = display_icon.trim();
    let text = text
        .strip_prefix('"')
        .map_or(text, |rest| rest.split('"').next().unwrap_or(rest));
    // `C:\x\a.exe,0`: the number after the last comma picks an icon in the file.
    let text = match text.rsplit_once(',') {
        Some((path, number)) if number.trim().trim_start_matches('-').parse::<u32>().is_ok() => {
            path
        }
        _ => text,
    };
    let text = text.trim();
    text.to_lowercase()
        .ends_with(".exe")
        .then(|| PathBuf::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, icon: &str, folder: &str) -> UninstallEntry {
        UninstallEntry {
            display_name: name.to_owned(),
            display_icon: icon.to_owned(),
            install_location: folder.to_owned(),
        }
    }

    fn everything_exists(_: &Path) -> bool {
        true
    }

    fn nothing_exists(_: &Path) -> bool {
        false
    }

    const G_HUB: &AppHint = &VENDORS[0].apps[0];

    #[test]
    fn logitech_is_in_the_table_by_its_vendor_id() {
        let vendor = vendor_of(0x046D).unwrap();
        assert_eq!(vendor.name, "Logitech");
        assert!(vendor.hidpp);
        assert!(vendor_of(0x1234).is_none());
    }

    #[test]
    fn the_program_is_found_through_the_icon_of_the_list_entry() {
        let installed = [entry(
            "Logitech G HUB",
            r"C:\Program Files\LGHUB\lghub.exe",
            "",
        )];
        assert_eq!(
            G_HUB.find(&installed, &everything_exists),
            Some(PathBuf::from(r"C:\Program Files\LGHUB\lghub.exe"))
        );
    }

    #[test]
    fn the_program_is_found_through_the_install_folder_when_the_icon_is_not_a_program() {
        let installed = [entry("Logitech G HUB", "", r"D:\Apps\LGHUB")];
        assert_eq!(
            G_HUB.find(&installed, &everything_exists),
            Some(PathBuf::from(r"D:\Apps\LGHUB").join("lghub.exe"))
        );
    }

    #[test]
    fn an_entry_whose_file_is_gone_does_not_count() {
        let installed = [entry("Logitech G HUB", r"C:\x\lghub.exe", "")];
        assert_eq!(G_HUB.find(&installed, &nothing_exists), None);
    }

    #[test]
    fn a_quoted_icon_with_an_index_is_read() {
        assert_eq!(
            exe_of_icon(r#""C:\Program Files\LGHUB\lghub.exe",0"#),
            Some(PathBuf::from(r"C:\Program Files\LGHUB\lghub.exe"))
        );
        assert_eq!(
            exe_of_icon(r"C:\a\b.exe,-12"),
            Some(PathBuf::from(r"C:\a\b.exe"))
        );
        assert_eq!(exe_of_icon(r"C:\a\b.ico"), None);
        assert_eq!(exe_of_icon(""), None);
    }

    #[test]
    fn names_match_whole_words_only_and_the_file_name_must_fit() {
        assert!(VIA_APP.matches_name("VIA"));
        assert!(VIA_APP.matches_name("via 3.0.1"));
        assert!(!VIA_APP.matches_name("Viber"));
        // A chipset driver from a company with the same letters is not the keyboard program.
        let installed = [entry("VIA Platform Device Manager", r"C:\x\setup.exe", "")];
        assert_eq!(VIA_APP.find(&installed, &everything_exists), None);
        let real = [entry(
            "VIA",
            r"C:\Users\me\AppData\Local\Programs\via\VIA.exe",
            "",
        )];
        assert!(VIA_APP.find(&real, &everything_exists).is_some());
    }
}
