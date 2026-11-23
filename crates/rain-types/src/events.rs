//! The live stream: the frame envelope, the topics and the three internal frames.
//! Foundation section 5.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::settings::Settings;
use crate::system::Health;

/// One message on the stream (`GET /stream`). Every frame carries a full snapshot, never a patch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    // The topic name, for example `sessions` (see `TOPIC_NAMES`), or an internal frame name.
    pub topic: String,
    // One counter per backend process; +1 for every frame published to all clients; starts at 1.
    pub seq: u64,
    // The snapshot. Its type depends on `topic`, so the frontend narrows it after reading `topic`.
    #[ts(type = "unknown")]
    pub payload: Value,
}

/// Ties a topic name to the type of its payload, so a publisher cannot send the wrong shape:
/// `bus.publish::<Sessions>(&payload)` only compiles for a `Sessions::Payload`.
pub trait Topic {
    const NAME: &'static str;
    type Payload: Serialize;
}

// The six topics that exist in the frontend contract today. Their payload types move into this
// crate in the stage that makes the topic real (the stage is named at each topic). Until then the
// payload is plain JSON, which keeps the name and the publish call stable.

/// Whole list of Claude sessions. Typed in stage 8.
pub struct Sessions;
impl Topic for Sessions {
    const NAME: &'static str = "sessions";
    type Payload = Value;
}

/// Whole list of scheduled jobs. Typed in stage 6.
pub struct Jobs;
impl Topic for Jobs {
    const NAME: &'static str = "jobs";
    type Payload = Value;
}

/// Whole list of idle jobs. Typed in stage 6.
pub struct IdleJobs;
impl Topic for IdleJobs {
    const NAME: &'static str = "idleJobs";
    type Payload = Value;
}

/// One launch run; the client matches on `run.id`. Typed in stage 4.
pub struct Launch;
impl Topic for Launch {
    const NAME: &'static str = "launch";
    type Payload = Value;
}

/// One disk scan; the client matches on `scan.id`. Typed in stage 3.
pub struct Scan;
impl Topic for Scan {
    const NAME: &'static str = "scan";
    type Payload = Value;
}

/// The state of game mode. Typed in stage 9.
pub struct GameMode;
impl Topic for GameMode {
    const NAME: &'static str = "gameMode";
    type Payload = Value;
}

/// The whole `Settings` object. Published after a change and when `hotkeyAvailable` changes.
/// Named `SettingsTopic` because `Settings` is the payload type.
pub struct SettingsTopic;
impl Topic for SettingsTopic {
    const NAME: &'static str = "settings";
    type Payload = Settings;
}

/// Config problems and the state of every data source (section 6.9).
pub struct HealthTopic;
impl Topic for HealthTopic {
    const NAME: &'static str = "health";
    type Payload = Health;
}

/// Every topic name a subscriber can ask for. Generated into the TypeScript file as `TOPIC_NAMES`.
/// One line per topic, so that a feature section adds its own line and touches no other.
pub const TOPIC_NAMES: &[&str] = &[
    Sessions::NAME,
    Jobs::NAME,
    IdleJobs::NAME,
    Launch::NAME,
    Scan::NAME,
    GameMode::NAME,
    SettingsTopic::NAME,
    HealthTopic::NAME,
];

// ---- internal frames: not topics, not for subscribers ----

/// Payload of the internal frame `hello`: the first frame of every connection, written to that
/// client only. A new `startId` tells the client that the backend is a different process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub start_id: String,
    pub version: String,
    pub api_version: u32,
    pub seq: u64,
}

/// Why the client must refetch everything it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ResyncReason {
    /// The client just connected.
    Connect,
    /// The client read too slowly and frames were dropped; see `Resync::missed`.
    Lagged,
}

/// Payload of the internal frame `resync`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Resync {
    #[ts(inline)]
    pub reason: ResyncReason,
    // Number of frames that were dropped. Only present when `reason` is `lagged`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed: Option<u64>,
}

/// Why the backend is stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    GameMode,
    Stop,
    Upgrade,
    Restart,
}

/// What the `lifecycle` frame announces. There is one kind today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LifecycleKind {
    Stopping,
}

/// Payload of the internal frame `lifecycle`: the last frame before the stream closes on a
/// planned stop. Its absence tells the client the backend crashed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Lifecycle {
    #[ts(inline)]
    pub kind: LifecycleKind,
    #[ts(inline)]
    pub reason: StopReason,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;
    use serde_json::json;

    #[test]
    fn frame_is_the_envelope_of_the_stream() {
        let frame = Frame {
            topic: "launch".into(),
            seq: 42,
            payload: json!({ "id": "run-1", "status": "running" }),
        };
        assert_eq!(
            round_trip(&frame),
            json!({
                "topic": "launch",
                "seq": 42,
                "payload": { "id": "run-1", "status": "running" }
            })
        );
    }

    #[test]
    fn topic_names_are_unique_and_camel_case() {
        for name in TOPIC_NAMES {
            let mut letters = name.chars();
            assert!(
                letters
                    .next()
                    .is_some_and(|first| first.is_ascii_lowercase())
                    && letters.all(|letter| letter.is_ascii_alphanumeric()),
                "{name} is not camelCase"
            );
        }
        let mut sorted = TOPIC_NAMES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            TOPIC_NAMES.len(),
            "a topic name appears twice"
        );
    }

    #[test]
    fn hello_uses_camel_case_keys() {
        let hello = Hello {
            start_id: "19a3f1c2-1f4c".into(),
            version: "0.1.0".into(),
            api_version: 1,
            seq: 0,
        };
        assert_eq!(
            round_trip(&hello),
            json!({ "startId": "19a3f1c2-1f4c", "version": "0.1.0", "apiVersion": 1, "seq": 0 })
        );
    }

    #[test]
    fn resync_on_connect_has_no_missed_count() {
        let resync = Resync {
            reason: ResyncReason::Connect,
            missed: None,
        };
        assert_eq!(round_trip(&resync), json!({ "reason": "connect" }));
    }

    #[test]
    fn resync_on_lag_carries_the_missed_count() {
        let resync = Resync {
            reason: ResyncReason::Lagged,
            missed: Some(1936),
        };
        assert_eq!(
            round_trip(&resync),
            json!({ "reason": "lagged", "missed": 1936 })
        );
    }

    #[test]
    fn lifecycle_names_the_stop_reason() {
        let lifecycle = Lifecycle {
            kind: LifecycleKind::Stopping,
            reason: StopReason::GameMode,
        };
        assert_eq!(
            round_trip(&lifecycle),
            json!({ "kind": "stopping", "reason": "gameMode" })
        );
    }

    #[test]
    fn every_stop_reason_reads_back() {
        for name in ["gameMode", "stop", "upgrade", "restart"] {
            let reason: StopReason = serde_json::from_value(json!(name)).unwrap();
            assert_eq!(serde_json::to_value(reason).unwrap(), json!(name));
        }
    }
}
