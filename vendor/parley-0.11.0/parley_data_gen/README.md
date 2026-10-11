## Unicode Data Generator

`parley_data_gen` is a small CLI that refreshes the Unicode artefacts checked into the `parley_data` crate. It pulls data from the canonical ICU4X upstream sources, recomputes Parley's composite property trie, and writes Rust modules that can be embedded directly into the repository.

The generator requires a network connection the first time it runs so that `icu_provider_source::SourceDataProvider` can download the latest ICU4X data files.

## Usage

```
cargo run -p parley_data_gen -- <output-dir>
```

The `output-dir` is created if it does not exist. After the command completes, the directory will contain:

- `icu4x_data/`: baked ICU4X data required for segmentation and normalization.
- `composite/`: a postcard blob and Rust module for Parley's `CompositePropsV1` trie.
- `mod.rs`: a convenience module that re-exports the generated content.

To update `parley_data`, copy the generated files into `parley_data/src/generated` (or simply set `<output-dir>` to `./parley_data_src/generated`).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

[Rust code of conduct]: https://www.rust-lang.org/policies/code-of-conduct
