# Moli URL parser patch

This directory vendors the crates.io `url` 2.5.8 source release, corresponding
to upstream commit `d6ea13c5f8e7e6e627f6390161b3e185bda5e5ce` in
<https://github.com/servo/rust-url>. The original MIT and Apache-2.0 licenses,
source, unit tests, and WPT data are retained. The root Cargo patch applies the
same parser to all consumers, including URL objects, DOM attributes, navigation,
and resource URL policy.

The functional patch follows the current WHATWG file URL algorithms:

- Preserve leading empty path segments, including paths through dot segments.
- Inherit file hosts independently of Windows drive-letter handling, and keep
  explicit hosts when parsing or replacing a drive-letter path.
- Normalize drive letters only in the first path segment. Protect only a file
  URL's root drive when shortening paths.
- Distinguish entering path-start state from reprocessing a drive letter found
  in file-host state; an empty host does not mean that a drive letter was found.
- Remove the obsolete parser side channel that erased a file host while parsing
  a drive letter.

References:
<https://url.spec.whatwg.org/#file-state>,
<https://url.spec.whatwg.org/#file-slash-state>,
<https://url.spec.whatwg.org/#file-host-state>,
<https://url.spec.whatwg.org/#path-state>, and
<https://url.spec.whatwg.org/#shorten-a-urls-path>.

The upstream WPT data is unchanged. The 39 file parsing/setter tests fixed by
this patch were removed from `tests/expected_failures.txt`; the eight remaining
upstream expected failures are retained. The obsolete unit-test expectation of
a host/drive-letter syntax violation is replaced with an explicit preservation
and no-violation assertion.

Packaging adjustment: `debug_metadata/url.natvis` is copied from the same pinned
upstream revision, and its include path is made package-local so the
`debugger_visualizer` feature can be tested from this standalone directory.

Run the upstream unit tests, WPT driver, and doctests with:

```sh
cargo test --manifest-path vendor/url-2.5.8/Cargo.toml --all-features
```

Workspace regressions live in `moli-url/src/file_url.rs`,
`moli-renderer-v8/src/script_vm/tests/url_components.rs`, and
`moli-url-policy/src/tests.rs`. Non-file parser and setter conformance issues
remain separate work; this patch does not change resource access permissions.
