/// Complete, compact upstream browser and V8 schemas. The discovery endpoint
/// filters commands and events to Moli's supported surface before serving them.
pub const CDP_PROTOCOL_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdp-protocol.json"));
