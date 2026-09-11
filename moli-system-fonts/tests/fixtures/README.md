# System font matching fixture

`moli-ahem.ttf` is the project's generated Latin test font, licensed under
MIT OR Apache-2.0. The native matching test registers it in an isolated
Fontconfig configuration so the candidate does not depend on installed fonts.

Regenerate it and the layout fixtures from the repository root with
`uv run scripts/generate-layout-test-font.py`. The script writes the same TTF
to both crates; tests in this crate do not read files from `moli-layout`.
