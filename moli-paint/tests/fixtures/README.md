# Paint font fixtures

`moli-text-stroke.ttf` is original project test data licensed under MIT OR
Apache-2.0. Regenerate it with `uv run scripts/generate-text-stroke-test-font.py`.
Its triangle glyphs have miter ratios below and above 4, distinguishing miter
joins from round joins and checking the bevel fallback at the miter limit.

`moli-pdf.ttf` is derived from the project's generated Ahem font by
`uv run scripts/generate-pdf-test-font.py`, under the same license.
