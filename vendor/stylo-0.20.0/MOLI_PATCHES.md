# Moli patches to Stylo 0.20.0

Source: https://github.com/ldm0/stylo at
`671d13d31b3b00e7b1e0088ba2b36a4e35c0858c`.
The upstream workspace sources and per-file/per-crate licenses are retained;
Stylo itself is MPL-2.0. Upstream CI workflows, build output and local Cargo
caches are not part of this vendored library.

## Embedder control over relative font-size inheritance

`style/device/servo.rs` adds the defaulted FontMetricsProvider hook
`preserve_font_size_keywords_in_relative_values()`. Its default is `true`,
preserving Stylo's existing behavior. Moli opts out in its shared cascade and
media-query font-metrics provider, alongside a 13px monospace default and a
16px proportional default.

`style/values/specified/font.rs` uses that hook to distinguish a directly
inherited keyword from an explicitly relative value. Direct keywords continue
to select the family's keyword table. Relative values use the numeric parent
size, represented relative to `medium` so later family changes still retain
the default-font-size dependency. Absolute lengths remain absolute.

Pure-percentage calculations retain that dependency; calculations containing
a length, including `0px`, do not. The existing typed calculation resolver is
used instead of inspecting CSS text or testing numeric zero.

Moli regression coverage:
`moli-renderer-v8/src/script_vm/tests/dom_xhr/computed_style/monospace_font_size.rs`.
It covers generic versus named/fallback families, keyword/numeric inheritance,
pure-percentage versus length calculations, and three-generation family switches.

No getter-side correction, font-width multiplier, default browser identity
change or synchronous layout policy change is part of this patch. Replace the
vendor override when a compatible upstream revision supports the embedder hook.
