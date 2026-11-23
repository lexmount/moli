Before committing changes that modify Rust source code or Rust build metadata
(such as `Cargo.toml`, `Cargo.lock`, or `rust-toolchain`), run formatting,
Clippy, and Nextest from the repository root and ensure they pass.

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

For local macOS work, default to focused Nextest tests covering the changed
code. On other platforms or when the user requests the full suite, run
`cargo nextest run --no-fail-fast`.

These commands are not required when the change set contains no Rust source or
Rust build metadata changes.
