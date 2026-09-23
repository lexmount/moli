# moli-html2md

Convert an existing DOM to Markdown through a small read-only interface. This
is an independent implementation with no runtime dependencies, HTML parser, or
intermediate DOM. `moli-dom::native::NativeDom` implements the interface directly.

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

Implement `Dom` with a copyable node ID and four borrowed queries:

- `node_kind`: document/fragment, lowercase element tag, decoded text, or ignored node.
- `first_child` and `next_sibling`: ordered traversal of a stable, acyclic tree.
- `attribute`: decoded attribute value, borrowed from the DOM.

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
  content uses the task stack; eligibility checks never rescan cell descendants.
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
and cell line breaks and paragraph boundaries use inline `<br>` tags. Attribute
newlines are encoded or normalized so they cannot break a table row. GFM supplies
missing trailing cells in short rows; the converter does not allocate a padded grid.

Tables with `role="presentation"` or `role="none"` retain ordinary block expansion,
in DOM order with blank lines between cells, even when structurally simple. Role
matching ignores ASCII case and surrounding HTML whitespace. The same fallback
applies to complex tables: non-unit spans, multiple `thead` rows, rows wider than
an explicit header, or cells containing nested tables, lists, headings, quotes
or preformatted blocks. Inner simple tables can still produce GFM when their
containing layout table is expanded. Other roles, borders and CSS do not select
another conversion path. Complex layouts remain readable blocks instead of raw
HTML tables.

Supported output includes headings, paragraphs, emphasis, strikethrough, links,
images, lists, blockquotes, hard breaks, and fenced code. Inline HTML is used when
Markdown delimiters cannot express an emphasis boundary.
Script/style/head/noscript/template subtrees are omitted.

`max_depth` counts the supplied root as depth zero; nodes at or beyond the limit
are omitted. All traversal paths, including raw code, use this limit.

`preformatted_code` preserves whitespace inside inline code when enabled. Its
default is `false`, which collapses ordinary HTML whitespace and moves boundary
spaces outside code delimiters. Fenced blocks always preserve nonempty code's
spaces and blank lines. Empty blocks still separate surrounding text.

This is a structural content dump. It omits non-rendered serialized state blobs
but retains content in collapsed panels and inactive tabs. It does not evaluate
stylesheets, layout, or JavaScript, and cannot preserve all HTML presentation (for example, arbitrary
ordered-list numbering or table spanning). DOM snapshots used by Moli's optional
strip/base/frame transformations remain the responsibility of the caller.

## Verification

`cargo test -p moli-html2md` covers an independent arena DOM, immutability,
whitespace, escaping, code, tables, depth limits, and 20,000 nested nodes on a
64 KiB thread stack. The [Turndown differential suite](tests/turndown/README.md)
pins 147 upstream fixtures and 50 additional cases. It compares rendered meaning,
checks DOM immutability, and explicitly asserts 12 documented differences that
preserve Moli's handling of literal text, code, emphasis, and list boundaries.
Targeted regressions also cover empty elements, links, attributes in headings and
tables, code whitespace and language names, list numbering, and loose lists.
An operation-count regression checks nested table fallback at 64, 128 and 256
levels with 256 text bytes per level, both with and without headers. It bounds
bytes copied during fragment materialization by output size; completed blocks
move into parent buffers without copying their text. A repeated flatten/copy
regression fails without timing thresholds.
Dropping 20,000 nested fragments is also tested on a 64 KiB thread stack.
Emphasis tests cover adjacent links that can use Markdown delimiters and
intraword link boundaries that still require inline HTML.

The HTML parser, reference JSON reader, and `pulldown-cmark` renderer are all
**test-only** dependencies. Tests use checked-in reference outputs and require
neither Node nor network access. NativeDom integration and real HTML fixtures
are tested in `moli-renderer-v8`.
