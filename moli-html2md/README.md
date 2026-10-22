# moli-html2md

Convert an existing DOM to Markdown through a small read-only interface. This
is an independent implementation without an HTML parser or intermediate DOM.
Unicode character classification uses `icu_properties`, with an ASCII fast path.
MathML escaping stays fused with Markdown protection and whitespace handling.
`moli-dom::native::NativeDom` implements the source-only interface; the browser
wraps it in a read-only view with computed CSS and live control values.

```rust
let root = dom.body_node_id().unwrap_or(dom.document_node_id());
let markdown = moli_html2md::Converter::new(moli_html2md::Options {
    max_depth: 1000,
    default_code_language: Some("text".to_owned()),
    ..Default::default()
})
.convert(&dom, root);
```

## DOM interface

Implement `Dom` with a copyable node ID, four required borrowed queries and four
optional hooks:

- `node_kind`: document/fragment, lowercase element tag, decoded text, or ignored node.
- `first_child` and `next_sibling`: ordered traversal of a stable, acyclic tree.
- `attribute`: decoded attribute value, borrowed from the DOM.
- Optional hooks: `control_value`, `has_block_layout`, `has_text_boundary`,
  and `may_have_fragment_links`.

Browser adapters can provide `control_value` for current input and textarea values,
including an empty string after a user clears it. Without that optional query,
input attributes and textarea child text supply defaults; a nonstandard textarea
`value` attribute does not override its child text.

The converter does not need parent pointers, mutable access, serialization, or
an allocation for each node. The caller chooses the root; its siblings are not
visited. `Other` nodes and their subtrees are ignored. Unknown elements expose
their children as ordinary content. Comments and doctypes produce no output.

## State machine

- A heap task stack tracks node entry, sibling continuation, and element exit.
  Traversal stores one continuation per level instead of collecting all children.
- An inline writer tracks desired and emitted styles, pending whitespace, and
  block boundaries. It collapses HTML whitespace across node boundaries, keeps
  Unicode spaces, and coalesces adjacent identical emphasis without changing nodes.
- Lists, quotes, headings, and table cells have output buffers finalized by exit
  tasks. Raw code text uses the same iterative traversal with formatting disabled.
- Tables inspect their direct sections, rows and cells before conversion. Nested
  content uses the task stack; structural pre-scans stop at nested table boundaries.
- Finished blocks and table cells form an owned fragment tree. Fallback joins
  and writes to parent buffers move fragment roots without copying descendant
  text; the final output is flattened in one iterative pass. Fragment destruction
  is iterative too. Quotes, headings and list items still materialize their
  captured text when rewriting its lines.
- List spacing follows block boundaries recorded during conversion. Nested
  lists keep their own spacing; list items need no preliminary DOM scan.
- Ordered lists use `start` followed by sequential numbers, like Turndown core.
  `reversed` and individual `li[value]` attributes are not represented.
- Code fences are chosen after collecting the code text. Language names retain
  punctuation and Unicode; a name containing backticks uses a tilde fence.
  Fallback language hints end at the first line ending.

Simple tables produce GFM pipe tables by default. A first row inside `thead` or
consisting entirely of `th` cells is used as the heading, following the heading
recognition used by Turndown's GFM tables plugin. Without a heading, the converter
adds an empty heading as wide as the widest data row and retains every data row.
Whitespace, comments and empty spacer rows do not affect recognition. The first
cell in each column supplies its `align` attribute; a preceding caption remains
a separate Markdown block.

Cells retain inline formatting, links, images and code. Literal pipes are escaped,
and cell line breaks and paragraph boundaries use inline `<br>` tags. Image labels
and link/image titles collapse HTML whitespace to ordinary spaces in every
context, keeping links on one line. Unicode spaces and literal entity text are
retained; URLs use destination escaping. GFM supplies
missing trailing cells in short rows; the converter does not allocate a padded grid.

Tables with `role="presentation"` or `role="none"` retain ordinary block
expansion, in DOM order with blank lines between cells, even when structurally
simple. Role matching ignores ASCII case and surrounding HTML whitespace. Nested
layout wrappers expand into readable blocks. Tables with header cells, captions,
explicit data roles, header relationships or real cell spans retain their
authored structure. These tables use content-only HTML for non-unit spans,
multiple `thead` rows, rows wider than an explicit header, or cells containing
nested tables, lists, headings, quotes or preformatted blocks. This retains
header, cell and nested-table associations that Markdown pipe tables cannot
express. Inner simple tables can still produce GFM when a presentational or
layout-only containing table is expanded. Scripts, styles and event handlers are
omitted from HTML fallback output.

Supported output includes headings, paragraphs, emphasis, strikethrough, links,
images, lists, blockquotes, hard breaks, media source links, separated form choices,
readable form values, and fenced code. Inline HTML is used when
Markdown delimiters cannot express an emphasis boundary.
Superscripts and subscripts retain their HTML markers. Complete author-supplied
TeX spans retain their commands and indices. Native MathML retains mathematical
markup and presentation attributes, with alternate annotations omitted, so
fractions, roots, matrices and powers do not collapse into ambiguous plain text.
Script/style/head/noscript/template subtrees are omitted.

