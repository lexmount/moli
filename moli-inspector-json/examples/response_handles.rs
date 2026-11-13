//! Isolates ownership projection with live raw envelopes for time and peak-RSS A/B.
//! Use one fresh process per mode and fixture; both modes use the System allocator.
use moli_inspector_json::InspectorJson;
use serde_json::{Value, json};
use std::{collections::BTreeSet, hint::black_box, time::Instant};

fn materialized_handles(message: &InspectorJson) -> Vec<String> {
    let mut ids = Vec::new();
    let mut stack = vec![(message.value(), 1000usize)];
    while let Some((value, depth)) = stack.pop() {
        let Some(next) = depth.checked_sub(1) else {
            continue;
        };
        match value {
            Value::Object(fields) => {
                for name in ["objectId", "promiseObjectId", "errorObjectId"] {
                    if let Some(id) = fields.get(name).and_then(Value::as_str) {
                        ids.push(id.to_owned());
                    }
                }
                stack.extend(fields.values().map(|value| (value, next)));
            }
            Value::Array(values) => stack.extend(values.iter().map(|value| (value, next))),
            _ => {}
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args
        .next()
        .expect("response_handles materialize|project FIXTURE [MESSAGES]");
    let bytes = std::fs::read(args.next().expect("fixture path")).unwrap();
    let count: usize = args.next().map_or(1, |value| value.parse().unwrap());
    assert!(matches!(mode.as_str(), "materialize" | "project") && count > 0);
    let mut queue = Vec::with_capacity(count);
    let started = Instant::now();
    let mut handles = 0;
    let mut object_ids = BTreeSet::new();
    for _ in 0..count {
        let message = InspectorJson::from_slice(&bytes).unwrap();
        let ids = if mode == "materialize" {
            materialized_handles(&message)
        } else {
            message.response_remote_object_ids().unwrap()
        };
        handles += black_box(&ids).len();
        object_ids.extend(ids);
        queue.push(message);
    }
    let nanos = started.elapsed().as_nanos();
    let memory = std::fs::read_to_string("/proc/self/status").unwrap();
    let kib = |field: &str| {
        memory.lines().find_map(|line| {
            let mut parts = line.split_ascii_whitespace();
            (parts.next()? == field)
                .then(|| parts.next()?.parse::<u64>().ok())
                .flatten()
        })
    };
    println!(
        "{}",
        json!({"mode":mode,"messages":count,"fixture_bytes":bytes.len(),"handles":handles,"object_ids":object_ids,"ns_per_message":nanos/count as u128,"peak_rss_kib":kib("VmHWM:"),"rss_kib":kib("VmRSS:")})
    );
    black_box(&queue);
}
