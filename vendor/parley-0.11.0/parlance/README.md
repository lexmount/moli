<div align="center">

# Parlance

**Fundamental text property types**

[![Latest published version.](https://img.shields.io/crates/v/parlance.svg)](https://crates.io/crates/parlance)
[![Documentation build status.](https://img.shields.io/docsrs/parlance.svg)](https://docs.rs/parlance)
[![Dependency staleness status.](https://deps.rs/crate/parlance/latest/status.svg)](https://deps.rs/crate/parlance)
[![Linebender Zulip chat.](https://img.shields.io/badge/Linebender-%23parley-blue?logo=Zulip)](https://xi.zulipchat.com/#narrow/channel/205635-parley)
[![Apache 2.0 or MIT license.](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](#license)

</div>

<!-- We use cargo-rdme to update the README with the contents of lib.rs.
To edit the following section, update it in lib.rs, then run:
cargo rdme --workspace-project=parlance --heading-base-level=0
Full documentation at https://github.com/orium/cargo-rdme -->

<!-- Intra-doc links used in lib.rs should be evaluated here.
See https://linebender.org/blog/doc-include/ for related discussion. -->
<!-- cargo-rdme start -->

Fundamental text property types.

This crate is intended as a lightweight, `no_std`-friendly vocabulary layer that can be shared
across style systems, text layout engines, and font tooling. It focuses on small, typed
representations of common “leaf” concepts (weights, widths, OpenType tags, language tags, etc).

## Features

- `std` (enabled by default): This is currently unused and is provided for forward compatibility.
- `bytemuck`: Implement traits from `bytemuck` on [`GenericFamily`], [`BidiLevel`] and
  [`NormalizedCoord`].

## Example

```rust
use parlance::{Language, Tag};

let tag = Tag::parse("wght").unwrap();
assert_eq!(tag.to_bytes(), *b"wght");

let lang = Language::parse("zh-Hans-CN").unwrap();
assert_eq!(lang.as_str(), "zh-Hans-CN");
assert_eq!(lang.language(), "zh");
assert_eq!(lang.script(), Some("Hans"));
assert_eq!(lang.region(), Some("CN"));
```

<!-- cargo-rdme end -->

## Minimum supported Rust Version (MSRV)

This version of Parlance has been verified to compile with **Rust 1.88** and later.

Future versions of Parlance might increase the Rust version requirement.
It will not be treated as a breaking change and as such can even happen with small patch releases.

## Community

Discussion of Parlance development happens in the [Linebender Zulip](https://xi.zulipchat.com/), specifically the [#parley channel](https://xi.zulipchat.com/#narrow/channel/205635-parley).
All public content can be read without logging in.

Contributions are welcome by pull request.
The [Rust code of conduct] applies.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache 2.0 license, shall be licensed as noted in the [License](#license) section, without any additional terms or conditions.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

[Rust code of conduct]: https://www.rust-lang.org/policies/code-of-conduct
