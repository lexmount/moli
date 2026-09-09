# Layout font fixtures

Regenerate the complete fixed-font corpus from the repository root with:

```bash
uv run scripts/generate-layout-test-font.py
```

The script pins its Python dependencies and verifies both external source-font
SHA-256 values before subsetting. A same-path but different distro font must
fail instead of silently changing Chromium/Rust geometry.

## Project-generated Latin face

`moli-ahem.ttf`, `.woff`, and `.woff2` are original project test data. They use
a deterministic 1000-unit em, 600-unit advance, 800-unit ascent, and 200-unit
descent. Every supported printable Latin character has a distinct glyph id.
These three files are licensed under the repository's MIT OR Apache-2.0 terms.

| File | SHA-256 |
|---|---|
| `moli-ahem.ttf` | `a7d23739002b2f3f20aa6d50d64b220287d9993dbd4e68aee84eba9f7b6eb336` |
| `moli-ahem.woff` | `17b7bee15b40e764d785d69412ec4fdc87715a5c107d46e31664fb6393f29f85` |
| `moli-ahem.woff2` | `c054d1e545d4c234c8c4bfea2bf903d8304fad3b20a51dab60338cea29c793d2` |

## Project-generated shared-glyph face

`moli-ligatures.ttf` is original project test data under the repository's
MIT OR Apache-2.0 terms. It has Latin `ff` / `fi` / `ffi`, combining
`a` + U+0301, and Arabic lam-alef substitutions with deliberately fractional
advances at common CSS font sizes. This makes glyph ownership and rounding
across DOM text-item boundaries testable without platform fonts.

Regenerate just this face, without external font sources, with:

```bash
uv run scripts/generate-layout-test-font.py --ligatures-only
```

SHA-256: `6d31f07720137b45db192ace541c50e3f526806fc07eeab648a2d7abd7dc01cd`.

## Subset fallback faces

| Fixture | Included code points | Source and source SHA-256 | License | Fixture SHA-256 |
|---|---|---|---|---|
| `moli-hebrew-emoji.ttf` | space, U+05D0, U+05D1, U+1F600 | Debian `fonts-dejavu-core` `DejaVuSans.ttf`, `57f73e11f51999432bf7ab22ce55b6f945d5eca1bf824404cfa9ec2e3718c84e` | Bitstream Vera license; DejaVu changes public domain | `bbc4e2f79c72e31d996ec97ed33d10ec71a3fd9ef207dccd27962c455e22a729` |
| `moli-cjk.ttf` | space, U+4E2D, U+6587 | Debian `fonts-droid-fallback` `DroidSansFallbackFull.ttf`, `acb6440a713d880a13a21b468ba7cd43f5a2b2934972e51be791c880730777b8` | Apache License 2.0 | `c6de02a7d957bac34a19921dc67c48a79c26672d0cf7a55c3fea7e75e73f138c` |
| `noto-color-emoji-cbdt-subset.ttf.b64` | U+0038, U+0039, U+00AE, U+2049, U+20E3 plus emoji shaping data | HarfBuzz `test/subset/data/fonts/NotoColorEmoji.subset.ttf`, `67c04abfbef8f3a1102d21ae8112adeb4fed7e74a05dc1e40825d81b82086ab7` | SIL Open Font License 1.1 | decoded bytes have the same SHA-256 as the source |

The subsets retain their upstream font family and PostScript metadata so the
Chromium differential can prove which face rendered each script. Their file
names are fixture labels, not embedded family names. See
`LICENSE-DejaVu.txt`, `LICENSE-Droid.txt`, `LICENSE-Apache-2.0.txt`, and
`LICENSE-OFL-1.1.txt` in this directory for the copied source notices and
license terms. The color-emoji fixture is stored as wrapped base64 only so a
text patch can carry the small upstream binary exactly; tests remove ASCII
whitespace before using it as a CSS data URL.
