//! The client side of the Rain backend: one pooled HTTP client over a Windows named pipe, and the
//! reader of the live stream (Server-Sent Events).
//!
//! The shell, the integration tests and `rain-backend --stop` all use this crate, so each of them
//! exercises the real transport. Design: docs/backend/00-foundation.md sections 3.3, 5.7 and 6.10.

mod client;
mod error;
mod sse;

pub use client::{PipeClient, RawResponse, pipe_exists};
pub use error::{error_from_response, transport_error};
pub use sse::{Notice, SseFrame, SseReader, run_stream};
