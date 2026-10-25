# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools==4.59.0"]
# ///
"""Generate original triangle glyphs that distinguish text stroke joins."""

from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen


def triangle(left: int, right: int):
    pen = TTGlyphPen(None)
    pen.moveTo((left, 0))
    pen.lineTo((right, 0))
    pen.lineTo((300, 800))
    pen.closePath()
    return pen.glyph()


builder = FontBuilder(1000, isTTF=True)
glyphs = {
    ".notdef": triangle(0, 600),
    "A": triangle(0, 600),
    "B": triangle(200, 400),
    "C": triangle(200, 400),
}
builder.setupGlyphOrder(list(glyphs))
builder.setupCharacterMap({ord(ch): ch for ch in "ABC"})
builder.setupGlyf(glyphs)
builder.setupHorizontalMetrics(
    {name: (600, {"B": 200, "C": -500}.get(name, 0)) for name in glyphs}
)
builder.setupHorizontalHeader(ascent=800, descent=-200)
builder.setupNameTable(
    {
        "familyName": "Moli Text Stroke Test",
        "styleName": "Regular",
        "uniqueFontIdentifier": "Moli Text Stroke Test 1.0",
        "fullName": "Moli Text Stroke Test",
        "psName": "MoliTextStrokeTest",
        "version": "Version 1.000",
    }
)
builder.setupOS2(
    sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200
)
builder.setupPost()
builder.setupMaxp()
builder.setupHead(flags=1, created=2_082_844_800, modified=2_082_844_800)
builder.save(
    Path(__file__).resolve().parents[1]
    / "moli-paint/tests/fixtures/moli-text-stroke.ttf"
)
