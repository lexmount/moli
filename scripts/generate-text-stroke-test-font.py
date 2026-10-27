# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools==4.59.0"]
# ///
"""Generate original triangle glyphs that distinguish text stroke joins."""

from pathlib import Path

from fontTools.colorLib.builder import buildCOLR, buildCPAL
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.scaleUpem import scale_upem
from fontTools.ttLib.tables.S_V_G_ import SVGDocument


def triangle(left: int, right: int):
    pen = TTGlyphPen(None)
    pen.moveTo((left, 0))
    pen.lineTo((right, 0))
    pen.lineTo((300, 800))
    pen.closePath()
    return pen.glyph()


def line(start: tuple[int, int], end: tuple[int, int]):
    pen = TTGlyphPen(None)
    pen.moveTo(start)
    pen.lineTo(end)
    pen.closePath()
    return pen.glyph()


def rectangle():
    pen = TTGlyphPen(None)
    pen.moveTo((0, 0))
    pen.lineTo((600, 0))
    pen.lineTo((600, 800))
    pen.lineTo((0, 800))
    pen.closePath()
    return pen.glyph()


builder = FontBuilder(1000, isTTF=True)
glyphs = {
    ".notdef": triangle(0, 600),
    "A": triangle(0, 600),
    "B": triangle(200, 400),
    "C": triangle(200, 400),
    "D": line((300, 0), (300, 800)),
    "E": line((0, 400), (600, 400)),
    "space": TTGlyphPen(None).glyph(),
    "R": rectangle(),
}
builder.setupGlyphOrder(list(glyphs))
builder.setupCharacterMap({**{ord(ch): ch for ch in "ABCDER"}, ord(" "): "space"})
builder.setupGlyf(glyphs)
builder.setupHorizontalMetrics(
    {name: (600, {"B": 200, "C": -500, "D": 300}.get(name, 0)) for name in glyphs}
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
scale_upem(builder.font, 250)
builder.save(
    Path(__file__).resolve().parents[1]
    / "moli-paint/tests/fixtures/moli-text-stroke-250.ttf"
)

# Keep the original glyf outlines. Moli's PDF backend does not install an SVG
# renderer, so R must fall back to its rectangle even though it has SVG data.
fixtures = Path(__file__).resolve().parents[1] / "moli-paint/tests/fixtures"
svg_font = TTFont(fixtures / "moli-text-stroke.ttf", recalcTimestamp=False)
svg_font["SVG "] = newTable("SVG ")
glyph_id = svg_font.getGlyphID("R")
svg_font["SVG "].docList = [
    SVGDocument(
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -800 600 1000">'
        '<rect x="0" y="-800" width="600" height="800" fill="lime"/></svg>',
        glyph_id,
        glyph_id,
    )
]
svg_font.save(fixtures / "moli-text-stroke-svg.ttf")

# A half-transparent COLR rectangle makes repeated painting observable in the
# interior, independently of anti-aliasing or the base triangle's outline.
for svg in [False, True]:
    color_font = TTFont(
        fixtures / ("moli-text-stroke-svg.ttf" if svg else "moli-text-stroke.ttf"),
        recalcTimestamp=False,
    )
    color_font["COLR"] = buildCOLR(
        {"A": [("R", 0)]}, version=0, glyphMap=color_font.getReverseGlyphMap()
    )
    color_font["CPAL"] = buildCPAL([[(1.0, 0.0, 0.0, 0.5)]])
    suffix = "-svg" if svg else ""
    color_font.save(fixtures / f"moli-text-stroke-colr{suffix}.ttf")
