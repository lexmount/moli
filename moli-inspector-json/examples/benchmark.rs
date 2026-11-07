//! Benchmarks the same captured Inspector messages through both JSON carriers.
//! Run with --fixtures DIR, optionally --rounds N and --mode value|raw|raw-view.
use moli_inspector_json::InspectorJson;
use serde_json::{Value, json};
use std::{hint::black_box, path::PathBuf, time::Instant};

fn pipeline(bytes: &[u8], mode: &str) -> String {
    match mode {
        "value" => {
            let source: Value = serde_json::from_slice(bytes).unwrap();
            let observed = source.clone();
            let mut queued = source.clone();
            queued["id"] = json!(1234567);
            queued["sessionId"] = json!("SID-benchmark");
            black_box(&observed);
            serde_json::to_string(&queued).unwrap()
        }
        "raw" | "raw-view" => {
            let source = InspectorJson::from_slice(bytes).unwrap();
            let observed = source.clone();
            let mut queued = source.clone();
            queued.set_id(1234567);
            queued.set_session_id(Some("SID-benchmark"));
            if mode == "raw-view" {
                black_box(queued.value());
            }
            black_box(&observed);
            serde_json::to_string(&queued).unwrap()
        }
        _ => panic!("unknown mode: {mode}"),
    }
}

fn main() {
    let mut fixtures = None;
    let mut rounds = 5;
    let mut mode = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--fixtures" => fixtures = args.next().map(PathBuf::from),
            "--rounds" => rounds = args.next().unwrap().parse::<usize>().unwrap(),
            "--mode" => mode = args.next(),
            _ => panic!("unknown argument: {argument}"),
        }
    }
    let fixtures = fixtures.expect(
        "--fixtures DIR is required; capture them with scripts/benchmark-inspector-json.mjs",
    );
    println!("case,mode,round,bytes,iterations,ns_per_iteration");
    for (name, iterations) in [
        ("evaluate-1k", 10000),
        ("evaluate-100k", 1000),
        ("evaluate-1m", 100),
        ("properties", 1000),
    ] {
        let bytes = std::fs::read(fixtures.join(format!("{name}.json"))).unwrap();
        let expected: Value = serde_json::from_str(&pipeline(&bytes, "value")).unwrap();
        for candidate in ["raw", "raw-view"] {
            let output: Value = serde_json::from_str(&pipeline(&bytes, candidate)).unwrap();
            assert_eq!(
                output, expected,
                "{name}/{candidate} must retain protocol semantics"
            );
        }
        for round in 0..rounds {
            let modes = ["value", "raw", "raw-view"];
            for index in 0..modes.len() {
                let candidate = modes[(index + round) % modes.len()];
                if mode.as_deref().is_some_and(|mode| mode != candidate) {
                    continue;
                }
                for _ in 0..20 {
                    black_box(pipeline(&bytes, candidate));
                }
                let started = Instant::now();
                for _ in 0..iterations {
                    black_box(pipeline(black_box(&bytes), candidate));
                }
                let ns = started.elapsed().as_nanos() as f64 / iterations as f64;
                println!(
                    "{name},{candidate},{round},{},{iterations},{ns:.1}",
                    bytes.len()
                );
            }
        }
    }
}
