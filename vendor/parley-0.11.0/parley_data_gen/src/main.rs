// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A small CLI that refreshes the Unicode artefacts checked into the `parley_data` crate.
//! It pulls data from the canonical ICU4X upstream sources, recomputes Parley's composite property trie, and
//! writes Rust modules that can be embedded directly into the repository.

fn main() {
    use std::{env, ffi::OsString, path::PathBuf, process};

    let mut args = env::args_os();
    let exe = args
        .next()
        .unwrap_or_else(|| OsString::from("parley_data_gen"));

    let Some(out_arg) = args.next() else {
        eprintln!(
            "Usage: {} <output-dir> [--compression=N --unsafe]",
            exe.to_string_lossy()
        );
        process::exit(1);
    };

    let out_path = PathBuf::from(out_arg);

    if let Err(err) = std::fs::create_dir_all(&out_path) {
        eprintln!(
            "Failed to create output directory '{}': {}",
            out_path.display(),
            err
        );
        process::exit(1);
    }

    let remaining: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();

    let compression = remaining
        .iter()
        .find_map(|a| a.strip_prefix("--compression="))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(1.0);
    let unsafe_access = remaining.iter().any(|a| a == "--unsafe");

    let config = parley_data_gen::Config {
        compression,
        unsafe_access,
    };
    parley_data_gen::generate(out_path, &config);
}
