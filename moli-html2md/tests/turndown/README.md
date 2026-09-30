# Turndown differential tests

Reference: [mixmark-io/turndown](https://github.com/mixmark-io/turndown/tree/aa84dfa3e2361edea8c43acbfc2b7a9363494bfd),
revision `aa84dfa3e2361edea8c43acbfc2b7a9363494bfd`, package version 7.2.4,
using `@mixmark-io/domino` 2.2.0.

`cases.json` contains all 147 HTML fixtures from upstream `test/index.html`,
plus the 50 independently written cases in `additional.json`. The upstream
fixtures retain their MIT license in `LICENSE`. Production Rust code does not
include the upstream JavaScript implementation.

## What is compared

Turndown is configured to match Moli's ATX headings, dash bullets, star emphasis,
fenced code, inline links, and horizontal rules. Per-case style-only options
(such as referenced links or tilde fences) are normalized to this configuration.
The semantic `preformattedCode` option is preserved and tested against Moli's
`preformatted_code` option. This corpus tests conversion behavior, not Turndown's
JavaScript plugin API or every output-style option.

Each case parses the source HTML through an independent test DOM implementing
`Dom`, converts it, and checks that the DOM is unchanged. Both Markdown outputs
are rendered with `pulldown-cmark`, then their HTML is compared exactly. This
accepts equivalent delimiter choices and list indentation while detecting lost
text, incorrect links, changed paragraph/list structure, and visible delimiters.
There are also exact Markdown assertions in the original and regression suites.

`expectations.json` documents 13 deliberate differences with exact Moli outputs.
These preserve bare pre blocks and avoid upstream losses of literal characters,
emphasis, whitespace, and list boundaries. They remain active assertions; no
case is skipped. A reference change that makes a difference obsolete also fails
the test. Separate converter tests cover GFM tables and strikethrough as Moli
extensions. Explicit table headers follow Turndown's GFM plugin; simple headerless
tables receive empty headings. Complex tables retain content-only HTML so merged
cells and nested tables keep their associations. Explicitly presentational tables
still expand into blocks.

## Findings tracked outside the reference corpus

The 13 exceptions above describe this corpus, not every design difference or
every correctness issue. [tracking.rs](../tracking.rs) records these open review
findings with active assertions of both Markdown and rendered HTML:

| Tests | Status | Current behavior and comparison |
| --- | --- | --- |
| `track_nested_list_spacing_through_a_transparent_wrapper`, `track_nested_list_spacing_before_a_trailing_empty_element` | Open compatibility decision | Moli keeps the outer list tight when an `ins` wraps the nested list or an empty `span` follows it. Turndown makes it loose. Both preserve the list items. |

Multiline image labels and link titles, nested sibling emphasis at paragraph
ends, and intraword mixed emphasis now have semantic regressions in
[tracking.rs](../tracking.rs). Attribute line breaks cannot introduce Markdown
blocks, and ambiguous emphasis boundaries retain their styles through inline HTML.

These tests are not ignored or expected to panic. A change to a recorded output
requires review; when fixing a bug, replace the current-output assertion with a
regression asserting the desired semantics and remove its open entry here.

The inline-code `br` separator and nested-emphasis closing-delimiter fixes have
independent semantic regressions in [inline_boundaries.rs](../inline_boundaries.rs).
Inline-code breaks follow Moli's whitespace mode (one space after collapsing),
without copying the extra spaces from Turndown's Markdown hard-break syntax.

## Regeneration

Normal Rust tests use the checked-in data and require neither Node nor network
access. To update the reference intentionally:

```sh
git clone https://github.com/mixmark-io/turndown.git /tmp/moli-turndown/upstream
git -C /tmp/moli-turndown/upstream checkout aa84dfa3e2361edea8c43acbfc2b7a9363494bfd
npm install --prefix /tmp/moli-turndown --ignore-scripts --no-audit --no-fund @mixmark-io/domino@2.2.0
node moli-html2md/tests/turndown/generate.mjs /tmp/moli-turndown/upstream
cargo test -p moli-html2md
```

The generator executes the pinned upstream source. It adapts import extensions
for Node's ESM loader and provides CommonJS `require` for Domino, without changing
conversion rules. The corpus records both the upstream revision and normalized
options. `cases.rs` provides a separate named Rust test for every fixture.
