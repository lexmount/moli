# Unicode Data

`parley_data` packages the Unicode data that Parley's text analysis and shaping pipeline needs at runtime. It exposes a locale-invariant `CompositeProps` data backed by a compact `CodePointTrie`, allowing the engine to obtain all required character properties with a single lookup.

## What is included

- `CompositeProps`, a trie that holds script, general category, grapheme cluster break, bidi class, and several emoji-related flags per scalar value.

## Cargo features

- `baked` *(default)* embeds pre-generated ICU4X and composite data from `src/generated`, enabling use in `no_std` targets without a filesystem.

## Regenerating the baked data

Simply run the below to regenerate the baked data.

```
cd <REPO_ROOT>
cargo run -p parley_data_gen -- ./parley_data/src/generated
```

The generator downloads the latest ICU4X upstream data and recomputes the composite trie to ensure Parley tracks the current Unicode release.

## Why have this crate?

You may wonder why we can't simply run `parley_data_gen` within a `build.rs` file of `Parley`. Although being possible, that option increases build time and requires a `std` compatible environment.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

[Rust code of conduct]: https://www.rust-lang.org/policies/code-of-conduct