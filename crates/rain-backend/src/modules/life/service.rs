//! The logic and the in-memory state of this module. Other modules call it, and only it.

/// Stub. Section 04 builds the vault, the habits and the daily notes.
#[derive(Debug, Default)]
pub struct Service;

impl Service {
    /// The deep check of a daily-note date format (the token syntax is this module's). The
    /// settings module calls it after its own character rules (section 06 table V). Until the
    /// syntax exists every format that passes the character rules is accepted.
    pub fn check_note_format(&self, _format: &str) -> Result<(), String> {
        Ok(())
    }
}
