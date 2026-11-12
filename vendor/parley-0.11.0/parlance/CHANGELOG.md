<!-- Instructions

This changelog follows the patterns described here: <https://keepachangelog.com/en/>.

Subheadings to categorize changes are `added, changed, deprecated, removed, fixed, security`.

-->

# Changelog

## [Unreleased]

This release has an [MSRV] of 1.88.

### Changed

- `Script::from_str_unchecked` has been deprecated, as `from_bytes` is just as ergonomic. ([#677][] by [@DJMcNab][])

## [0.1.0] - 2026-03-27

This release has an [MSRV] of 1.88.

### Added

This is lightweight crate that contains types representing text-related concepts such as `FontWeight`, `FontFamily`, `Script`, `Language`, etc.
It is developed as part of the Parley project but is intended to be widely useful for any Rust project working with text.
([#500][], [#501][], [#502][], [#505][], [#508][] by [@waywardmonkeys][])

[MSRV]: README.md#minimum-supported-rust-version-msrv

[@waywardmonkeys]: https://github.com/waywardmonkeys
[@DJMcNab]: https://github.com/DJMcNab

[#500]: https://github.com/linebender/parley/pull/500
[#501]: https://github.com/linebender/parley/pull/501
[#502]: https://github.com/linebender/parley/pull/502
[#505]: https://github.com/linebender/parley/pull/505
[#508]: https://github.com/linebender/parley/pull/508
[#677]: https://github.com/linebender/parley/pull/508

[Unreleased]: https://github.com/linebender/parley/compare/parlance-v0.1.0...HEAD
[0.1.0]: https://github.com/linebender/parley/compare/parlance-v0.0.0...parlance-v0.1.0
