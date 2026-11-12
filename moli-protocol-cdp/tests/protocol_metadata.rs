use std::io::Read;

use moli_protocol_cdp::{CDP_PROTOCOL_BROTLI, upstream_protocol_schema};
use serde_json::Value;

#[test]
fn embedded_protocol_preserves_the_complete_pinned_schemas() {
    let mut original: Value =
        serde_json::from_str(include_str!("../protocol/browser_protocol.json")).unwrap();
    let mut javascript: Value =
        serde_json::from_str(include_str!("../protocol/js_protocol.json")).unwrap();
    original["domains"]
        .as_array_mut()
        .unwrap()
        .append(javascript["domains"].as_array_mut().unwrap());
    let embedded = upstream_protocol_schema();
    assert_eq!(embedded, original);
}

#[test]
fn embedded_brotli_protocol_round_trips_byte_for_byte() {
    let compact = include_bytes!(concat!(env!("OUT_DIR"), "/cdp-protocol.json"));
    let mut decoded = Vec::new();
    brotlic::DecompressorReader::new(CDP_PROTOCOL_BROTLI)
        .read_to_end(&mut decoded)
        .expect("decompress embedded protocol");
    assert_eq!(decoded, compact.as_slice());
    assert!(CDP_PROTOCOL_BROTLI.len() < compact.len() / 2);
}
