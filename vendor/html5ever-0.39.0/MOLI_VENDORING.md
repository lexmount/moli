# html5ever 0.39.0

This directory contains the source of the crates.io `html5ever` 0.39.0 release.
The upstream source is licensed under MIT or Apache-2.0; see `LICENSE-MIT`
and `LICENSE-APACHE`.

Moli needs original source positions when preparing parser-created scripts,
including scripts in child Documents. The local patch adds an opt-in
`TokenizerOpts::track_tag_positions` and a backward-compatible
`TokenSink::process_tag_token_with_position` hook. Positions identify the next
input character after a completed tag, using one-based lines and UTF-16 columns.
CR, LF and CRLF are normalized; decoded character references do not add source
newlines or columns. Existing sinks retain the original token API and tracking
defaults to disabled.

The cursor follows raw input consumption, SIMD text prefixes, committed
lookahead and character-reference rewinds. It does not buffer or reserialize
the input. Leading BOM handling is consumed once per tokenizer so later BOMs
remain text even when they arrive at an input chunk boundary.

The upstream tokenizer and driver tests are retained. Additional tokenizer
tests cover input splits, entity rewinds, newline normalization, UTF-16,
SIMD-sized text, BOMs, EOF, default sinks and profiling. This crate is excluded
from the workspace, so run its tests explicitly with
`cargo test --manifest-path vendor/html5ever-0.39.0/Cargo.toml --all-features`.
