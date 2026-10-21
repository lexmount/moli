use std::{env, fs, path::PathBuf};

const PROTOCOL_SOURCE: &str = "../vendor/v8-152.2.0/v8/include/js_protocol-1.3.json";

fn main() {
    println!("cargo:rerun-if-changed={PROTOCOL_SOURCE}");
    let source = fs::read(PROTOCOL_SOURCE).expect("read vendored CDP protocol metadata");
    let protocol: serde_json::Value =
        serde_json::from_slice(&source).expect("parse vendored CDP protocol metadata");
    let compact = serde_json::to_vec(&protocol).expect("serialize compact CDP protocol metadata");
    let destination = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build output directory"))
        .join("cdp-protocol.json");
    fs::write(destination, compact).expect("write compact CDP protocol metadata");
}
