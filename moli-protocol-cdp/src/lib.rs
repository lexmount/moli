//! Chrome DevTools Protocol wire helpers for Moli.
//!
//! This crate owns CDP-specific message parsing and Chrome protocol metadata.
//! Protocol-neutral owner state and command execution live in
//! `moli-protocol`.

mod dom;
mod metadata;
mod runtime;
mod target;
mod time;
mod wire;

pub use dom::{
    backend_node_id_for_snapshot, collect_flattened_node_snapshot, frontend_node_id_for_snapshot,
    node_snapshot_base_payload, node_snapshot_to_cdp, node_snapshot_to_cdp_with_limit,
};
pub use metadata::CDP_PROTOCOL_JSON;
pub use runtime::remote_object_from_json_value;
pub use target::{CdpTargetKindWire, CdpTargetType, cdp_target_type_wire_value};
pub use time::{DEFAULT_LOADER_ID, monotonic_timestamp_seconds};
pub use wire::{
    CdpInspectorTaskMode, CdpRendererCommandAccess, CdpRendererCommandPolicy,
    CdpRendererCommandReplacement, CdpRendererCommandReplayDispatch, CdpRequest, ParsedCdpCommand,
};
