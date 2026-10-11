# Moli patches to Parley 0.11.0

Source: https://github.com/lexmount/parley at
`a18cfc3ba6638d44ff0d0c9ce7b0b3396b42e8cf` (the current main dependency).
The upstream workspace, shaping engine, fontique and Apache-2.0 / MIT licenses
are retained. Version numbers alone do not order this fork and crates.io 0.11.1.
The older vendored 0.11.1 package is replaced, rather than used to downgrade
main's font-instance, atom, whitespace and inline alignment implementations.

## Optional text-item allocation

`Layout::set_text_item_quantization()` configures allocation after shaping and
before line breaking. DOM inline layout uses 1/64 CSS px; Canvas and SVG retain
raw horizontal measurements.

`text_advance.rs` checkpoints each item's raw tail and rounds its allocated
width upwards, floored at zero. `data.rs` uses that allocation for intrinsic
widths; `line_break.rs` uses it for fit decisions and checkpoints. Committed
line items split at caller-owned text boundaries. Atoms stay intact and shared
glyphs belong to the item containing their first logical character; source
continuations remain explicit zero-width line items. Font fallback alone does
not split the allocation group, while bidi transitions and inline boxes do.
`line.rs` advances to the allocated item boundary without changing glyph
advances; `run.rs` includes allocation and justification in reported widths.
`cluster.rs` and `run.rs` restrict source geometry to the owning text item,
including ligatures and combining characters crossing a source boundary.
Reconfiguration changes only layout policy, never the shaped source data.
Empty source text keeps Parley's synthetic metrics run unquantized, preserving
empty text-control caret geometry and inline-box allocation without indexing
an empty caller-owned text-item list.

The fork already supplies logical RTL shaping metadata, atomic ligature
traversal and break-reason-aware float retry support. The old cluster-based
patches are not layered over that engine.

Regression coverage: `moli-layout/src/inline/tests/text_item_advance.rs`,
`text_item_advance/shared_glyphs.rs` and `moli-layout/tests/phase4_layout_contract.rs`.

Retire this override when a compatible upstream provides text-item allocation.
