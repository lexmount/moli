// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Preview a selection of cases used for testing linebreaking against a browser reference.
//!
//! Run using:
//! ```sh
//! cargo run -p parley_linebreaking_cases --bin preview_cases -- [starting_seed] [count]
//! ```

use parley_linebreaking_cases::Case;

fn main() {
    let mut args = std::env::args();
    let _ = args.next();
    let starting_seed = args.next().map(|it| it.parse::<u64>()).unwrap_or(Ok(0));
    let count = args.next().map(|it| it.parse::<u64>()).unwrap_or(Ok(30));
    let trailing = args.next();
    // TODO: Use let chains.
    if let (Ok(starting_seed), Ok(count), None) = (starting_seed, count, trailing) {
        for seed in starting_seed..(starting_seed + count) {
            let case = Case::from_seed(seed);
            println!(
                "case {seed:3} ({:^12}): {}",
                format!("{:?}", case.strategy),
                case.text
            );
        }
    } else {
        eprintln!(
            "Usage: cargo run -p parley_linebreaking_cases --bin preview_cases -- <starting_seed> <count>"
        );
    }
}
