//! The server: the pipe, the router with its layers, and the routes that belong to the backend
//! itself. Foundation section 11.1.

pub mod builtin;
mod layers;
mod media;
pub mod pipe;
pub mod router;
mod stream;
