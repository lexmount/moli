# Parley Benchmarks

A suite of benchmarks used to evaluate Parley performance.

This uses [Tango](https://github.com/bazhenov/tango) to perform paired benchmarking.

## Setup

Install [`cargo-export`](https://crates.io/crates/cargo-export) via:

```sh
cargo install cargo-export
```

## Usage

```shell
# Capture a baseline via
cargo export target/benchmarks -- bench --bench=main

# Apply changes to Parley

# Compare changes with baseline
cargo bench -q --bench=main -- compare ../target/benchmarks/main
```
