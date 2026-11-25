//! The programs that game mode never closes, by name. Section 06 6.11.
//!
//! One table, one comment per group. A name is the file name of the program without `.exe`,
//! lower case. A pattern that ends in `*` matches every name that starts with the text before it.
//! Add a program here, never in the code that uses the table.

/// Windows' own shell and helper programs. Windows itself (anything under `%WINDIR%`) is kept by
/// folder; these are the ones that live elsewhere or that are worth naming (a Store app, the
/// search host). Closing any of them breaks the desktop.
pub const SHELL: &[&str] = &[
    "explorer",
    "dwm",
    "sihost",
    "ctfmon",
    "taskhostw",
    "runtimebroker",
    "shellexperiencehost",
    "startmenuexperiencehost",
    "searchhost",
    "searchindexer",
    "searchprotocolhost",
    "searchfilterhost",
    "textinputhost",
    "applicationframehost",
    "systemsettings",
    "systemsettingsbroker",
    "conhost",
    "openconsole",
    "fontdrvhost",
    "audiodg",
    "securityhealthsystray",
    "securityhealthservice",
    "smartscreen",
    "widgets",
    "widgetservice",
    "lockapp",
    "backgroundtaskhost",
    "dllhost",
    "svchost",
    "winlogon",
    "csrss",
    "wininit",
    "services",
    "lsass",
    "smss",
    "dashost",
    "unsecapp",
    "wmiprvse",
    "credentialuibroker",
    "pickerhost",
    "useroobebroker",
    "phoneexperiencehost",
    "crossdeviceservice",
    "crossdeviceresume",
];

/// A group of programs that are kept for one reason.
pub struct Group {
    /// What the group is for. Also what the tests print.
    pub name: &'static str,
    pub patterns: &'static [&'static str],
}

/// The built-in keep list. The user can keep more in `gamemode.toml`.
pub const GROUPS: &[Group] = &[
    // Anti-cheat: ending one while a game is installed can ban the account or break the game.
    Group {
        name: "anti-cheat",
        patterns: &[
            "vgc",
            "vgtray",
            "easyanticheat*",
            "beservice*",
            "bedaisy*",
            "faceit*",
            "esea*",
        ],
    },
    // Game launchers and their helpers: the game starts and updates through them.
    Group {
        name: "game launchers",
        patterns: &[
            "steam*",
            "gameoverlayui",
            "epicgameslauncher",
            "epicwebhelper",
            "riotclientservices",
            "riotclientux*",
            "riotclient*",
            "battle.net",
            "eadesktop",
            "eabackgroundservice",
            "upc",
            "ubisoftconnect",
            "galaxyclient*",
            "xboxpcapp",
            "xbox*",
            "gamingservices*",
        ],
    },
    // Graphics and audio driver tools: they apply the settings that games rely on.
    Group {
        name: "graphics and audio drivers",
        patterns: &[
            "radeonsoftware",
            "amdrsserv",
            "amdow",
            "cncmd",
            "amd*",
            "nvcontainer",
            "nvidia*",
            "nvoverlay*",
            "nvsphelper64",
            "nvdisplay*",
            "igfx*",
            "rtk*",
            "nahimicsvc*",
            "dolby*",
        ],
    },
    // Keyboard and mouse tools: closing them resets the lighting, macros and DPI of the device.
    Group {
        name: "keyboard and mouse tools",
        patterns: &[
            "lghub*",
            "logi*",
            "razer*",
            "icue",
            "corsair*",
            "steelseries*",
            "wootility",
        ],
    },
    // Overlays and capture that the player may want while playing.
    Group {
        name: "overlays and capture",
        patterns: &["obs64", "obs32", "msiafterburner", "rtss*", "gamebar*"],
    },
    // Password and security tools: antivirus trays and the password managers.
    Group {
        name: "password and security",
        patterns: &[
            "1password",
            "bitwarden",
            "keepass*",
            "avgui",
            "avastui",
            "mbam*",
            "nortonui",
            "norton*",
            "mcuicnt",
            "ekrn",
            "egui",
            "bdagent",
            "kaspersky*",
            "avp",
            "defender*",
            "securityhealth*",
            "msmpeng",
            "mpdefendercoreservice",
        ],
    },
];

/// The name of a program without `.exe`, lower case.
pub fn stem(file_name: &str) -> String {
    let lower = file_name.trim().to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
}

fn matches(pattern: &str, stem: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => stem.starts_with(prefix),
        None => stem == pattern,
    }
}

/// Whether a program of the shell is meant (by name, no `.exe`).
pub fn is_shell(file_name: &str) -> bool {
    let name = stem(file_name);
    SHELL.contains(&name.as_str())
}

