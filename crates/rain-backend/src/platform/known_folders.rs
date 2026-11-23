//! Where Windows keeps the user's folders. Section 06 6.1.2 and section 01 6.1.
//!
//! The paths always come from Windows (`SHGetKnownFolderPath`), never from a user name or a
//! literal. The call needs no COM apartment.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_ProgramData,
    FOLDERID_Programs, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::GUID;

/// The folders that Rain asks Windows for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownFolder {
    /// `%APPDATA%`: `obsidian.json`, Rain's own settings.
    RoamingAppData,
    /// `%LOCALAPPDATA%`.
    LocalAppData,
    /// `%ProgramData%`: the Epic Games Launcher data.
    ProgramData,
    /// The user's profile folder.
    Profile,
    /// `...\Start Menu\Programs` of the user.
    Programs,
    /// `...\Start Menu\Programs` of all users.
    CommonPrograms,
}

impl KnownFolder {
    fn id(self) -> &'static GUID {
        match self {
            Self::RoamingAppData => &FOLDERID_RoamingAppData,
            Self::LocalAppData => &FOLDERID_LocalAppData,
            Self::ProgramData => &FOLDERID_ProgramData,
            Self::Profile => &FOLDERID_Profile,
            Self::Programs => &FOLDERID_Programs,
            Self::CommonPrograms => &FOLDERID_CommonPrograms,
        }
    }
}

/// The path of a known folder, as Windows says it (the folder may not exist).
pub fn known_folder(folder: KnownFolder) -> io::Result<PathBuf> {
    // SAFETY: the id is a valid constant, no token is used, and on success Windows returns a
    // NUL-terminated string that is copied below and then freed with `CoTaskMemFree`.
    let text = unsafe { SHGetKnownFolderPath(folder.id(), KF_FLAG_DEFAULT, None) }
        .map_err(|e| io::Error::other(format!("SHGetKnownFolderPath failed: {e}")))?;
    // SAFETY: `text` is the valid string just returned; the slice is read before the free.
    let path = PathBuf::from(OsString::from_wide(unsafe { text.as_wide() }));
    // SAFETY: the string was allocated by the shell for the caller to free, and is not used again.
    unsafe { CoTaskMemFree(Some(text.0 as *const _)) };
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_agrees_with_the_environment_about_the_roaming_and_local_folders() {
        let roaming = known_folder(KnownFolder::RoamingAppData).unwrap();
        let local = known_folder(KnownFolder::LocalAppData).unwrap();
        assert_eq!(
            roaming.to_string_lossy().to_lowercase(),
            std::env::var("APPDATA").unwrap().to_lowercase()
        );
        assert_eq!(
            local.to_string_lossy().to_lowercase(),
            std::env::var("LOCALAPPDATA").unwrap().to_lowercase()
        );
    }

    #[test]
    fn every_known_folder_has_an_absolute_path() {
        for folder in [
            KnownFolder::RoamingAppData,
            KnownFolder::LocalAppData,
            KnownFolder::ProgramData,
            KnownFolder::Profile,
            KnownFolder::Programs,
            KnownFolder::CommonPrograms,
        ] {
            assert!(known_folder(folder).unwrap().is_absolute(), "{folder:?}");
        }
    }

    #[test]
    fn the_start_menu_folders_are_inside_the_roaming_and_program_data_folders() {
        let programs = known_folder(KnownFolder::Programs).unwrap();
        assert!(programs.starts_with(known_folder(KnownFolder::RoamingAppData).unwrap()));
        let common = known_folder(KnownFolder::CommonPrograms).unwrap();
        assert!(common.starts_with(known_folder(KnownFolder::ProgramData).unwrap()));
    }
}