`max_depth` counts the supplied root as depth zero; nodes at or beyond the limit
are omitted. All traversal paths, including raw code, use this limit.

`preformatted_code` preserves whitespace inside inline code when enabled. Its
default is `false`, which collapses ordinary HTML whitespace and moves boundary
spaces outside code delimiters. Fenced blocks always preserve nonempty code's
spaces and blank lines. Empty blocks still separate surrounding text.

## Content policy

This is a structural content dump. It omits non-rendered serialized state blobs
but retains content in collapsed panels and inactive tabs. It does not evaluate
stylesheets, layout, or JavaScript. Callers may supply computed block boundaries
through the optional boundary hooks.

Moli's browser export uses current CSS visibility. It retains explicit
disclosure targets, content tabpanels, searchable hidden content and collapsed
lines inside visible preformatted source. Hidden navigation, menus, dialogs and
popovers do not become disclosure content. An inherited root visibility veil
does not discard the document; explicitly hidden descendants remain excluded.
Hidden table cells retain their positions, while wholly invisible tables and
lists and collapsed rows are omitted. Computed boxes supplement document
structure without changing a tight list into a loose one merely because each
item contains a single block box.

The converter cannot preserve all HTML presentation (for example, arbitrary
ordered-list numbering). DOM snapshots used by Moli's optional strip/base/frame
transformations remain the responsibility of the caller.

## Verification

`cargo test -p moli-html2md` covers an independent arena DOM, immutability,
whitespace, escaping, code, tables, depth limits, and 20,000 nested nodes on a
64 KiB thread stack. The [Turndown differential suite](tests/turndown/README.md)
pins 147 upstream fixtures and 50 additional cases. It compares rendered
meaning, checks DOM immutability, and explicitly asserts 16 documented
differences that preserve Moli's handling of literal text, code, emphasis, and
list boundaries. Targeted regressions also cover empty elements, links,
attributes in headings and tables, code whitespace and language names, list
numbering, and loose lists. An operation-count regression checks nested table
fallback at 64, 128 and 256 levels with 256 text bytes per level, both with and
without headers. It bounds bytes copied during fragment materialization by
output size; completed blocks move into parent buffers without copying their
text. A repeated flatten/copy regression fails without timing thresholds.
Dropping 20,000 nested fragments is also tested on a 64 KiB thread stack.
Emphasis tests cover adjacent links that can use Markdown delimiters and
intraword link boundaries that still require inline HTML. Combining marks and
format characters retain Markdown emphasis; Unicode punctuation and symbols
keep their required HTML fallback. MathML tests check attribute encoding,
literal entity text, Markdown punctuation, Unicode and line normalization.

The HTML parser, reference JSON reader, and `pulldown-cmark` renderer are all
**test-only** dependencies. Tests use checked-in reference outputs and require
neither Node nor network access. NativeDom integration and real HTML fixtures
are tested in `moli-renderer-v8`.

Unavailable images retain their text alternatives. Media references use standard
source declarations before lazy-loading fallbacks. Password values and hidden or
file input values are omitted.

Plain DOM attributes describe the supplied snapshot, including default form
values and checked state. Browser adapters can expose live attributes and
`control_value` instead. Placeholder hints are not field values. Empty
text-entry controls emit no value; button-like controls may use accessible
labels. Generic containers do not acquire text from tooltip titles or prohibited
names. Child output takes precedence over any fallback label. Links retain
application protocols while executable JavaScript and inline HTML document URLs
are excluded.

SVG title children are tooltip metadata, not body text. They supply a name only
when an SVG has no emitted content; title elements outside SVG are omitted.
Code omits embedded copy controls and scripts.

Backslash-delimited inline TeX becomes inline dollar math. Invisible separators
keep adjacent formulas distinct across empty wrappers. They also separate a
closing `$` from an immediately following digit, which some math readers require.
Image alternatives keep literal dollars escaped for math-aware readers. TeX
payloads that cannot fit complete Markdown math delimiters fall back to escaped
text, including unbalanced raw braces inside `\verb`. Comment removal preserves
text adjacency and control-word boundaries. Superscript/subscript boundaries use
HTML emphasis when nested Markdown markers would pair across those tags.
MathML deduplication retains links, formatting and surrounding whitespace,
including Unicode spaces; filtered containers keep their grouping, and `maction`
keeps its first element.

Linked preformatted source retains HTML when needed to keep link and line-anchor
positions. Its language marker and physical newlines are preserved; a standalone
`pre` HTML block accepts blank lines. Complex HTML tables use physical row and
cell boundaries without blank lines. Plain heading wrappers remain Markdown
headings. Captured headings, list items and quotations contain their own fragment
targets rather than appending them to the preceding block.