/// The group of the built-in keep list that holds this program.
pub fn group_of(file_name: &str) -> Option<&'static str> {
    let name = stem(file_name);
    GROUPS
        .iter()
        .find(|group| group.patterns.iter().any(|pattern| matches(pattern, &name)))
        .map(|group| group.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_group_keeps_its_known_programs_whatever_the_case() {
        let table = [
            ("vgc.exe", "anti-cheat"),
            ("VGTRAY.exe", "anti-cheat"),
            ("EasyAntiCheat_EOS.exe", "anti-cheat"),
            ("BEService.exe", "anti-cheat"),
            ("BEDaisy.exe", "anti-cheat"),
            ("FACEITClient.exe", "anti-cheat"),
            ("ESEAClient.exe", "anti-cheat"),
            ("steam.exe", "game launchers"),
            ("steamwebhelper.exe", "game launchers"),
            ("EpicGamesLauncher.exe", "game launchers"),
            ("EpicWebHelper.exe", "game launchers"),
            ("RiotClientServices.exe", "game launchers"),
            ("RiotClientUx.exe", "game launchers"),
            ("RiotClientUxRender.exe", "game launchers"),
            ("Battle.net.exe", "game launchers"),
            ("EADesktop.exe", "game launchers"),
            ("EABackgroundService.exe", "game launchers"),
            ("upc.exe", "game launchers"),
            ("UbisoftConnect.exe", "game launchers"),
            ("GalaxyClient.exe", "game launchers"),
            ("XboxPcApp.exe", "game launchers"),
            ("GamingServices.exe", "game launchers"),
            ("GamingServicesNet.exe", "game launchers"),
            ("RadeonSoftware.exe", "graphics and audio drivers"),
            ("AMDRSServ.exe", "graphics and audio drivers"),
            ("amdow.exe", "graphics and audio drivers"),
            ("cncmd.exe", "graphics and audio drivers"),
            ("nvcontainer.exe", "graphics and audio drivers"),
            ("NVIDIA app.exe", "graphics and audio drivers"),
            ("NVIDIA Overlay.exe", "graphics and audio drivers"),
            ("nvsphelper64.exe", "graphics and audio drivers"),
            ("igfxEM.exe", "graphics and audio drivers"),
            ("RtkAudUService64.exe", "graphics and audio drivers"),
            ("RtkUWP.exe", "graphics and audio drivers"),
            ("DefenderSessionHelper.exe", "password and security"),
            ("NahimicSvc64.exe", "graphics and audio drivers"),
            ("DolbyDAX2API.exe", "graphics and audio drivers"),
            ("lghub.exe", "keyboard and mouse tools"),
            ("lghub_agent.exe", "keyboard and mouse tools"),
            ("logioptionsplus_agent.exe", "keyboard and mouse tools"),
            ("RazerCentral.exe", "keyboard and mouse tools"),
            ("iCUE.exe", "keyboard and mouse tools"),
            ("CorsairService.exe", "keyboard and mouse tools"),
            ("SteelSeriesGG.exe", "keyboard and mouse tools"),
            ("Wootility.exe", "keyboard and mouse tools"),
            ("obs64.exe", "overlays and capture"),
            ("MSIAfterburner.exe", "overlays and capture"),
            ("RTSS.exe", "overlays and capture"),
            ("RTSSHooksLoader64.exe", "overlays and capture"),
            ("1Password.exe", "password and security"),
            ("Bitwarden.exe", "password and security"),
            ("avgui.exe", "password and security"),
            ("NortonUI.exe", "password and security"),
        ];
        for (file, group) in table {
            assert_eq!(group_of(file), Some(group), "{file}");
        }
    }

    #[test]
    fn ordinary_programs_are_not_in_the_list() {
        for file in [
            "Discord.exe",
            "chrome.exe",
            "Spotify.exe",
            "Slack.exe",
            "notepad++.exe",
            "Teams.exe",
            "OneDrive.exe",
        ] {
            assert_eq!(group_of(file), None, "{file}");
        }
    }

    #[test]
    fn the_shell_is_named_by_stem_and_case() {
        for file in [
            "explorer.exe",
            "DWM.EXE",
            "sihost.exe",
            "ctfmon.exe",
            "taskhostw.exe",
            "RuntimeBroker.exe",
            "ShellExperienceHost.exe",
            "StartMenuExperienceHost.exe",
            "SearchHost.exe",
            "TextInputHost.exe",
            "ApplicationFrameHost.exe",
            "SystemSettings.exe",
            "conhost.exe",
            "fontdrvhost.exe",
            "audiodg.exe",
            "SecurityHealthSystray.exe",
        ] {
            assert!(is_shell(file), "{file}");
        }
        assert!(!is_shell("Discord.exe"));
    }

    #[test]
    fn a_pattern_with_a_star_needs_its_prefix_and_a_plain_one_the_whole_name() {
        assert!(matches("vgc", "vgc"));
        assert!(!matches("vgc", "vgcx"));
        assert!(matches("easyanticheat*", "easyanticheat"));
        assert!(!matches("easyanticheat*", "easy"));
    }
}
