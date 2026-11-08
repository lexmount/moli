# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools==4.59.0"]
# ///
"""Generate tiny, distinguishable faces for inline SVG typography regressions.

The family has regular, italic, condensed, semibold and bold faces. Their M
outlines deliberately differ, so raster tests distinguish actual face selection
from normal fallback or synthetic styling.
"""
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "moli-layout" / "tests" / "fixtures"
FAMILY = "Moli SVG Variants"
OPEN_TYPE_UNIX_EPOCH = 2_082_844_800


def glyph(points=()):
    pen = TTGlyphPen(None)
    if points:
        pen.moveTo(points[0])
        for point in points[1:]:
            pen.lineTo(point)
        pen.closePath()
    return pen.glyph()


def build_face(suffix: str, style: str, width_class: int, points, advance: int, weight: int = 400):
    italic = suffix == "italic"
    bold = weight >= 700
    builder = FontBuilder(1000, isTTF=True)
    builder.setupGlyphOrder([".notdef", "space", "M"])
    builder.setupCharacterMap({0x20: "space", 0x4D: "M"})
    builder.setupGlyf({".notdef": glyph(), "space": glyph(), "M": glyph(points)})
    builder.setupHorizontalMetrics({".notdef": (700, 0), "space": (300, 0), "M": (advance, 0)})
    builder.setupHorizontalHeader(ascent=800, descent=-200, lineGap=0)
    builder.setupNameTable({
        "familyName": FAMILY,
        "styleName": style,
        "typographicFamily": FAMILY,
        "typographicSubfamily": style,
        "uniqueFontIdentifier": f"{FAMILY} {style} 1.0",
        "fullName": f"{FAMILY} {style}",
        "psName": f"MoliSVGVariants-{style}",
        "version": "Version 1.000",
        "licenseDescription": "MIT OR Apache-2.0; generated test fixture.",
    })
    builder.setupOS2(
        sTypoAscender=800, sTypoDescender=-200, sTypoLineGap=0,
        usWinAscent=800, usWinDescent=200, sxHeight=800, sCapHeight=800,
        usWeightClass=weight, usWidthClass=width_class, fsType=0,
        fsSelection=1 if italic else (1 << 5 if bold else (1 << 6 if weight == 400 else 0)),
    )
    builder.setupPost(italicAngle=-12 if italic else 0)
    builder.setupMaxp()
    builder.setupHead(created=OPEN_TYPE_UNIX_EPOCH, modified=OPEN_TYPE_UNIX_EPOCH,
                      macStyle=(2 if italic else 0) | (1 if bold else 0))
    builder.save(FIXTURES / f"moli-svg-{suffix}.ttf")


def main():
    FIXTURES.mkdir(parents=True, exist_ok=True)
    build_face("regular", "Regular", 5, [(0, 0), (600, 0), (600, 800), (0, 800)], 700)
    build_face("italic", "Italic", 5, [(0, 0), (600, 0), (600, 800)], 700)
    build_face("condensed", "Condensed", 3, [(0, 0), (300, 0), (300, 800), (0, 800)], 400)
    build_face("semibold", "SemiBold", 5, [(0, 400), (300, 0), (600, 400), (300, 800)], 700, 600)
    build_face("bold", "Bold", 5, [(0, 0), (600, 0), (600, 200), (200, 200), (200, 800), (0, 800)], 700, 700)


if __name__ == "__main__":
    main()
