<div align="center">

# Parley Emoji

**Unicode emoji presentation resolution**

[![Latest published version.](https://img.shields.io/crates/v/parley_emoji.svg)](https://crates.io/crates/parley_emoji)
[![Documentation build status.](https://img.shields.io/docsrs/parley_emoji.svg)](https://docs.rs/parley_emoji)
[![MIT license.](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
\
[![Linebender Zulip chat.](https://img.shields.io/badge/Linebender-%23parley-blue?logo=Zulip)](https://xi.zulipchat.com/#narrow/channel/205635-parley)
[![GitHub Actions CI status.](https://img.shields.io/github/actions/workflow/status/linebender/parley/ci.yml?logo=github&label=CI)](https://github.com/linebender/parley/actions)
[![Dependency staleness status.](https://deps.rs/crate/parley_emoji/latest/status.svg)](https://deps.rs/crate/parley_emoji)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=parley_emoji
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs should be evaluated here.
See https://linebender.org/blog/doc-include/ for related discussion. -->
[_alloc]: https://doc.rust-lang.org/stable/alloc/

<!-- cargo-rdme start -->

Emoji presentation resolution for text layout, translated from Christian Hansen's [c-emoji].

Some Unicode characters and sequences can be displayed either as ordinary
text glyphs or as emoji. This crate determines the preferred presentation
of a grapheme cluster from its Unicode emoji properties, variation selectors,
and sequence structure. The implementation recognizes the emoji
sequences defined by [Unicode Technical Standard #51][UTS51].

WARNING: This crate is currently designed only for use within Parley;
if you have a use case for it, please
[reach out](https://xi.zulipchat.com/#narrow/channel/205635-parley).
This crate exists entirely because the code it adapts doesn't match
Parley's existing license, but is otherwise currently treated as an
internal implementation detail of Parley.

In Parley, this impacts font selection for these clusters.

## Features

The following crate [feature flags](https://doc.rust-lang.org/cargo/reference/features.html#dependency-features) are available:

- `std` (enabled by default): This is currently unused and is provided for forward compatibility.

Note that Parley Emoji currently requires that an allocator is available (i.e. it depends on [`alloc`][_alloc]).
Currently, Parley Emoji does not actually allocate, but the dependency is kept so starting to allocate
in the future is not a breaking change.

[c-emoji]: <https://github.com/chansen/c-emoji>
[UTS51]: <https://www.unicode.org/reports/tr51/>

<!-- cargo-rdme end -->

## Minimum supported Rust Version (MSRV)

This version of Parley Emoji has been verified to compile with **Rust 1.88** and later.

Future versions of Parley Emoji might increase the Rust version requirement.
It will not be treated as a breaking change and as such can even happen with small patch releases.

## Community

Discussion of Parley Emoji development happens in the [Linebender Zulip](https://xi.zulipchat.com/), specifically the [#parley channel](https://xi.zulipchat.com/#narrow/channel/205635-parley).
All public content can be read without logging in.

## License

Licensed under the MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>).
This crate contains code derived from [C-Emoji](https://github.com/chansen/c-emoji).
Note that this is a different license and copyright notice than the rest of the Parley repository.
Therefore, code from this crate cannot be copied into the rest of Parley, and must be used through the public API.

## Contribution

Contributions are welcome by pull request.
The [Rust code of conduct] applies.
Please feel free to add your name to the [AUTHORS] file in any substantive pull request.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you shall be licensed as above, without any additional terms or conditions.

[Rust code of conduct]: https://www.rust-lang.org/policies/code-of-conduct
[AUTHORS]: ../AUTHORS
