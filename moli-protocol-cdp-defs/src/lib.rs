//! CDP action types and static support catalog shared by dispatch and schema generation.

pub mod actions;
mod events;
pub use events::LocalCdpEvent;
mod support;

pub use support::{
    DomainMembers, FORWARDED_COMMAND_DOMAINS, FORWARDED_EVENT_DOMAINS, IMPLEMENTED_COMMANDS,
    SERVER_COMMANDS,
};
