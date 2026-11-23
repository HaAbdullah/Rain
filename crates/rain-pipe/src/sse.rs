//! Reads the live stream (Server-Sent Events) from a hyper body, and keeps it open.
//! Copied in shape from the spike `types-stream` (`sse.rs`); see foundation sections 5.4 and 5.7.

use std::time::Duration;

use http_body_util::BodyExt;
use hyper::Method;
use hyper::body::Incoming;
use rain_types::ApiError;
use rain_types::limits::STREAM_BACKOFF;
use tokio_util::sync::CancellationToken;

use crate::client::PipeClient;
use crate::error::transport_error;

/// One frame of a Server-Sent Events stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseFrame {
    /// An event. `data` is the JSON envelope of foundation section 5.1; several `data:` lines are
    /// joined with a newline.
    Event { event: String, data: String },
    /// A comment line such as `: ka`, which the backend sends to show that it is alive.
    Comment(String),
}

/// Cuts a byte stream into frames. A frame ends at a blank line.
///
/// Bytes are kept as bytes until a whole frame has arrived, so a character that a read splits in
/// the middle (UTF-8 text in a payload) is never damaged.
#[derive(Default)]
struct FrameParser {
    buffer: Vec<u8>,
}

impl FrameParser {
    fn push(&mut self, bytes: &[u8]) {
        // The backend ends its lines with "\n" only. A "\r" (from a proxy that writes "\r\n")
        // is dropped.
        self.buffer
            .extend(bytes.iter().filter(|byte| **byte != b'\r'));
    }

    fn take_frame(&mut self) -> Option<SseFrame> {
        let end = self.buffer.windows(2).position(|pair| pair == b"\n\n")?;
        let block: Vec<u8> = self.buffer.drain(..end + 2).collect();
        let block = String::from_utf8_lossy(&block);
        let (mut event, mut data, mut comment) = (String::new(), Vec::new(), None);
        for line in block.lines() {
            if let Some(text) = line.strip_prefix(':') {
                comment = Some(text.trim().to_owned());
            } else if let Some(name) = line.strip_prefix("event:") {
                event = name.trim().to_owned();
            } else if let Some(value) = line.strip_prefix("data:") {
                data.push(value.strip_prefix(' ').unwrap_or(value).to_owned());
            }
        }
        if data.is_empty() && event.is_empty() {
            return Some(SseFrame::Comment(comment.unwrap_or_default()));
        }
        Some(SseFrame::Event {
            event: if event.is_empty() {
                "message".into()
            } else {
                event
            },
            data: data.join("\n"),
        })
    }
}

/// Reads frames from the body of a stream response.
pub struct SseReader {
    body: Incoming,
    parser: FrameParser,
}

impl SseReader {
    pub fn new(body: Incoming) -> Self {
        Self {
            body,
            parser: FrameParser::default(),
        }
    }

    /// The next frame, as soon as its last byte has arrived. `Ok(None)` means that the server
    /// closed the stream.
    pub async fn next(&mut self) -> Result<Option<SseFrame>, ApiError> {
        loop {
            if let Some(frame) = self.parser.take_frame() {
                return Ok(Some(frame));
            }
            match self.body.frame().await {
                None => return Ok(None),
                Some(Err(e)) => return Err(transport_error(&e)),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        self.parser.push(&data);
                    }
                }
            }
        }
    }
}

/// What [`run_stream`] reports to its owner (in the shell: an `app.emit` call).
#[derive(Debug)]
pub enum Notice {
    Connecting { attempt: u32 },
    Connected,
    Frame(SseFrame),
    Disconnected { reason: String },
}

