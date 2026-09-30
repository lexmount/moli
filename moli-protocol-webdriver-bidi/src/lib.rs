//! WebDriver BiDi protocol adapter surface.
//!
//! This crate serializes protocol-neutral `moli-protocol::automation`
//! events into WebDriver BiDi wire shapes and keeps the connection-local state
//! needed to bridge commands until shared protocol owner state fans out events.

mod browsing_context;
mod commands;
mod events;
mod network;
mod protocol;
mod responses;
mod script_values;
mod storage;
mod user_context;

pub use commands::automation_command_from_bidi_command;
pub use events::{
    bidi_event_from_automation_event, bidi_event_from_protocol_message, script_realm_created_event,
    script_realm_destroyed_event,
};
pub use protocol::{
    BidiCommand, BidiCommandOutcome, BidiConnectionState, BidiDevToolsCommandContext,
    BidiDevToolsCommandDispatch, BidiError, BidiErrorCode, BidiEventSourceHookPlan,
    BidiInputCommand, BidiInputCommandDispatch, BidiSessionRegistry, parse_bidi_command,
};
pub use responses::{
    bidi_response_from_devtools_error, bidi_response_from_devtools_result, error_response,
    success_response,
};

#[cfg(test)]
mod tests;
