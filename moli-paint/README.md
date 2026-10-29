# Paint backends

`raster_snapshot` produces RGBA pixels through AnyRender/Vello CPU.
`pdf::pdf_snapshot` consumes the same owned snapshot and writes PDF through
upstream [Krilla at `5eb4e1f`](https://github.com/LaurenzV/krilla/commit/5eb4e1fb24a24ce78b533c23b080969917530c32).
CDP `Page.printToPDF` and CLI `--dump pdf` use the PDF backend.
CLI PDF exports include CSS backgrounds, preserving colored controls and white
text on colored backgrounds. CDP continues to use its `printBackground` option,
which defaults to false. Both paths apply print media styles unless explicitly
overridden.
The renderer owns the print request, restores media after snapshot capture, and
returns PDF bytes to both clients without publishing interactive geometry. Vector
print capture is independent of the full-document bitmap dimension limit.

The entry points and shared code are organized by responsibility:

- `moli/src/fetch_dump/pdf.rs` starts CLI printing; `screenshot.rs` handles PNG exports.
- `moli-renderer-v8/src/runtime/page_print.rs` owns PDF request/reply types and the
  print lifecycle. `page_screenshot.rs` owns screenshot/screencast types and capture.
- `moli-paint/src/validation.rs` validates shared resource references, scope balance
  and work budgets; `error.rs` defines the public paint errors.
- `moli-paint/src/geometry.rs` converts shapes and transforms; `scene/` converts
  drawing primitives into AnyRender commands for both output backends.
- `moli-paint/src/raster/` allocates and renders pixel surfaces;
  `pdf/` encodes native PDF content and invokes bounded raster fallbacks for effects
  that need them. Ordinary PDF text bypasses AnyRender to retain Unicode mappings.

The dependency uses a fixed Git revision with only `raster-images` enabled;
`simple-text` remains disabled because Parley already performs shaping. Krilla's
own `pdf-writer` Git pin (`fdf0901`) is recorded in `Cargo.lock`. This revision
requires Rust 1.92; the repository toolchain is 1.96.1. Krilla and the adapter now
share `skrifa` 0.44 with Parley, Vello and usvg. The adapter uses `skrifa::raw`
instead of declaring a separate `read-fonts` dependency. Moli's root
`[patch.crates-io]` also aligns `subsetter` with `skrifa` 0.44 and `write-fonts`
0.50, so subsetting shares `read-fonts` 0.41 and `font-types` 0.12 with the
rendering stack. The patch pins our [subsetter dependency update](https://github.com/lexmount/subsetter/commit/8d3815245835af309559c3b4a872d6a0a54913a9)
and can be removed when upstream publishes compatible fontations dependencies.
The dependency update preserves the subsetting algorithm, variable fonts and
CFF2 support. Krilla's `imagesize` 0.14 and `base64` 0.22
also coexist with usvg's 0.15 and 0.23; their version requirements do not overlap.

Synthetic weight is expressed in local CSS pixels. The pinned AnyRender CPU
adapter passes it to both fill and stroke; the Glifo patch converts it when
absorbing transforms and preparing cached outlines. Its `vello_common` source
matches the published 0.2.0 crate and is pinned alongside Glifo to share types.

The AnyRender CPU adapter exposes a cached glyph measurer backed by Glifo's
prepared-glyph ink query. Native PDF culling and filter image allocation consume
the adapter's result types, without depending on Glifo builders or caches.
The query resolves COLR layers and clip boxes, supported
bitmap strikes and outlines in renderer priority order, with variation,
hinting, synthetic weight, stroke joins and transforms applied. Bounds exclude
device sampling, filters and caller clips. Empty ink is distinct from unavailable
geometry, which retains a conservative fallback. PDF measurement reuses its
adapter-owned measurement cache; raster allocation adds a sampling guard. This replaces
the adapter's separate outline-metric, side-bearing and empty-path checks.
Native bitmap culling retains a conservative fallback because Krilla selects
and places bitmap strikes differently from the CPU renderer.

The PDF adapter embeds and subsets fonts, using Parley's existing glyph positions
and the snapshot's shared logical text/UTF-8 cluster ranges. These mappings are
collected only for print demands, keeping them out of the screenshot hot path.
Ligature ranges are joined in logical order before arranging RTL clusters visually. Synthetic italic
uses a glyph-local shear. Opaque, uniform synthetic weight keeps native PDF
fill-and-stroke text; unequal X/Y expansion or translucent fills use the expanded
contour so each axis and the fill opacity are preserved. Authored text stroke
follows that contour with its original width. Contours are prepared only after
page and cluster clipping. Moli caches contours and painted PDF graphics for the
whole export, across text runs and pages, and submits them through Krilla's existing
path/graphic APIs. Contour keys include the font instance, font size and independent
X/Y expansion; graphics also include fill or stroke paint. Placement and paint order
do not duplicate geometry. This cache is separate from prepared-ink measurement;
Krilla remains pinned to upstream without an expanded-glyph extension. The visible native
fill carries text semantics when available; path-only text retains one invisible
native copy for searching and extraction. Font instances
are cached separately by font identity and normalized variation coordinates.
The PDF backend leaves SVG glyph rendering disabled, so SVG-backed glyphs use
their available base outlines for synthetic fill and stroke. Fonts with SVG data
bypass the native synthetic-fill shortcut, whose Type3 rendering forces a fill.
Synthetic passes resolve each glyph's contour or native outline fallback independently;
neighboring glyphs do not select its drawing mode or add searchable copies.
The adapter reverses `avar` v1 and `fvar` normalization for Krilla's design-space
variation API. Non-default `avar` v2 instances currently return an explicit error.

Paths, borders, decorations, basic SVG paths, images, sRGB linear and non-repeating
radial gradients, clips, opacity and PDF blend modes use native PDF primitives.
Existing AnyRender geometry lowering is shared for non-text drawing; text bypasses
its glyph API because that API does not carry Unicode semantics. SVG scenes that
cannot be replayed natively, shadows, other gradient interpolation spaces and
filters use bounded CPU raster fallbacks. Ordinary filtered text gets one transparent
text copy for extraction, subject to page ownership, clips and opacity. Fully
transparent groups contribute no text. Curved/rotated clips use a bounded CPU
coverage probe at two samples per CSS pixel to exclude invisible clusters; the
actual glyphs remain native PDF text. Destination masks that leave some visible
ink and contain body text currently return an explicit error: their per-cluster
semantic visibility is not inferred from the result image's bounding box.
Destination-dependent operators (including `plus-lighter`) also require a
containing isolated layer. Shadow and clip-mask glyphs never add semantic copies.
When visible lettering comes only from `background-clip:text` with a transparent
foreground, it is rasterized mask artwork and is not searchable in this version.

`krilla-svg` was evaluated but not added: both its compatible release and the
pinned upstream Git revision use usvg 0.47, while Moli owns usvg 0.48 trees.
Basic SVG geometry reuses the existing AnyRender lowering; more complex SVGs
use a bounded image fallback.

Pagination retains the previous fit-to-printable-width behavior and slices the
continuous print layout. Each text cluster is assigned by the center of its visible, transformed ink
bounds after document clipping. It has one semantic owner page; ink that
crosses onto a neighboring page is outlined there. This prevents off-page runs and
replayed slices from duplicating the extracted document. Device pixel ratio does
not affect PDF page geometry. Page ranges still refer to original document pages.

This does not implement CSS fragmentation (`break-inside`, repeated table headers,
`@page`), tagged PDF or document outlines. HTML anchor links are retained as PDF
annotations, including resolved relative URLs, wrapped lines, transforms, overflow
and clip-path clipping, embedded frames and page boundaries. Same-document IDs and
legacy named anchors become PDF destinations; omitted destination pages fall back
to the original URL. Curved clip boundaries use half-CSS-pixel strips; rectangular
and transformed rectangular clips use polygon intersections. JavaScript handlers,
image maps and links inside SVG resources are not exported. PDF reading order still
follows paint order, and complex-script extraction depends on the reader's handling
of ToUnicode/ActualText. SVG text that takes a raster fallback has no separate text
mapping. This is not an accessibility or PDF/UA implementation. Krilla
uses ActualText for ambiguous/multiple-glyph clusters and Type3 fonts for some
color glyphs. A reader that ignores these can report incorrect text even when
MuPDF/Poppler extract it correctly; track upstream issues [411](https://github.com/LaurenzV/krilla/issues/411)
and [425](https://github.com/LaurenzV/krilla/issues/425). In the generated shaping
sample, PDF.js 5.6.205 reproduces incorrect extraction of ordinary `a` after the
same glyph was mapped to the `a` + combining acute cluster. MuPDF 1.28.2 and
Poppler 25.03.0 extract those strings correctly. These results were rechecked
with the pinned Git revision. This is an observed compatibility
limit, not a claim of cross-reader complex-text conformance. Apple Preview and
interactive browser selection/copy still require manual checks.

Run the PDF regressions with:

```sh
cargo nextest run -p moli-paint -E 'test(pdf::)'
cargo nextest run -p moli-protocol -E 'test(print_to_pdf) | test(domains::page::pdf::)'
```

`tests/fixtures/moli-pdf.ttf` is original MIT/Apache-2.0 test data derived from the
repository's generated Ahem face. It adds `fi`/`ffi` ligatures and a `wght` axis with
a non-linear `avar` map, combining-mark positioning, Hebrew coverage and COLR
color glyphs. Regenerate it with
`uv run scripts/generate-pdf-test-font.py` from the repository root.

Reader-level regression and artifact generation:

```sh
uv run --locked --script scripts/check-native-pdf.py --moli target/debug/moli
cd moli-cdp-smoke
uv run moli-cdp-smoke --group pdf
```

The reader script checks the real CLI with MuPDF, including mixed RTL, multi-glyph
clusters, repeated base letters, color glyphs, opacity, shadows, filters and a
200,100 CSS-pixel document. It retains PDFs, rendered pages, extracted text and
per-process timings/sampled peak RSS in `target/native-pdf-check/`. The Rust tests
use lopdf for PDF objects and ordinary text; its extractor does not interpret
ActualText and Type3 text correctly, so it is not the complex-text oracle.
The reader script also compares rendered solid and gradient button backgrounds
against CLI screenshots; extracting a white label alone does not establish that
the label is visible in the PDF. It checks the white label's visible pixels as
well as its extracted text. A local fixture server supplies relative URLs, wrapped
links, rotated and clipped links, an iframe with an SVG icon, and internal ID/named
anchors. MuPDF checks both targets and hit regions, including PDF QuadPoints and
destination page numbers. The font fixtures and page resources are bundled;
these checks do not depend on public websites or system fonts.

CI runs this script against the exact HEAD release artifact in the
`Native PDF rendering and links` job. Its locked MuPDF dependency and Python 3.12
match the local command above. Failure blocks the job; PDF files, raster previews,
screenshots, extracted text and link JSON are uploaded even when a check fails.
The CI job does not substitute PDF parsing or nonempty text for visual checks.

Validation on 2026-10-12 (Linux x86_64, Rust 1.96.1): `cargo fmt --all`,
workspace Clippy with all targets/features and warnings denied, and the full
nextest run passed (20,012 tests; 17 skipped). The Python smoke unit suite passed
43 tests. The real PDF smoke group passed with both debug and release binaries.
The visual comparison covered borders, rounded clips, an sRGB gradient, alpha
image, SVG paths, transformed text, opacity, shadow and blur. Interior gradient
samples differed by at most one 8-bit channel value from the raster screenshot.
The Git revision also passed the real CLI/MuPDF checks and CDP PDF smoke group;
its rendered visual sample was pixel-identical to the crates.io baseline.

Release sizes measured with Rust 1.96.1 on Linux x86_64, using
`cargo build -p moli --release --locked` with the repository's unchanged release
profile and default features. Packaging used `scripts/release.py` with version
1.1.16, which strips the binary and creates a gzip level-9 archive:

| Revision | Release binary bytes | Stripped binary bytes | Archive bytes |
| --- | ---: | ---: | ---: |
| Before native PDF (`d9928760`) | 156,266,960 | 107,667,480 | 43,425,106 |
| Native PDF, official Krilla Git (`7a52ca98`) | 158,167,208 | 109,363,224 | 44,157,871 |
| Same Krilla, root `subsetter` patch (`8d38152`) | 157,856,280 | 109,084,696 | 44,031,505 |

The subsetter dependency update removes the duplicate `skrifa`, `read-fonts`
and `font-types` versions. It saves 272 KiB in the stripped binary and 126,366
bytes in the archive. The stripped binary's native-PDF increase relative to
`d9928760` is now 1.35 MiB (1.32%), down from 1.62 MiB. The four CLI reader
samples below (82 pages total) produced byte-identical PDFs before and after
the dependency update; their extracted text and rendered pixels also matched.
The updated subsetter passed all 174 tests, and 14 static, variable, CFF and
CFF2 font-subset comparisons were byte-identical to the registry version.

One observed release CLI run with local data URLs and fixed fonts:

| Input | Pages | PDF bytes | Whole CLI time | Sampled process peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Font Text 123 | 1 | 3,290 | 95 ms | 53.3 MiB |
| Shaping / font variants | 1 | 9,119 | 32 ms | 52.9 MiB |
| Opacity / shadow / blur | 1 | 7,070 | 52 ms | 56.1 MiB |
| 286 rows, 200,100 CSS px | 79 | 45,215 | 144 ms | 59.3 MiB |

These include process startup, font loading, layout and encoding. RSS is sampled
from `/proc` every 5 ms, not an allocator-level peak; timings are observations,
not a performance guarantee. The long document contained no Image XObjects, and
all 286 row labels were extracted once. The effects sample contained two local
raster images, with one semantic copy of each visible text run.
