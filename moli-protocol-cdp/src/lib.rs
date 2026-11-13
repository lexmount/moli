//! Chrome DevTools Protocol wire helpers for Moli.
//!
//! This crate owns CDP-specific message parsing and Chrome protocol metadata.
//! Protocol-neutral owner state and command execution live in
//! `moli-protocol`.

mod dom;
mod event_contract;
mod metadata;
mod runtime;
mod target;
mod time;
mod wire;

pub use dom::{
    backend_node_id_for_snapshot, collect_flattened_node_snapshot, frontend_node_id_for_snapshot,
    node_snapshot_base_payload, node_snapshot_to_cdp, node_snapshot_to_cdp_with_limit,
};
pub use event_contract::{CdpEventContractError, validate_event_name, validate_event_names};
pub use metadata::{CDP_PROTOCOL_BROTLI, advertised_protocol_json, is_advertised_event};
pub use runtime::remote_object_from_json_value;
pub use target::{CdpTargetKindWire, CdpTargetType, cdp_target_type_wire_value};
pub use time::{DEFAULT_LOADER_ID, monotonic_timestamp_seconds};
pub use wire::{
    CdpCommandParseError, CdpCommandParseErrorKind, CdpInspectorTaskMode, CdpRendererCommandAccess,
    CdpRendererCommandPolicy, CdpRendererCommandReplacement, CdpRendererCommandReplayDispatch,
    CdpRequest, ParsedCdpCommand,
};
