use std::{env, fs, io::Write, path::PathBuf};

use brotlic::{BrotliEncoderOptions, CompressorWriter, Quality};

const PROTOCOL_SOURCES: &[&str] = &[
    "protocol/browser_protocol.json",
    "protocol/js_protocol.json",
];

fn main() {
    let mut protocol = serde_json::json!({"version": {"major": "1", "minor": "3"}, "domains": []});
    for source in PROTOCOL_SOURCES {
        println!("cargo:rerun-if-changed={source}");
        let source = fs::read(source).expect("read pinned CDP protocol metadata");
        let mut source: serde_json::Value =
            serde_json::from_slice(&source).expect("parse pinned CDP protocol metadata");
        assert_eq!(source["version"], protocol["version"]);
        protocol["domains"]
            .as_array_mut()
            .expect("protocol domains")
            .append(source["domains"].as_array_mut().expect("source domains"));
    }
    let compact = serde_json::to_vec(&protocol).expect("serialize compact CDP protocol metadata");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build output directory"));
    // Keep the input as a build artifact for the byte-for-byte round-trip test.
    // Only the Brotli resource is embedded in production code.
    fs::write(output.join("cdp-protocol.json"), &compact)
        .expect("write compact CDP protocol metadata");
    let encoder = BrotliEncoderOptions::new()
        .quality(Quality::new(11).expect("Brotli quality 11 must be valid"))
        .build()
        .expect("configure protocol Brotli encoder");
    let mut compressor = CompressorWriter::with_encoder(encoder, Vec::new());
    compressor
        .write_all(&compact)
        .expect("compress CDP protocol metadata");
    let compressed = compressor
        .into_inner()
        .expect("finish protocol Brotli stream");
    fs::write(output.join("cdp-protocol.json.br"), compressed)
        .expect("write compressed CDP protocol metadata");
}
