# Moli patches to Parley 0.11.1

Source: the `parley` 0.11.1 crates.io package, archive SHA-256
`22d2ff88bd3f7d68d1d9b09c7e6209f9a8e8c05088295140a2bcf2e9b17038c5`.
Original source and Apache-2.0 / MIT licenses are retained. Cargo cache metadata
and the standalone lockfile are not part of this vendored dependency.

## Incremental float layout

`BreakLines::last_line_metrics()` exposes the last committed line's metrics
without changing the breaker. Moli uses its advance minus trailing whitespace
to retry an unbreakable word in a wider float exclusion band. Reading only
`LineBreakData::advance` would incorrectly include hanging whitespace.

Regression coverage: `moli-layout/tests/phase4_layout_contract.rs`.

## Optional text-item allocation

`Layout::set_text_item_quantization()` configures allocation precision after
shaping and before line breaking. It is opt-in: Moli enables 1/64 CSS px
allocation for DOM inline layout, not Canvas or SVG text measurement.

- `text_advance.rs` accumulates raw advances within each caller-supplied text
  item, then rounds its allocated width upward (never below zero).
- `data.rs` uses the same allocation in intrinsic measurement; `line_break.rs`
  preserves it across break/revert checkpoints and line-item splitting.
- `line.rs` advances between allocated items without rounding glyph advances;
  `alignment.rs` keeps justified item and glyph positions consistent.
- Font fallback alone does not split an allocation item. Bidi transitions and
  inline boxes do. Shared ligatures/combining glyphs belong to the item with
  their first logical source character; reconfiguration restores the original
  shaping clusters before assigning new boundaries.
- RTL shaping metadata and logical ligature traversal are corrected even when
  quantization is disabled. Word boundaries must remain with their source
  characters instead of moving to a neighboring ligature component.

Regression coverage: `moli-layout/src/inline/tests/text_item_advance.rs` and
`text_item_advance/shared_glyphs.rs`, using checked-in fonts. Tests include raw
glyph preservation, all text partitions of shared glyphs, RTL word breaks,
intrinsic-versus-positioned width, repeated layout, justification, negative
spacing and zero-width source fragments.

Retire these patches when compatible upstream releases provide the metrics
and text-item allocation behavior. The default, unquantized path must remain
available to non-DOM callers.
