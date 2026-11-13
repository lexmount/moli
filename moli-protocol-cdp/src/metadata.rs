use std::{
    io::{BufReader, Read},
    sync::OnceLock,
};

/// Final supported CDP schema, validated at build time and Brotli-compressed at quality 11.
pub const CDP_PROTOCOL_BROTLI: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/cdp-protocol.json.br"));

/// Decode the final discovery response once, without runtime schema processing.
pub fn advertised_protocol_json() -> &'static str {
    static JSON: OnceLock<String> = OnceLock::new();
    JSON.get_or_init(|| {
        let decoder = brotlic::DecompressorReader::new(CDP_PROTOCOL_BROTLI);
        let mut json = String::new();
        BufReader::new(decoder)
            .read_to_string(&mut json)
            .expect("decode embedded CDP protocol metadata");
        json
    })
}
