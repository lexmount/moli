//! Holds distinct Inspector messages for external allocation and memory profiling.
//! Uses System, matching the carrier example, not Moli's PartitionAlloc shim.
use moli_inspector_json::InspectorJson;
use serde_json::{Value, json};
use std::{hint::black_box, path::PathBuf};

fn main() {
    let mut fixture = None;
    let mut mode = "raw".to_owned();
    let mut count = 16usize;
    let mut clones = 1usize;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--fixture" => fixture = args.next().map(PathBuf::from),
            "--mode" => mode = args.next().expect("--mode value|raw|raw-view"),
            "--messages" => count = args.next().unwrap().parse().unwrap(),
            "--clones" => clones = args.next().unwrap().parse().unwrap(),
            _ => panic!("unknown argument: {argument}"),
        }
    }
    assert!(count > 0 && clones > 0);
    let bytes = std::fs::read(fixture.expect("--fixture PATH is required")).unwrap();
    let mut messages = Vec::with_capacity(count.checked_mul(clones).unwrap());
    for _ in 0..count {
        let message = match mode.as_str() {
            "value" => InspectorJson::from(serde_json::from_slice::<Value>(&bytes).unwrap()),
            "raw" | "raw-view" => InspectorJson::from_slice(&bytes).unwrap(),
            _ => panic!("unknown mode: {mode}"),
        };
        if mode == "raw-view" {
            black_box(message.value());
        }
        for _ in 0..clones {
            messages.push(message.clone());
        }
    }
    // Sample while the queue is live. No sleeps, background sampling or parsing
    // of the payload solely for measurement; those would change retained memory.
    let memory = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    let kib = |field: &str| {
        memory.lines().find_map(|line| {
            let mut columns = line.split_ascii_whitespace();
            (columns.next()? == field)
                .then(|| columns.next()?.parse::<u64>().ok())
                .flatten()
        })
    };
    println!(
        "{}",
        json!({
            "mode":mode,"distinct_messages":count,"clones":clones,
            "queue_entries":messages.len(),"fixture_bytes":bytes.len(),
            "rss_kib":kib("Rss:"),"pss_kib":kib("Pss:")
        })
    );
    black_box(&messages);
}
