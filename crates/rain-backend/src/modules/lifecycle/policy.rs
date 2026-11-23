//! Which reasons for stopping the backend are refused, and when. Section 06 6.9.
//!
//! | Reason | Refused when |
//! |---|---|
//! | `gameMode` | never |
//! | `stop` | never |
//! | `upgrade` | work is running (`busy`) |
//! | `restart` | work is running, or game mode is not off |
//!
//! A busy backend is never ended by `upgrade` or `restart`: the shell keeps the old one and tries
//! again at its next start. Only `stop` and `--stop --force` end running work. `force` skips the
//! refusals; the stop route of core calls this before it accepts a request.

use rain_types::StopReason;

use crate::core::error::{ApiError, ApiResult};

const SOURCE: &str = "lifecycle";

pub fn check_stop(
    reason: StopReason,
    force: bool,
    busy: bool,
    game_mode_active: bool,
) -> ApiResult<()> {
    if force {
        return Ok(());
    }
    match reason {
        StopReason::GameMode | StopReason::Stop => Ok(()),
        StopReason::Upgrade | StopReason::Restart if busy => Err(busy_error()),
        StopReason::Restart if game_mode_active => {
            Err(ApiError::conflict("gameMode", "Game mode is running.").with_source(SOURCE))
        }
        StopReason::Upgrade | StopReason::Restart => Ok(()),
    }
}

fn busy_error() -> ApiError {
    ApiError::conflict("busy", "Wait until the running jobs end, then try again.")
        .with_source(SOURCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason_of(result: ApiResult<()>) -> Option<String> {
        result.err().and_then(|error| error.body.reason)
    }

    #[test]
    fn game_mode_and_stop_are_never_refused() {
        for reason in [StopReason::GameMode, StopReason::Stop] {
            assert!(check_stop(reason, false, true, true).is_ok(), "{reason:?}");
        }
    }

    #[test]
    fn upgrade_and_restart_wait_for_running_work() {
        for reason in [StopReason::Upgrade, StopReason::Restart] {
            assert_eq!(
                reason_of(check_stop(reason, false, true, false)).as_deref(),
                Some("busy"),
                "{reason:?}"
            );
            assert!(check_stop(reason, false, false, false).is_ok());
        }
    }

    #[test]
    fn a_restart_is_refused_while_game_mode_runs_but_an_upgrade_is_not() {
        assert_eq!(
            reason_of(check_stop(StopReason::Restart, false, false, true)).as_deref(),
            Some("gameMode")
        );
        assert!(check_stop(StopReason::Upgrade, false, false, true).is_ok());
    }

    #[test]
    fn force_ends_the_refusals() {
        assert!(check_stop(StopReason::Restart, true, true, true).is_ok());
        assert!(check_stop(StopReason::Upgrade, true, true, false).is_ok());
    }

    #[test]
    fn the_refusals_are_conflicts() {
        let error = check_stop(StopReason::Upgrade, false, true, false).unwrap_err();
        assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
        assert_eq!(error.body.source.as_deref(), Some("lifecycle"));
    }
}
