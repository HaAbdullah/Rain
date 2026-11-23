//! The command of the autostart entry. Section 06 6.4.
//!
//! The registry value `HKCU\...\Run\Rain` holds `"<shell exe>" --hidden`. The program that starts
//! at logon is the shell, because the global hotkey lives there; the shell then starts the
//! backend. The path is always the one that the shell reported (`PUT /internal/shell`), never a
//! guess.

/// The argument that makes the shell start with a hidden window.
const HIDDEN_FLAG: &str = "--hidden";

/// The path cannot be put into the command (it holds a quote).
#[derive(Debug, PartialEq, Eq)]
pub struct BadExePath;

/// `"<exe>" --hidden`. A path that holds a `"` is refused: it could end the quoting early.
pub fn command_for(shell_exe: &str) -> Result<String, BadExePath> {
    let exe = shell_exe.trim();
    if exe.is_empty() || exe.contains('"') || exe.chars().any(char::is_control) {
        return Err(BadExePath);
    }
    Ok(format!("\"{exe}\" {HIDDEN_FLAG}"))
}

/// Windows compares paths without regard to case, so a value that only differs in case is the
/// same entry.
pub fn same_command(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_the_quoted_exe_and_the_hidden_flag() {
        assert_eq!(
            command_for(r"C:\Program Files\Rain\rain.exe").as_deref(),
            Ok(r#""C:\Program Files\Rain\rain.exe" --hidden"#)
        );
        assert_eq!(
            command_for("  C:\\Rain\\rain.exe ").as_deref(),
            Ok(r#""C:\Rain\rain.exe" --hidden"#)
        );
    }

    #[test]
    fn a_path_with_a_quote_a_control_character_or_nothing_is_refused() {
        for bad in ["C:\\a\"b\\rain.exe", "C:\\a\nb.exe", "", "   "] {
            assert_eq!(command_for(bad), Err(BadExePath), "{bad:?}");
        }
    }

    #[test]
    fn commands_that_differ_only_in_case_are_the_same_entry() {
        assert!(same_command(
            r#""C:\Rain\Rain.exe" --hidden"#,
            r#""c:\rain\rain.exe" --hidden"#
        ));
        assert!(!same_command(
            r#""C:\Rain\rain.exe" --hidden"#,
            r#""C:\Other\rain.exe" --hidden"#
        ));
    }
}
