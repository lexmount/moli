<div align="center">

# Parley Engine

**Low level text layout**

[![Latest published version.](https://img.shields.io/crates/v/parley_engine.svg)](https://crates.io/crates/parley_engine)
[![Documentation build status.](https://img.shields.io/docsrs/parley_engine.svg)](https://docs.rs/parley_engine)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)
\
[![Linebender Zulip chat.](https://img.shields.io/badge/Linebender-%23parley-blue?logo=Zulip)](https://xi.zulipchat.com/#narrow/channel/205635-parley)
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/linebender/parley/ci.yml?logo=github&label=CI)](https://github.com/linebender/parley/actions)
[![Dependency staleness status.](https://deps.rs/crate/parley_engine/latest/status.svg)](https://deps.rs/crate/parley_engine)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=parley_engine --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs should be evaluated here.
See https://linebender.org/blog/doc-include/ for related discussion. -->

<!-- cargo-rdme start -->

Parley Engine provides low level APIs for shaping paragraphs of text.

## Usage

Use [`Analyzer`], [`Analysis`], [`Shaper`] and [`ShapedText`] to shape a paragraph of text into
glyphs. Correct reshaping of lines is in progress; in the meantime you can break text at
[`Atom`] or [`ShapedCluster`][crate::shape::ShapedCluster] boundaries.

Text analysis is performed before shaping, and the same source string must be passed to each
stage.

Higher-level users may prefer using [`parley`][parley], which uses this crate and implements
layout and styling.

```rust
let mut analysis = Analysis::default();
let mut analyzer = Analyzer::default();
let mut shaped_text = ShapedText::default();
let mut shaper = Shaper::default();

let text = "The quick brown ثعلب jumps over the lazy dog.";
let char_count = text.chars().count();
let char_style_indices = vec![0; char_count];

analyzer.analyze(text, &AnalysisOptions::default(), &mut analysis);
shaper.shape_text(
    text,
    &analysis,
    &char_style_indices,
    [Item {
        char_end: char_count.try_into().unwrap(),
        options: ShapeOptions {
            font_size: 16.0,
            language: None,
            features: &[],
            variations: &[],
        },
     }],
     select_font, // Selects fonts covering each cluster.
     &mut shaped_text,
);

for (run_idx, run) in shaped_text.runs().iter().enumerate() {
    let slice = shaped_text.run_slice(run_idx as u32);
    // You can, for example, measure grapheme advances for hit-testing or
    // placing carets.
    for atom in slice.atoms_start() {
        for grapheme in atom.graphemes_start() {
            std::dbg!(grapheme);
        }
    }

    // Or get glyphs for rendering (for simplicity, this iterates clusters in
    // logical order, but for rendering you'd want to reorder runs and clusters
    // according to their `run.bidi_level`).
    for cluster in slice.shaped_clusters_range() {
        for glyph in slice.shaped_cluster_glyphs(cluster) {
            std::dbg!(glyph);
        }
    }
}
```

## Features

- `std` (enabled by default): This is currently unused and is provided for forward compatibility.
- `bytemuck`: Forwarded to [`parlance`], which then implements traits from `bytemuck` on its
  types.

[parley]: https://docs.rs/parley

<!-- cargo-rdme end -->

## Minimum supported Rust Version (MSRV)

This version of Parley Engine has been verified to compile with **Rust 1.88** and later.

Future versions of Parley Engine might increase the Rust version requirement.
It will not be treated as a breaking change and as such can even happen with small patch releases.

<details>
<summary>Click here if compiling fails.</summary>

As time has passed, some of Parley Engine's dependencies could have released versions with a higher Rust requirement.
If you encounter a compilation issue due to a dependency and don't want to upgrade your Rust toolchain, then you could downgrade the dependency.

```sh
# Use the problematic dependency's name and version
cargo update -p package_name --precise 0.1.1
```
</details>

## Community

Discussion of Parley Engine development happens in the [Linebender Zulip](https://xi.zulipchat.com/), specifically the [#parley channel](https://xi.zulipchat.com/#narrow/channel/205635-parley).
All public content can be read without logging in.

Contributions are welcome by pull request. The [Rust code of conduct] applies.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache 2.0 license, shall be licensed as noted in the [License](#license) section, without any additional terms or conditions.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

[Rust code of conduct]: https://www.rust-lang.org/policies/code-of-conduct
