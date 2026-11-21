//! The IPC contract with the UI. Everything here is exported to `ui/src/bindings.ts`.

pub mod commands;
mod error;
pub mod events;
pub mod types;

pub use error::IpcError;
