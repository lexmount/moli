// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fontique benchmarks.
//!
//! Goal: Provide a repeatable measurement point for changes to system font enumeration,
//! such as switching from a directory scan to a CoreText "available fonts" enumeration.

use tango_bench::Benchmark;

#[cfg(target_vendor = "apple")]
use parley::fontique::{Collection, CollectionOptions};
#[cfg(target_vendor = "apple")]
use std::hint::black_box;
#[cfg(target_vendor = "apple")]
use tango_bench::benchmark_fn;

/// Benchmark: one-time initialization cost of a system-font-backed `Collection`.
///
/// Notes:
/// - On Apple platforms this exercises the CoreText backend (system font enumeration + scanning).
/// - Use this benchmark to compare two commits/branches (e.g. before/after a PR).
#[cfg(target_vendor = "apple")]
pub fn system_fonts_init() -> Vec<Benchmark> {
    vec![benchmark_fn(
        "Fontique - system fonts init (CoreText)",
        |b| {
            b.iter(|| {
                // Creating the collection triggers system font enumeration and scanning/parsing
                // work (e.g. reading font name tables).
                let mut collection = Collection::new(CollectionOptions {
                    shared: false,
                    system_fonts: true,
                });

                // Force a read to prevent the optimizer from treating initialization as unused.
                let family_count = collection.family_names().count();
                black_box(family_count);
            })
        },
    )]
}

/// Non-Apple platforms: skip this benchmark to avoid meaningless cross-platform noise.
#[cfg(not(target_vendor = "apple"))]
pub fn system_fonts_init() -> Vec<Benchmark> {
    Vec::new()
}
