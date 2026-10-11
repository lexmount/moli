# Noto Global Fonts

These are Google's Noto Fonts, used to test Parley's support for glyphs not covered by Roboto and Arimo.
Both are licensed under the OFL 1.1:

- `NotoKufiArabic-Regular.otf`'s license is in `NotoKufiArabic-Regular-LICENSE.txt`.
- `NotoSansCJKsc-Regular-subset.otf`'s license is in `NotoSansCJKsc-Regular-subset-LICENSE.txt`.

## Noto Sans CJK Simplified Chinese Subset

For our tests, we use only a subset of Noto Sans CJK, as the full CJK font is more than 15MiB, which is impractically large for storing in-repo.
The full unsubsetted font was obtained from <https://github.com/notofonts/noto-cjk/releases/tag/Sans2.004> (specifically,
[Language Specific OTFs Simplified Chinese (简体中文)](https://github.com/notofonts/noto-cjk/releases/download/Sans2.004/08_NotoSansCJKsc.zip)).
The original font file (`NotoSansCJKsc-Regular.otf`), had sha256 hash `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`.

Currently, this subset is only a single glyph, produced using [fonttools](https://github.com/fonttools/fonttools):

```sh
fonttools subset NotoSansCJKsc-Regular.otf \
  --output-file=NotoSansCJKsc-Regular-subset.otf \
  --unicodes=U+9AA8 \
  --layout-features='*' \
  --glyph-names \
  --symbol-cmap \
  --legacy-cmap \
  --notdef-glyph --notdef-outline \
  --recommended-glyphs \
  --name-IDs='*' \
  --no-hinting \
  --desubroutinize
```

If in adding future tests you need more glyphs, then replacing the current subset is fine, so long as this doc is updated.
Note that this might change if the font file gets to be a considerable size; recondier replacing it if larger than, say, 150KiB.
