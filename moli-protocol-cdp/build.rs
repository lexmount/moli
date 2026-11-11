use std::{env, fs, path::PathBuf};

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
    let destination = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build output directory"))
        .join("cdp-protocol.json");
    fs::write(destination, compact).expect("write compact CDP protocol metadata");
}
