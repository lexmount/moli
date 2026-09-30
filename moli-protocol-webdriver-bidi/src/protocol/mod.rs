//! WebDriver BiDi protocol adapter surface.
//!
//! This crate is intentionally thin. It serializes protocol-neutral
//! `moli-protocol::automation` events into WebDriver BiDi wire shapes and keeps
//! only the connection-local subscription and protocol bridge state needed until
//! the shared typed EventHub owns live fan-out.

mod connection;
mod event_manager;
mod events;
mod types;

pub use connection::BidiConnectionState;
pub use event_manager::BidiEventSourceHookPlan;
pub use types::{
    BidiCommand, BidiCommandOutcome, BidiDevToolsCommandContext, BidiDevToolsCommandDispatch,
    BidiError, BidiErrorCode, BidiInputCommand, BidiInputCommandDispatch, BidiSessionRegistry,
    parse_bidi_command,
};