/// Keeps the stream open until `stop` is cancelled.
///
/// - Reconnects forever with the backoff of `STREAM_BACKOFF` (the backend may be restarting). The
///   backoff starts again after any frame.
/// - `idle_limit`: when not even a keep-alive comment arrives for this long, the connection is
///   dead (a half-open pipe gives no error), so it is dropped and opened again.
///
/// The backend sends `hello` and `resync` as the first two frames of every connection.
pub async fn run_stream(
    client: PipeClient,
    path: &str,
    idle_limit: Duration,
    stop: CancellationToken,
    mut on_notice: impl FnMut(Notice),
) {
    let mut attempt = 0u32;
    let mut backoff = 0usize; // index into STREAM_BACKOFF
    while !stop.is_cancelled() {
        attempt += 1;
        on_notice(Notice::Connecting { attempt });
        let response = tokio::select! {
            r = client.send(Method::GET, path, None, &[("accept", "text/event-stream")]) => r,
            () = stop.cancelled() => return,
        };
        match response {
            Ok(response) if response.status().is_success() => {
                on_notice(Notice::Connected);
                let mut reader = SseReader::new(response.into_body());
                let reason = loop {
                    tokio::select! {
                        () = stop.cancelled() => return,
                        got = tokio::time::timeout(idle_limit, reader.next()) => match got {
                            Err(_) => break format!("nothing received for {idle_limit:?}"),
                            Ok(Ok(Some(frame))) => {
                                backoff = 0;
                                on_notice(Notice::Frame(frame));
                            }
                            Ok(Ok(None)) => break "stream closed by the server".to_owned(),
                            Ok(Err(error)) => break error.message,
                        }
                    }
                };
                on_notice(Notice::Disconnected { reason });
            }
            Ok(response) => on_notice(Notice::Disconnected {
                reason: format!("HTTP {}", response.status()),
            }),
            Err(error) => on_notice(Notice::Disconnected {
                reason: error.message,
            }),
        }
        let wait = STREAM_BACKOFF[backoff.min(STREAM_BACKOFF.len() - 1)];
        backoff += 1;
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = stop.cancelled() => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames_of(input: &str) -> Vec<SseFrame> {
        let mut parser = FrameParser::default();
        parser.push(input.as_bytes());
        std::iter::from_fn(|| parser.take_frame()).collect()
    }

    fn event(name: &str, data: &str) -> SseFrame {
        SseFrame::Event {
            event: name.into(),
            data: data.into(),
        }
    }

    #[test]
    fn an_event_has_a_name_and_data() {
        let frames = frames_of("event: jobs\nid: 7\ndata: {\"a\":1}\n\n");
        assert_eq!(frames, [event("jobs", "{\"a\":1}")]);
    }

    #[test]
    fn a_comment_is_a_frame_of_its_own() {
        assert_eq!(frames_of(": ka\n\n"), [SseFrame::Comment("ka".into())]);
    }

    #[test]
    fn several_data_lines_are_joined_with_a_newline() {
        assert_eq!(
            frames_of("data: a\ndata: b\n\n"),
            [event("message", "a\nb")]
        );
    }

    #[test]
    fn carriage_returns_before_the_newlines_are_ignored() {
        assert_eq!(frames_of("event: x\r\ndata: y\r\n\r\n"), [event("x", "y")]);
    }

    #[test]
    fn a_character_split_between_two_reads_is_kept_whole() {
        let bytes = "data: Zoë\n\n".as_bytes();
        let split = bytes.iter().position(|byte| *byte == 0xC3).unwrap() + 1; // inside the ë
        let mut parser = FrameParser::default();
        parser.push(&bytes[..split]);
        assert_eq!(parser.take_frame(), None);
        parser.push(&bytes[split..]);
        assert_eq!(parser.take_frame(), Some(event("message", "Zoë")));
    }

    #[test]
    fn a_frame_that_has_not_ended_waits_for_more_bytes() {
        let mut parser = FrameParser::default();
        parser.push(b"event: x\ndata: ");
        assert_eq!(parser.take_frame(), None);
        parser.push(b"y\n\nevent: z\n");
        assert_eq!(parser.take_frame(), Some(event("x", "y")));
        assert_eq!(parser.take_frame(), None);
    }
}
