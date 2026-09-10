# Moli's usvg 0.48.1 patch set

Source: the published `usvg` 0.48.1 crate from
<https://github.com/linebender/resvg>, commit
`68b14c4c3bccdb60344c777406486b54c36ec1a4`, path `crates/usvg`.

Crate archive SHA-256:
`977d0a4abdef933f424a99fe09f95576e089b90aebc6f016a3bc813762493e91`.
The upstream MIT and Apache-2.0 licenses are retained. Source, README and
normalized/original manifests are otherwise copied from that archive.

Local changes retain layout provenance that is otherwise discarded:

- Original XML element indices on `Text` and `TextSpan`, independent of
  authored IDs. No source XML or live DOM attributes are modified.
- `Text::layouted_clusters()` exposes logical normalized byte ranges,
  typographic advance, font ascent/descent, bidi direction and final placement
  from the very same shaped clusters used to build paint glyphs.
- Advance is sampled after letter/word spacing but before textLength placement
  adjustments; glyph scaling remains in the final cluster transform.

These additions do not introduce a second text shaper or change paint output.
SVG DOM UTF-16 indexing, source-node association and immutable snapshot ownership
belong to Moli's consumer, not to an alternate layout algorithm in this patch.
