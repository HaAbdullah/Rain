//! The one COM thread when a call does not return. Foundation section 11.6.
//!
//! This is its own test program on purpose: the test holds the COM thread for a moment, and any
//! other test in the same process that used COM at that time would see "busy".

use std::sync::{Arc, Barrier};
use std::time::Duration;

use rain_backend::platform::com::{ComError, run, run_with_timeout};

#[tokio::test]
async fn a_stuck_call_times_out_then_new_calls_fail_fast_until_it_returns() {
    let release = Arc::new(Barrier::new(2));
    let hold = Arc::clone(&release);
    let stuck = run_with_timeout(
        move || {
            hold.wait(); // the call does not return until the test says so
            Ok(())
        },
        Duration::from_millis(100),
    )
    .await;
    assert_eq!(stuck, Err(ComError::TimedOut));
    assert_eq!(
        run(|| Ok(1)).await,
        Err(ComError::Busy),
        "no queue behind the stuck call"
    );

    release.wait(); // the stuck call returns now
    for _ in 0..100 {
        if run(|| Ok(1)).await == Ok(1) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the COM thread did not recover after the stuck call returned");
}
