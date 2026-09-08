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

Hierarchical URL parsing and mutation also keep the authority distinct from
the logical path:

- In relative special URLs, consume all authority slashes before the host.
  In non-special URLs, backslashes remain path data and only two leading
  forward slashes introduce an authority.
- Share the serialization-only "/." prefix adjustment between parsing, path
  replacement, and path-segment mutation. Keep query and fragment offsets in
  sync when the prefix is added or removed.
- Discard that prefix when adding a host, and restore it when removing a host
  from a URL whose logical path starts with "//". Host removal also clears the
  internal host kind and preserves an empty hierarchical path with a slash.
- Distinguish a missing host from an empty authority when clearing pathname,
  so the Web API adapter no longer needs its own empty-authority workaround.

References:
<https://url.spec.whatwg.org/#relative-state>,
<https://url.spec.whatwg.org/#relative-slash-state>,
<https://url.spec.whatwg.org/#special-authority-ignore-slashes-state>,
<https://url.spec.whatwg.org/#concept-url-serializer>, and
<https://url.spec.whatwg.org/#dom-url-pathname>.

The upstream WPT data is unchanged. The 39 file parsing/setter tests and seven
hierarchical setter tests fixed by these patches were removed from
`tests/expected_failures.txt`; the one remaining upstream expected failure is
retained. The obsolete unit-test expectation of a host/drive-letter syntax
violation is replaced with an explicit preservation and no-violation assertion.

Packaging adjustment: `debug_metadata/url.natvis` is copied from the same pinned
upstream revision, and its include path is made package-local so the
`debugger_visualizer` feature can be tested from this standalone directory.

Run the upstream unit tests, WPT driver, and doctests with:

```sh
cargo test --manifest-path vendor/url-2.5.8/Cargo.toml --all-features
```

Workspace regressions live in `moli-url/src/file_url.rs`,
`moli-url/src/hierarchical_path.rs`,
`moli-renderer-v8/src/script_vm/tests/url_components.rs`, and
`moli-url-policy/src/tests.rs`. Remaining opaque-path and percent-encoding
conformance issues are separate work; these patches do not change resource
access permissions.
