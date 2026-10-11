<!-- Instructions

This changelog follows the patterns described here: <https://keepachangelog.com/en/>.

Subheadings to categorize changes are `added, changed, deprecated, removed, fixed, security`.

This changelog is updated based on pull request bodies - see `.github/pull_request_template.md`.
In general, when submitting a PR, you should generally not manually edit this file.
Every change to this file must be written by a human.

-->

# Changelog

## [Unreleased]

This release has an [MSRV] of 1.88.

### Added

#### Parlance

- `BidiLevel` to encode bidirectional text embedding levels. ([#710][] by [@tomcur][])
- `Script::is_cursive` returning whether a script is cursive. ([#728][] by [@tomcur][])  
  This can be used to decide, for example, whether to apply letter spacing.

#### Fontique

- `Collection::family_ids` to iterate over unique font family identifiers. ([#725][] by [@tomcur][])

#### Parley

- `Layout::text_len` and `Layout::alignment`, `Cursor::upstream_cluster` and `Cursor::downstream_cluster`, `PlainEditor::is_cursor_visible`, and `PlainEditorDriver::set_selection`. ([#716][] by [@DataTriny][])
  `ClusterPath::new` and the `Style::locale` field are now public as well.
  Together these make it possible to build an accessibility integration outside of Parley.

### Changed

#### Parley

- Breaking change: the `accesskit` feature, and the AccessKit integration it enabled, were removed. ([#716][] by [@DataTriny][])
  The integration now lives in the `vello_editor` example, where it is easier to evolve and can be copied and adapted by consumers.
  This removed `LayoutAccessibility`, `PlainEditor::try_accessibility`, `PlainEditorDriver::accessibility`, `PlainEditorDriver::select_from_accesskit`, `Cursor::from_access_position`, `Cursor::to_access_position`, `Selection::from_access_selection`, and `Selection::to_access_selection`.
- Breaking change: the `Glyph::style_index` field was removed. Use `Cluster::{style, style_index}` or `GlyphRun::{style, style_index}` instead. ([#661][] by [@tomcur][])
- Breaking change: `Cluster` now spans a full grapheme cluster instead of a single character. ([#715][] by [@tomcur][])  
  Shaped clusters that cross grapheme boundaries are represented using the existing `Cluster::is_ligature_start` and `Cluster::is_ligature_continuation`; note these methods previously encoded graphemes as well.
  Shaped clusters' advances are split evenly over the grapheme clusters they overlap.
  `Run::cluster_range` now returns grapheme cluster indices relative to the run's shaped run.
- Breaking change: lines with mixed inline content, like different fonts or sizes, or inline boxes, are now sized more closely to the CSS line-box model. ([#697][] by [@tomcur][])  
  The `LineMetrics::{ascent,descent,leading}` fields were removed, and `LineMetrics::block_{min,max}_coord` now describe the block-axis layout bounds of each line box.
  Glyphs may overflow these layout bounds, especially when a small line height is used.
  `LineMetrics::content_block_{min,max}_coord` provide the typographic content bounds, covering typographic ascent and descent as well as inline boxes, ignoring the specified line height.
  Note glyphs overflow these content bounds as well, for example when many combining marks are stacked.
  The union of the line-box and content bounds is close to the old `LineMetrics::block_{min,max}_coord` fields.
- `parley::editing::Cursor::{previous,next}_logical_word` now land at the previous/next logical start of a word and skip over whitespace. ([#215][] by [@tomcur][])

### Fixed

#### Parley

- Applying letter spacing now stops optional ligatures from forming. ([#731][] by [@tomcur][])

#### Fontique

- Fix compilation on 32-bit platforms without 64-bit atomics (e.g. `mipsel-unknown-linux-gnu`). ([#671][] by [@nicoburns][])
- Don't panic when fontconfig exposes no fonts. ([#717][] by [@ogoffart][])
- `Collection::load_fonts_from_paths` no longer registers duplicate faces. ([#754][] by [@ChrisJr404][])

## [0.11.0] - 2026-06-24

This release has an [MSRV] of 1.88.

### Added

#### Parley

- Line break opportunity override ([#640][] by [@taj-p][]).
  The builders now have a `set_line_break_override` method which can be used to customise where Parley detects line-breaking opportunities. It can be called with `Some(parley::CHROMIUM_LINE_BREAK_OVERRIDE)` to make line-breaking match Chromium, or with a custom instance of `AsciiLineBreakTable` created with `AsciiLineBreakTableBuilder`. If you do not call the function then you will get the existing standard ICU-defined behavior.
- `From` conversions into `StyleProperty` for `FontWidth`, `FontStyle`, `FontWeight`, `WordBreak`, `OverflowWrap`, and `TextWrapMode`, matching the existing conversions for font family, variations, features, and line height. ([#643][] by [@waywardmonkeys][])

### Changed

#### Parley

- Upgrade to `read-fonts` 0.40, `skrifa` 0.43, `harfrust` 0.10 ([#650][] by [@nicoburns][])

### Fixed

#### Parley

- Fix font selection for emoji followed by a variation selector ([#637][] by [@kane50613][])

## [0.10.0] - 2026-06-01

This release has an [MSRV] of 1.88.

### Added

#### Parley

- Add `complex-scripts` Cargo feature to enable dictionary-based line and word breaking for complex scripts (CJK, Thai, Khmer, Lao, Myanmar). When disabled, the lightweight segmenter is used (no change in default behavior) ([#621][] by [@Linktime][])
- `PlainEditor`, `Layout`, `LayoutAccessibility`, and `Generation` now implement `Debug`. ([#615][] by [@NandishwarSingh][])
  Note: The `Layout` implementation provides a compact summary by default; the alternate form (`{:#?}`) formats the full underlying data.
- Add `StyleSet::clear` method ([#614][] by [@lainon1][])
- Restore the ability to use a different alignment width than breaking width ([#618][] by [@DJMcNab][])


#### Fontique

- Fontique supports FreeBSD ([#610][] by [@fundon][])

### Fixed

#### Parley

- `StyleRunBuilder` now handles a first style run that references a non-zero style table index. ([#626][] by [@waywardmonkeys][])
- Fix color emoji matching when emoji with non printing variation selector ([#617][] by [@fundon][])

### Changed

#### Parley

- Upgrade to harfrust 0.8 ([#632][] by [@nicoburns][])

## [0.9.0] - 2026-04-21

This release has an [MSRV] of 1.88.

### Added

#### Parley

- Floating boxes and other advanced layouts. ([#421][] by [@nicoburns][])

  This comes with a few changes to the API.
  Additionally, it brings a number of additional advanced capabilities to users who are calling the `break_lines()` function to use the  `BreakLines` struct directly:

  - The `Layout::align` method no longer accepts a `width` parameter.
    Text will now automatically be aligned relative to the `max_advance` passed to `break_all_lines()`, unless that is `Infinite` (or `f32::MAX` which we treat as infinite) in which case it will be aligned relative to the width of the longest line in the layout.
    Additionally, if a line is given a (non-infinite) line-specific `line_max_advance` using the advanced `BreakLines` API, then it will aligned relative to that width instead.
  - The `min_coord` and `max_coord` fields of `LineMetrics` which record the start and end position of the line in the block/y axis are renamed to `block_min_coord` and `inline_min_coord`, and there are new `inline_min_coord` and `inline_max_coord` which record the start and end position of the line in the x/inline axis.
  - Inline boxes now have an additional `InlineBoxKind` field.
    `InFlow` behaves like inline boxes in previous versions of Parley.
    `OutOfFlow` gets positioned like an `InFlow` box, but does not take up space (like `position:absolute` in CSS), and `CustomOutOfFlow` can be used in conjunction with the low-level `.break_lines()` API to implement floated boxes (like CSS `float`) which  position depends on the state of line breaking.
  - Each line can have it's x/y/width/height set individually, and text will lay out into that box (the width for each line must be less than or equal to the width of the overall layout).
    This enables text to be laid out around "excluded regions", by setting a line box that avoids those regions.
    The logic for computing the positions of each line is left to the user.

#### Fontique

- `FallbackKey` now implements `From<Script>` for convenience. ([#594][] by [@xStrom][])

### Changed

- Updated to `hashbrown` 0.17. ([#506][] by [@waywardmonkeys][])
- Updated to `read-fonts` 0.39, `skrifa` 0.42, and `harfrust` 0.6. ([#600][] by [@nicoburns][])

### Fixed

#### Parley

- Going to the start of text when moving up on a single line. ([#609][] by [@jordanhalase][])

#### Fontique

- CJK text on macOS when PingFangUI contains hvgl outlines. ([#598][] by [@dfrg][])

## [0.8.0] - 2026-03-27

This release has an [MSRV] of 1.88.

### Added

#### Parley

- CSS `text-indent` support. ([#551][] by [@devunt][])
- `break_line_with_next` method which implements a new line-breaking algorithm based on the number of characters rather than the width of the text. ([#575][] by [@taj-p][])
- `Layout::lines` now returns an iterator that also implements `ExactSizeIterator` and `DoubleEndedIterator`. ([#554][], [#560][] by [@xStrom])
- `BoundingBox` now implements `Debug`. ([#486][] by [@taj-p][])
- Lower-level `StyleRunBuilder` to go along with the existing `RangedBuilder` and `TreeBuilder`. `StyleRunBuilder` accept non-overlapping ranges of styles. ([#516][] by [@waywardmonkeys][])
- X-height, cap-height to RunMetrics. ([#523][] by [@elbaro][])
- All possible AccessKit text properties. ([#563][] by [@mwcampbell][])
- PlainEditor getters `scale`, `font_size`, and `default_style`. ([#571][] by [@zeophlite][])

#### Fontique

- `load_system_fonts` and `load_fonts_from_paths` methods. ([#540][] by [@nicoburns][])
- Method to convert non-shared `Collection`/`SourceCache` to shared variants. ([#541][] by [@nicoburns][])

### Fixed

#### Parley

- Max-content width being reset to 0 by mandatory line breaks. ([#531][] by [@nicoburns][])
- Cursor rectangle calculation for last line. ([#547][] by [@areopagitics][])
- Treat Line Separator and Paragraph Separator as mandatory break. ([#549][] by [@xorgy][])
- Justification space count for non-space line breaks. ([#565][] by [@devunt][])
- Font fallback by computing `map_len` in `fill_cluster_in_place`. ([#566][] by [@devunt][])

#### Fontique

- Clear fallback_families cache on attribute change. ([#562][] by [@dqii][])
- Fix font family name override being ignored during registration. ([#564][] by [@devunt][])
- Reset fallback cache when fallback families are modified. ([#567][] by [@devunt][])
- Panic on Windows 7. ([#589][] by [@guiguiprim][])

### Changed

#### Parley

- AccessKit has been updated to v0.24. ([#532][] by [@nicoburns][])
- Fontations has updated: harfrust (0.5), skrifa (0.40), read-fonts (0.37). ([#510][] by [@waywardmonkeys][])
- Parley now uses `icu4x` rather than `swash` for text analysis. ([#436][] by [@conor-93][] and [@taj-p][])

- `TextStyle` now uses separate lifetimes for family vs settings.
  This lets callers keep a long-lived font-family value while borrowing per-run settings without forcing both to share a single lifetime. ([#519][] by [@waywardmonkeys][])
- Breaking change: `Glyph::y` is now in Y-down coordinate space instead of Y-up coordinate space. ([#528][] by [@valadaptive][])

  **This does not change the API surface, but *will* change the behavior of existing code!** If you're iterating over non-positioned glyphs using the `GlyphRun::glyphs` method and positioning each glyph yourself, with code like:

  ```rust
  let run_y = glyph_run.baseline();
  for glyph in glyph_run.glyphs() {
    let glyph_y = run_y - glyph.y;
    ...
  }
  ```

  You'll need to update your code to *add* `glyph.y`, instead of subtracting it:

  ```rust
  let run_y = glyph_run.baseline();
  for glyph in glyph_run.glyphs() {
    let glyph_y = run_y + glyph.y;
    ...
  }
  ```

  If you instead use the `GlyphRun::positioned_glyphs` method, your code will not need to change.

#### Fontique

- The `windows` crate was updated to 0.62. ([#518][] by [@nicoburns][])
- On macOS: Enumerate system fonts via CoreText. ([#536][] by [@raiscui][], [#568][] by [@grebmeg][])

### Removed

#### Fontique

- Fontique no longer has conversions between it's script type and that of `icu4x` and `unicode_properties`.
  If you need these then you will now need to implement the conversions yourself (see linked PR for removed implementation). ([#512][] by [@waywardmonkeys][])

## [0.7.0] - 2025-11-24

This release has an [MSRV] of 1.83.

### Highlights

[#448][] by [@taj-p][]) and ([#449][] by [@nicoburns][] collectively fix a significant performance bug that occurred when laying
out large paragraphs of text.
Previously the time to perform layout was non-linear with respect to the input size and laying out
paragraphs of text with more than ~1k characters was very slow.

The new `TextWrapMode` style implements the semantics of the [`text-wrap-mode`](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/text-wrap-mode)
CSS property and allows text-wrapping to be disabled completely for a span of text.

### Migration

Some modules have been moved:

- `parley::editor` and `parley::layout::editor` are now `parley::editing`.
- `parley::layout::cursor` is now `parley::cursor`.

Fontique no longer sets the `dlopen` feature of `yeslogic-fontconfig-sys` by default. If you wish to run Fontique on a Linux system
without fontconfig installed then you will need to enable the new `fontconfig-dlopen` feature of the `fontique` crate.
If you wish to compile Fontique on a Linux system without the `fontconfig-dlopen` enabled then you will need the fontconfig dev
package (e.g. `libfontconfig1-dev` on Ubuntu) installed.

### Added

#### Parley

- Add `TextWrapMode` style. This allow line wrapping to be disabled completely for a span of text (excluding explicit line breaks). ([#367][] by [@nicoburns][])
- Add `Cluster::from_point_exact` method for hit-testing spans of text. This is useful for implementing "hover" or "click" functionality. ([#447][] by [@nicoburns][])

### Changed

#### Parley

- Split off various modules into "editing" folder. ([#440][] by [@PoignardAzur][])
- Split contents of layout/mod.rs file. ([#444][] by [@PoignardAzur][])

#### Fontique

- Make the yeslogic-fontconfig-sys/dlopen feature optional. [#467][] by [@ogoffart][])

### Fixed

#### Parley

- Running line height calculation. ([#448][] by [@taj-p][])
- Optimise line height computation. ([#449][] by [@nicoburns][])
- Add word and letter spacing to text layout based on style properties. ([#468][] by [@dolsup][])
- Hang trailing whitespace preceding explicit newline. ([#276][] by [@wfdewith][])

#### Fontique

- Fix build on platforms without 64bit atomics. ([#451][] by [@nicoburns][])

## [0.6.0] - 2025-10-06

This release has an [MSRV] of 1.82.

### Highlights

Parley now uses [HarfRust](https://github.com/harfbuzz/harfrust) rather than [Swash](https://github.com/dfrg/swash). This means that
Parley now has production-quality shaping for all scripts and can be recommended for general usage.

### Migration

As Parley now uses it's own `parley::BoundingBox` in place of `kurbo::Rect`, you may need to convert the type if you were
previously passing one of these values into a function that expects `kurbo::Rect`.
The following function may be used to perform this conversion:

```rust
fn bounding_box_to_rect(bb: parley::BoundingBox) -> kurbo::Rect {
    kurbo::Rect::new(bb.x0, bb.y0, bb.x1, bb.y1)
}
```

### Added

#### Parley

- Shift-click support through `Selection::shift_click_extension` and `PlainEditorDriver::shift_click_extension`. ([#385][] by [@kekelp][])
- Add some benchmarks using [Tango](https://github.com/bazhenov/tango). ([#405][] by [@taj-p][])

#### Fontique

- Cache character mapping metadata for each font to improve performance of font selection. ([#413][] by [@dfrg][])
- Upgrade `icu4x` dependencies to v2.x. ([#418][] by [@nicoburns][])
- Added an `unregister_font` method to remove a font from a collection. ([#395][] by [@taj-p][])

### Changed

#### Parley

- Breaking change: `Alignment` variants have been renamed to better match CSS. `Alignment::Justified` is now `Alignment::Justify` and `Alignment::Middle` is now `Alignment::Center`. ([#389][] by [@waywardmonkeys][])
- In the `PlainEditor`, triple-click now selects paragraphs rather than words ([#381][] by [@DJMcNab][])
- Updated to `accesskit` 0.21. ([#390][] by [@mwcampbell][])
- Uses `HarfRust` for text shaping. ([#400][] by [@taj-p][])
- Parley no longer depends on `peniko` or `kurbo`. ([#414][] by [@nicoburns][]):
  - Breaking change: The use of `peniko::Font` has been replaced with `linebender_resource_handle::FontData`, as such `parley::Font` is now called `Parley::FontData`.
    Note that this is the same type as in previous releases, and so is fully backwards-compatible, just with a different name.
  - Breaking change: The use of `kurbo::Rect` has been replaced with a new `parley::BoundingBox` type.

#### Fontique

- The fontconfig backend, used to enumerate system fonts on Linux, has been rewritten to call into the system's fontconfig library instead of parsing fontconfig's configuration files itself. This should significantly improve the behavior of system fonts and generic families on Linux. ([#378][] by [@valadaptive][])
- Fontique no longer depends on `peniko`. The use of `peniko::Blob` has been replaced with `linebender_resource_handle::Blob`. This is unlikely to affect users of the crate. ([#414][] by [@nicoburns][])

### Fixed

#### Parley

- Selection extension moves the focus to the side being extended. ([#385][] by [@kekelp][])
- Ranged builder default style not respecting `scale`. ([#368][] by [@xStrom][])
- Cluster source character not correct. ([#402][] by [@taj-p][])
- Don't justify the last line of a paragraph. ([#410][] by [@taj-p][])

#### Fontique

- Font family name aliases (secondary names for font families, often in another language) not being registered. ([#380][] by [@valadaptive][])

## [0.5.0] - 2025-06-01

This release has an [MSRV] of 1.82.

### Added

#### Parley

- Editor features required by Android IME. ([#334][] by [@mwcampbell][])

### Changed

#### Parley

- Breaking change: `Layout::min_content_width`, `Layout::max_content_width`, and `Layout::content_widths` have been replaced with `Layout::calculate_content_widths`, which does not internally cache the widths. This means that `Layout` is now `Sync` again, but callers will have to cache the min and max content widths themselves. ([#353][] by [@valadaptive][])
- Breaking change: the line height style property (`StyleProperty::LineHeight` and the `line_height` field on `TextStyle`) is now a `LineHeight` enum that allows you to specify absolute, font-size-relative, and font-metrics-relative line heights.
  Previously, it was always font-size-relative. ([#362][] by [@valadaptive][])
  - The default line height was previously `LineHeight::FontSizeRelative(1.0)` if you used `RangedStyleBuilder`, or `LineHeight::FontSizeRelative(1.2)` if you used `TreeStyleBuilder`.
    It is now `LineHeight::MetricsRelative(1.0)` in both cases.
    This will affect layout if you don't specify your own line height.
- Breaking change: `{RangedBuilder, TreeBuilder}::{build_into, build}` methods now consume `self`. ([#369][] by [@dhardy][])

## [0.4.0] - 2025-05-08

This release has an [MSRV] of 1.82.

### Migration

Quantization of vertical layout metrics is now optional.
For an easy upgrade we recommend enabling it by setting `quantize` to `true` when calling [`LayoutContext::ranged_builder`](https://docs.rs/parley/0.4.0/parley/struct.LayoutContext.html#method.ranged_builder) or [`LayoutContext::tree_builder`](https://docs.rs/parley/0.4.0/parley/struct.LayoutContext.html#method.tree_builder).

### Added

#### Parley

- Option to skip quantization of vertical layout metrics for advanced rendering use cases. ([#297][] by [@valadaptive][], [#344][] by [@xStrom][])
- The `WordBreak` and `OverflowWrap` style properties for controlling line wrapping. ([#315][] by [@valadaptive][])
- `PlainEditor` methods `raw_selection` and `raw_text`. ([#316][], [#317][] by [@mwcampbell][])
- `PlainEditor::selection_geometry_with`, the equivalent of `Selection::geometry_with` method. ([#318][] by [@valadaptive][])
- `BreakLines::is_done` method to check if all the text has been placed into lines. ([#319][] by [@valadaptive][])

### Changed

#### Parley

- Breaking change: `Selection::geometry`, `Selection::geometry_with`, and `PlainEditor::selection_geometry` now include the line indices that the selection rectangles belong to. ([#318][] by [@valadaptive][])
- Updated to `accesskit` 0.19. ([#294][] by [@waywardmonkeys][], [#348][] by [@xStrom][])
- Now displaying selected newlines as whitespace in the selection highlight. ([#296][] by [@valadaptive][])
- Made `BreakReason` public. ([#300][] by [@valadaptive][])

#### Fontique

- Breaking change: `Collection::register_fonts` now takes a `Blob<u8>` instead of a `Vec<u8>`. ([#306][] by [@valadaptive][])
- Breaking change: `Collection::register_fonts` now takes an optional second parameter which allows overriding the metadata used for matching the font. ([#312][] by [@valadaptive][])

### Fixed

#### Parley

- Text editing for layouts which contain inline boxes. ([#299][] by [@valadaptive][])
- Cursor navigation in RTL text sometimes getting stuck within a line. ([#331][] by [@valadaptive][])
- Using `Layout::align` on an aligned layout without breaking lines again. ([#342][] by [@xStrom][])
- Selection box height going below ascent + descent with small line heights. ([#344][] by [@xStrom][])
- Rounding error accumulation of vertical layout metrics. ([#344][] by [@xStrom][])

#### Fontique

- Panic on macOS when running in debug mode. ([#335][] by [@NoahR02][])

## [0.3.0] - 2025-02-27

This release has an [MSRV] of 1.82.

### Added

#### Parley

- `Generation` on `PlainEditor` to help implement lazy drawing. ([#143][] by [@xorgy][])
- Support for preedit for input methods in `PlainEditor`. ([#192][] by [@DJMcNab][], [#198][] by [@tomcur][])
- `PlainEditor` method to get a cursor area for use by the platform's input method. ([#224][] by [@tomcur][])
- `Layout` methods to calculate minimum and maximum content widths. ([#259][] by [@wfdewith][])
- `PlainEditor` now implements `Clone`. ([#133][] by [@nicoburns][])
- Navigation and byte selection operations for `PlainEditor`. ([#146][] by [@xorgy][])
- AccessKit integration. ([#166][] by [@mwcampbell][])
- `first_style` method to `Cluster`. ([#264][] by [@nicoburns][])

#### Fontique

- `FontStretch`, `FontStyle`, and `FontWeight` get helper functions `from_fontconfig`. ([#212][] by [@waywardmonkeys][])
- Impl `bytemuck` traits for `GenericFamily`. ([#213][] by [@waywardmonkeys][])

### Changed

#### Parley

- Breaking change: The cursor API has been completely reworked. ([#170][] by [@dfrg][])
- Breaking change: `PlainEditor`s API is now method-based rather than enum based. ([#154][] by [@mwcampbell][])
- Breaking change: `PlainEditor`'s semantics are no longer transactional. ([#192][] by [@DJMcNab][])
- Breaking change: `Alignment::Start` and `Alignment::End` now depend on text base direction.
  `Alignment::Left` and `Alignment::Right` are introduced for text direction-independent alignment. ([#250][] by [@tomcur][])
- Breaking change: `Layout` is no longer `Sync`. ([#259][] by [@wfdewith][])
- Breaking change: `PlainEditor`'s width is now `Option<f32>` rather than `f32`. ([#137][] by [@nicoburns][])
- Breaking change: Make alignment when free space is negative configurable. ([#241][] by [@nicoburns][])
- Allow Bidi `base_level` to be determined from text. ([#245][] by [@tomcur][])

#### Fontique

- Breaking change: `Stretch`, `Style`, and `Weight` renamed to `FontWidth`, `FontStyle`, `FontWeight`. ([#211][], [#223][] by [@waywardmonkeys][])
- Depend on `read-fonts` instead of `skrifa`. ([#162][] by [@nicoburns][])

### Fixed

#### Parley

- Alignment of right-to-left text. ([#250][], [#268][] by [@tomcur][])
- Performing line breaking or aligning a layout again, after justified alignment had been applied previously. ([#271][] by [@tomcur][])
- Placement of inline boxes. ([#163][] by [@spirali][])
- Cursor position for lines that are not left-aligned. ([#169][] by [@mwcampbell][])
- `Cursor::from_point` not using the line's offset. ([#176][] by [@DJMcNab][])
- Off-by-one error in `PlainEditor::cursor_at`. ([#187][] by [@tomcur][])
- Binary search in `Layout::line_for_offset`. ([#188][] by [@tomcur][])
- Whitespace collapsing at the start of inline spans. ([#191][] by [@nicoburns][])
- Collapsing selection. ([#201][] by [@tomcur][])
- Affinities not being ignored in `Selection::is_collapsed`. ([#202][] by [@tomcur][])
- Misc. inline box layout issues. ([#207][] by [@nicoburns][])
- Linebreaking for lines without text runs. ([#249][] by [@wfdewith][])
- Calculating trailing whitespace for all lines. ([#256][] by [@wfdewith][])
- Strip whitespace following a newline when whitespace collapsing is enabled. ([#254][] by [@nicoburns][])
- Account for inline boxes when collapsing whitespace after newlines. ([#280][] by [@nicoburns][])

#### Fontique

- Skip adding font family as fallback if it has zero coverage for given script. ([#182][] by [@richardhozak][])

## [0.2.0] - 2024-10-10

This release has an [MSRV] of 1.75.

### Added

#### Parley

- Example using `tiny-skia` which renders into a PNG. ([#55][] by [@nicoburns][])
    - Breaking change: There is now a blanket implementation for `Brush`.
- A Swash example which renders into a PNG. ([#54][] by [@nicoburns][])
- An example with Vello on Winit which shows a basic text editor .([#106][] by [@dfrg][])
- `PlainEditor`, a basic action-based text editor based on Parley `Selection` and `Cursor`. ([#126][] by [@xorgy][])
- Tree style builder. ([#76][] by [@nicoburns][])
- Conversions for `FontFamily`, `FontStack`, and `StyleProperty` to make styling more ergonomic. ([#129][] by [@xorgy][])

### Changed

#### General

- Repository layout updated to match Linebender standard. ([#59][] by [@waywardmonkeys][])

#### Parley

- Emoji clusters now get an Emoji family added by default. ([#56][] by [@dfrg][])
- Style builders now accept `Into<StyleProperty<'a, B: Brush>>` so you can push a `GenericFamily` or `FontStack` directly. ([#129][] by [@xorgy][])

#### Fontique

- Removed unsafe code from fontconfig cache. ([#78][] by [@waywardmonkeys][])
- Switched to `windows-rs` for `dwrite` backend. ([#85][] by [@dfrg][])

### Fixed

#### Fontique

- Search correct paths for fonts on Apple platforms. ([#71][] by [@waywardmonkeys][])

### Removed

#### Fontique

- Breaking change: removed conversion to/from `icu_properties::Script` for `fontique::Script`. ([#72][] by [@waywardmonkeys][])
    - This can be restored by using the `icu_properties` feature of `fontique`.

## [0.1.0] - 2024-05-01

This release has an [MSRV][] of 1.70.

- Initial release

[MSRV]: README.md#minimum-supported-rust-version-msrv

[@areopagitics]: https://github.com/areopagitics
[@ChrisJr404]: https://github.com/ChrisJr404
[@conor-93]: https://github.com/conor-93
[@DataTriny]: https://github.com/DataTriny
[@devunt]: https://github.com/devunt
[@dfrg]: https://github.com/dfrg
[@dhardy]: https://github.com/dhardy
[@DJMcNab]: https://github.com/DJMcNab
[@dolsup]: https://github.com/dolsup
[@dqii]: https://github.com/dqii
[@elbaro]: https://github.com/elbaro
[@fundon]: https://github.com/fundon
[@guiguiprim]: https://github.com/guiguiprim
[@grebmeg]: https://github.com/grebmeg
[@jordanhalase]: https://github.com/jordanhalase
[@kane50613]: https://github.com/kane50613
[@kekelp]: https://github.com/kekelp
[@lainon1]: https://github.com/lainon1
[@Linktime]: https://github.com/Linktime
[@mwcampbell]: https://github.com/mwcampbell
[@NandishwarSingh]: https://github.com/NandishwarSingh
[@nicoburns]: https://github.com/nicoburns
[@NoahR02]: https://github.com/NoahR02
[@ogoffart]: https://github.com/ogoffart
[@PoignardAzur]: https://github.com/@PoignardAzur
[@raiscui]: https://github.com/raiscui
[@richardhozak]: https://github.com/richardhozak
[@spirali]: https://github.com/spirali
[@taj-p]: https://github.com/taj-p
[@tomcur]: https://github.com/tomcur
[@valadaptive]: https://github.com/valadaptive
[@waywardmonkeys]: https://github.com/waywardmonkeys
[@wfdewith]: https://github.com/wfdewith
[@xorgy]: https://github.com/xorgy
[@xStrom]: https://github.com/xStrom
[@zeophlite]: https://github.com/zeophlite

[#54]: https://github.com/linebender/parley/pull/54
[#55]: https://github.com/linebender/parley/pull/55
[#56]: https://github.com/linebender/parley/pull/56
[#59]: https://github.com/linebender/parley/pull/59
[#71]: https://github.com/linebender/parley/pull/71
[#72]: https://github.com/linebender/parley/pull/72
[#76]: https://github.com/linebender/parley/pull/76
[#78]: https://github.com/linebender/parley/pull/78
[#85]: https://github.com/linebender/parley/pull/85
[#106]: https://github.com/linebender/parley/pull/106
[#126]: https://github.com/linebender/parley/pull/126
[#129]: https://github.com/linebender/parley/pull/129
[#133]: https://github.com/linebender/parley/pull/133
[#137]: https://github.com/linebender/parley/pull/137
[#143]: https://github.com/linebender/parley/pull/143
[#146]: https://github.com/linebender/parley/pull/146
[#154]: https://github.com/linebender/parley/pull/154
[#162]: https://github.com/linebender/parley/pull/162
[#163]: https://github.com/linebender/parley/pull/163
[#166]: https://github.com/linebender/parley/pull/166
[#169]: https://github.com/linebender/parley/pull/169
[#170]: https://github.com/linebender/parley/pull/170
[#176]: https://github.com/linebender/parley/pull/176
[#182]: https://github.com/linebender/parley/pull/182
[#187]: https://github.com/linebender/parley/pull/187
[#188]: https://github.com/linebender/parley/pull/188
[#191]: https://github.com/linebender/parley/pull/191
[#192]: https://github.com/linebender/parley/pull/192
[#198]: https://github.com/linebender/parley/pull/198
[#201]: https://github.com/linebender/parley/pull/201
[#202]: https://github.com/linebender/parley/pull/202
[#207]: https://github.com/linebender/parley/pull/207
[#211]: https://github.com/linebender/parley/pull/211
[#212]: https://github.com/linebender/parley/pull/212
[#213]: https://github.com/linebender/parley/pull/213
[#215]: https://github.com/linebender/parley/pull/215
[#223]: https://github.com/linebender/parley/pull/223
[#224]: https://github.com/linebender/parley/pull/224
[#241]: https://github.com/linebender/parley/pull/241
[#245]: https://github.com/linebender/parley/pull/245
[#249]: https://github.com/linebender/parley/pull/249
[#250]: https://github.com/linebender/parley/pull/250
[#254]: https://github.com/linebender/parley/pull/254
[#256]: https://github.com/linebender/parley/pull/256
[#259]: https://github.com/linebender/parley/pull/259
[#264]: https://github.com/linebender/parley/pull/264
[#268]: https://github.com/linebender/parley/pull/268
[#271]: https://github.com/linebender/parley/pull/271
[#276]: https://github.com/linebender/parley/pull/276
[#280]: https://github.com/linebender/parley/pull/280
[#294]: https://github.com/linebender/parley/pull/294
[#296]: https://github.com/linebender/parley/pull/296
[#297]: https://github.com/linebender/parley/pull/297
[#299]: https://github.com/linebender/parley/pull/299
[#300]: https://github.com/linebender/parley/pull/300
[#306]: https://github.com/linebender/parley/pull/306
[#312]: https://github.com/linebender/parley/pull/312
[#315]: https://github.com/linebender/parley/pull/315
[#316]: https://github.com/linebender/parley/pull/316
[#317]: https://github.com/linebender/parley/pull/317
[#318]: https://github.com/linebender/parley/pull/318
[#319]: https://github.com/linebender/parley/pull/319
[#331]: https://github.com/linebender/parley/pull/331
[#334]: https://github.com/linebender/parley/pull/334
[#335]: https://github.com/linebender/parley/pull/335
[#342]: https://github.com/linebender/parley/pull/342
[#344]: https://github.com/linebender/parley/pull/344
[#348]: https://github.com/linebender/parley/pull/348
[#353]: https://github.com/linebender/parley/pull/353
[#362]: https://github.com/linebender/parley/pull/362
[#367]: https://github.com/linebender/parley/pull/367
[#368]: https://github.com/linebender/parley/pull/368
[#369]: https://github.com/linebender/parley/pull/369
[#378]: https://github.com/linebender/parley/pull/378
[#380]: https://github.com/linebender/parley/pull/380
[#381]: https://github.com/linebender/parley/pull/381
[#385]: https://github.com/linebender/parley/pull/385
[#389]: https://github.com/linebender/parley/pull/389
[#390]: https://github.com/linebender/parley/pull/390
[#395]: https://github.com/linebender/parley/pull/395
[#400]: https://github.com/linebender/parley/pull/400
[#402]: https://github.com/linebender/parley/pull/402
[#405]: https://github.com/linebender/parley/pull/405
[#410]: https://github.com/linebender/parley/pull/410
[#413]: https://github.com/linebender/parley/pull/413
[#414]: https://github.com/linebender/parley/pull/414
[#418]: https://github.com/linebender/parley/pull/418
[#421]: https://github.com/linebender/parley/pull/421
[#436]: https://github.com/linebender/parley/pull/436
[#440]: https://github.com/linebender/parley/pull/440
[#444]: https://github.com/linebender/parley/pull/444
[#447]: https://github.com/linebender/parley/pull/447
[#448]: https://github.com/linebender/parley/pull/448
[#449]: https://github.com/linebender/parley/pull/449
[#451]: https://github.com/linebender/parley/pull/451
[#467]: https://github.com/linebender/parley/pull/467
[#468]: https://github.com/linebender/parley/pull/468
[#486]: https://github.com/linebender/parley/pull/486
[#506]: https://github.com/linebender/parley/pull/506
[#510]: https://github.com/linebender/parley/pull/510
[#512]: https://github.com/linebender/parley/pull/512
[#516]: https://github.com/linebender/parley/pull/516
[#518]: https://github.com/linebender/parley/pull/518
[#519]: https://github.com/linebender/parley/pull/519
[#523]: https://github.com/linebender/parley/pull/523
[#528]: https://github.com/linebender/parley/pull/528
[#531]: https://github.com/linebender/parley/pull/531
[#532]: https://github.com/linebender/parley/pull/532
[#536]: https://github.com/linebender/parley/pull/536
[#540]: https://github.com/linebender/parley/pull/540
[#541]: https://github.com/linebender/parley/pull/541
[#547]: https://github.com/linebender/parley/pull/547
[#549]: https://github.com/linebender/parley/pull/549
[#551]: https://github.com/linebender/parley/pull/551
[#554]: https://github.com/linebender/parley/pull/554
[#560]: https://github.com/linebender/parley/pull/560
[#562]: https://github.com/linebender/parley/pull/562
[#563]: https://github.com/linebender/parley/pull/563
[#564]: https://github.com/linebender/parley/pull/564
[#565]: https://github.com/linebender/parley/pull/565
[#566]: https://github.com/linebender/parley/pull/566
[#567]: https://github.com/linebender/parley/pull/567
[#568]: https://github.com/linebender/parley/pull/568
[#571]: https://github.com/linebender/parley/pull/571
[#575]: https://github.com/linebender/parley/pull/575
[#589]: https://github.com/linebender/parley/pull/589
[#594]: https://github.com/linebender/parley/pull/594
[#598]: https://github.com/linebender/parley/pull/598
[#600]: https://github.com/linebender/parley/pull/600
[#609]: https://github.com/linebender/parley/pull/609
[#610]: https://github.com/linebender/parley/pull/610
[#614]: https://github.com/linebender/parley/pull/614
[#615]: https://github.com/linebender/parley/pull/615
[#617]: https://github.com/linebender/parley/pull/617
[#618]: https://github.com/linebender/parley/pull/618
[#621]: https://github.com/linebender/parley/pull/621
[#626]: https://github.com/linebender/parley/pull/626
[#632]: https://github.com/linebender/parley/pull/632
[#637]: https://github.com/linebender/parley/pull/637
[#640]: https://github.com/linebender/parley/pull/640
[#643]: https://github.com/linebender/parley/pull/643
[#650]: https://github.com/linebender/parley/pull/650
[#661]: https://github.com/linebender/parley/pull/661
[#671]: https://github.com/linebender/parley/pull/671
[#697]: https://github.com/linebender/parley/pull/697
[#710]: https://github.com/linebender/parley/pull/710
[#715]: https://github.com/linebender/parley/pull/715
[#716]: https://github.com/linebender/parley/pull/716
[#717]: https://github.com/linebender/parley/pull/717
[#725]: https://github.com/linebender/parley/pull/725
[#728]: https://github.com/linebender/parley/pull/728
[#731]: https://github.com/linebender/parley/pull/731
[#754]: https://github.com/linebender/parley/pull/754

[Unreleased]: https://github.com/linebender/parley/compare/v0.11.0...HEAD
[0.11.0]: https://github.com/linebender/parley/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/linebender/parley/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/linebender/parley/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/linebender/parley/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/linebender/parley/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/linebender/parley/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/linebender/parley/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/linebender/parley/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/linebender/parley/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/linebender/parley/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/linebender/parley/compare/v0.0.0...v0.1.0
