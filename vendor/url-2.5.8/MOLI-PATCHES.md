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

Path percent-encoding also follows the current URL Standard:

- Include "^" in the hierarchical path encode set, including path setters and
  path-segment mutation, without changing opaque paths, queries, or fragments.
- When parsing an opaque path, encode only the last space immediately before
  "?" or "#". Lookahead ignores ASCII tabs and newlines just as parsing does.
- Preserve that encoded path when removing a query or fragment. The obsolete
  trailing-space stripping algorithm and the V8-only query-removal workaround
  are no longer needed.

References:
<https://url.spec.whatwg.org/#path-percent-encode-set>,
<https://url.spec.whatwg.org/#cannot-be-a-base-url-path-state>,
<https://url.spec.whatwg.org/#dom-url-search>, and
<https://url.spec.whatwg.org/#dom-url-hash>.
The normative snapshot used for this update is
<https://url.spec.whatwg.org/commit-snapshots/55d6699373ba68a16ec182f34222a74ed8bc3dac/>.

The 39 file parsing/setter tests and seven hierarchical setter tests fixed by
these patches were removed from `tests/expected_failures.txt`; the one remaining
upstream expected failure is retained. The obsolete unit-test expectations of
a host/drive-letter syntax violation and opaque-path space stripping are
replaced with explicit preservation assertions.

Eleven vendored WPT records (two parsing cases and nine setter cases) had
expectations predating the current caret and opaque-space rules. Their expected
results are synchronized with the corresponding records in
<https://github.com/web-platform-tests/wpt/blob/258f285de043b79e44324228c0fd800b38d21879/url/resources/urltestdata.json>
and
<https://github.com/web-platform-tests/wpt/blob/258f285de043b79e44324228c0fd800b38d21879/url/resources/setters_tests.json>.
Their inputs, all other expected results, and the WPT driver are unchanged.

Packaging adjustment: `debug_metadata/url.natvis` is copied from the same pinned
upstream revision, and its include path is made package-local so the
`debugger_visualizer` feature can be tested from this standalone directory.

Run the upstream unit tests, WPT driver, and doctests with:

```sh
cargo test --manifest-path vendor/url-2.5.8/Cargo.toml --all-features
```

Workspace regressions live in `moli-url/src/file_url.rs`,
`moli-url/src/hierarchical_path.rs`,
`moli-url/src/path_encoding.rs`,
`moli-renderer-v8/src/script_vm/tests/url_components.rs`, and
`moli-url-policy/src/tests.rs`. These patches do not change resource access
permissions.
