<div align="center">

# Parley

**Rich text layout**

[![Latest published parley version.](https://img.shields.io/crates/v/parley.svg)](https://crates.io/crates/parley)
[![Documentation build status.](https://img.shields.io/docsrs/parley.svg)](https://docs.rs/parley)
[![Dependency staleness status.](https://deps.rs/crate/parley/latest/status.svg)](https://deps.rs/crate/parley)
[![Linebender Zulip chat.](https://img.shields.io/badge/Linebender-%23parley-blue?logo=Zulip)](https://xi.zulipchat.com/#narrow/channel/205635-parley)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)

</div>

Parley provides an API for implementing rich text layout.

## The Parley text stack

Parley has four key dependencies: Fontique, HarfRust, Skrifa, and ICU4X. These crates cover different pieces of the text-rendering process.

### Fontique

Fontique provides font enumeration and fallback.

**Font enumeration** means listing (enumerating) all the fonts installed on the system.
It also means collecting metadata about those fonts: whether they are serif, sans-serif, monospace, their weight, the code points they cover, etc.
The library is responsible for loading fonts into memory; it will use memory-mapped IO to load portions into memory lazily and share them between processes on the system.

**Font fallback** is matching runs of text to a font.
This is necessary because fonts typically don't cover the entire Unicode range: you have different fonts for latin text, chinese text, arabic text, etc and also usually a separate font for emoji.
But if you have, say arabic text or emoji embedded within latin text, you don't typically specify the font for the arabic text or the emoji, one is chosen for you.
Font fallback is the process which makes that choice.

### HarfRust

HarfRust is a Rust port of HarfBuzz text shaping engine. **Text shaping** means mapping runs of Unicode codepoints to specific glyphs within fonts.
This includes applying ligatures, resolving emoji modifiers, but also much more complex transformations for some scripts.

### Skrifa

Skrifa reads TrueType and OpenType fonts.

It is built on top of the [read-fonts](https://github.com/googlefonts/fontations/tree/main/read-fonts) low-level parsing library and is also part of the [oxidize](https://github.com/googlefonts/oxidize) project.

Skrifa provides higher level metrics on top of read-fonts.
Notably it converts the raw glyph representations in font files into scaled, hinted vector paths suitable for rasterisation.

### ICU4X

ICU4X enables text analysis and internationalisation. For Parley, this includes locale and language recognition, 
bidirectional text evaluation, text segmentation, emoji recognition, NFC/NFD normalisation and other Unicode character information.

### Parley

Parley itself does text layout and includes utilities for text selection and editing.

**Text layout** means computing x/y coordinates for each glyph in a string of text.
Besides what the other libraries do, this involves things like determining a glyph's size, line breaking, and bidi resolution.

## Web Platform Tests

The [Web Platform Tests](https://github.com/web-platform-tests/wpt) contain a large number of tests for correct text layout. These can be run against Parley by using [Blitz](https://github.com/dioxuslabs/blitz)'s WPT test runner. See: the
[Parley WPT guide](https://github.com/DioxusLabs/blitz/blob/main/docs/parley.md).

## Minimum supported Rust Version (MSRV)

This version of Parley has been verified to compile with **Rust 1.88** and later.

Future versions of Parley might increase the Rust version requirement.
It will not be treated as a breaking change and as such can even happen with small patch releases.

<details>
<summary>Click here if compiling fails.</summary>

As time has passed, some of Parley's dependencies could have released versions with a higher Rust requirement.
If you encounter a compilation issue due to a dependency and don't want to upgrade your Rust toolchain, then you could downgrade the dependency.

```sh
# Use the problematic dependency's name and version
cargo update -p package_name --precise 0.1.1
```
</details>

## Community

Discussion of Parley development happens in the [Linebender Zulip](https://xi.zulipchat.com/), specifically the [#parley channel](https://xi.zulipchat.com/#narrow/channel/205635-parley).
All public content can be read without logging in.

Contributions are welcome by pull request. The [Rust code of conduct] applies.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache 2.0 license, shall be licensed as noted in the [License](#license) section, without any additional terms or conditions.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Some code in this repository is not available under these licenses:

- `parley_emoji` is licensed only under its MIT license (which has a different attribution to Parley's MIT License).
  See `./parley_emoji/README.md` for details.
- `parley_tests/tests/emoji` is licensed only under the Apache License, Version 2.0 (and not the MIT license).
  See `./parley_tests/tests/emoji/README.md` for details.
- Some other files used for tests and benchmarks are under different licenses.
  See `./parley_dev/README.md` for details.

[Rust code of conduct]: https://www.rust-lang.org/policies/code-of-conduct
