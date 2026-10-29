# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools==4.59.0"]
# ///
"""Generate an original variable font with measurable optical-size advances."""

from pathlib import Path
from tempfile import TemporaryDirectory

from fontTools.designspaceLib import AxisDescriptor, DesignSpaceDocument, SourceDescriptor
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.varLib import build


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "moli-layout/tests/fixtures/moli-opsz.ttf"
EPOCH = 2_082_844_800


def master(path: Path, advance: int) -> None:
    builder = FontBuilder(1000, isTTF=True)
    order = [".notdef", "space", "A"]
    builder.setupGlyphOrder(order)
    builder.setupCharacterMap({0x20: "space", 0x41: "A"})
    glyphs = {}
    for name in order:
        pen = TTGlyphPen(None)
        if name != "space":
            pen.moveTo((0, 0))
            pen.lineTo((advance, 0))
            pen.lineTo((advance, 800))
            pen.lineTo((0, 800))
            pen.closePath()
        glyphs[name] = pen.glyph()
    builder.setupGlyf(glyphs)
    builder.setupHorizontalMetrics({name: (advance, 0) for name in order})
    builder.setupHorizontalHeader(ascent=800, descent=-200, lineGap=0)
    builder.setupNameTable({
        "familyName": "Moli Optical Sizing Test",
        "styleName": "Regular",
        "uniqueFontIdentifier": "Moli Optical Sizing Test 1.0",
        "fullName": "Moli Optical Sizing Test",
        "psName": "MoliOpticalSizingTest",
        "version": "Version 1.000",
    })
    builder.setupOS2(
        sTypoAscender=800, sTypoDescender=-200, sTypoLineGap=0,
        usWinAscent=800, usWinDescent=200, sxHeight=800, sCapHeight=800,
        fsSelection=1 << 6,
    )
    builder.setupPost()
    builder.setupMaxp()
    builder.setupHead(created=EPOCH, modified=EPOCH)
    builder.save(path)


def main() -> None:
    with TemporaryDirectory() as directory:
        designspace = DesignSpaceDocument()
        axis = AxisDescriptor()
        axis.name, axis.tag = "Optical Size", "opsz"
        axis.minimum, axis.default, axis.maximum = 8, 16, 64
        designspace.addAxis(axis)
        for size, advance in [(8, 400), (16, 600), (64, 1200)]:
            path = Path(directory) / f"master-{size}.ttf"
            master(path, advance)
            source = SourceDescriptor()
            source.path = str(path)
            source.name = f"opsz-{size}"
            source.location = {axis.name: size}
            designspace.addSource(source)
        font, _, _ = build(designspace)
        font.recalcTimestamp = False
        font["head"].created = font["head"].modified = EPOCH
        font.save(OUTPUT)


if __name__ == "__main__":
    main()
