/// Complete upstream browser and V8 schemas, compacted and Brotli-compressed at quality 11.
pub const CDP_PROTOCOL_BROTLI: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/cdp-protocol.json.br"));

/// Decode the upstream schema on demand. The discovery endpoint caches its
/// filtered result, without retaining another copy of the unfiltered JSON.
pub fn upstream_protocol_schema() -> serde_json::Value {
    serde_json::from_reader(brotlic::DecompressorReader::new(CDP_PROTOCOL_BROTLI))
        .expect("decode embedded CDP protocol metadata")
}
