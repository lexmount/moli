# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "brotli==1.1.0",
#   "fonttools==4.59.0",
# ]
# ///
"""Generate the deterministic fixed fonts used by Phase 3 layout tests."""

import hashlib
import os
import tempfile
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.subset import Options, Subsetter
from fontTools.ttLib import TTFont
from fontTools.designspaceLib import AxisDescriptor, DesignSpaceDocument, SourceDescriptor
from fontTools.varLib import build as build_variable_font


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "moli-layout" / "tests" / "fixtures"
TTF_PATH = FIXTURES / "moli-ahem.ttf"
HEBREW_EMOJI_PATH = FIXTURES / "moli-hebrew-emoji.ttf"
CJK_PATH = FIXTURES / "moli-cjk.ttf"
DEJAVU_SOURCE = Path(
    os.environ.get(
        "MOLI_DEJAVU_FONT_SOURCE",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    )
)
DROID_CJK_SOURCE = Path(
    os.environ.get(
        "MOLI_DROID_CJK_FONT_SOURCE",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    )
)
DEJAVU_SOURCE_SHA256 = "57f73e11f51999432bf7ab22ce55b6f945d5eca1bf824404cfa9ec2e3718c84e"
DROID_CJK_SOURCE_SHA256 = "acb6440a713d880a13a21b468ba7cd43f5a2b2934972e51be791c880730777b8"
# OpenType stores seconds since 1904-01-01. This value is Unix epoch and avoids
# FontTools treating a deliberately deterministic zero as a Unix timestamp.
OPEN_TYPE_UNIX_EPOCH = 2_082_844_800


def empty_glyph():
    pen = TTGlyphPen(None)
    return pen.glyph()


def box_glyph():
    pen = TTGlyphPen(None)
    pen.moveTo((0, 0))
    pen.lineTo((600, 0))
    pen.lineTo((600, 800))
    pen.lineTo((0, 800))
    pen.closePath()
    return pen.glyph()


def build_ttf() -> None:
    builder = FontBuilder(1000, isTTF=True)
    codepoints = list(range(0x21, 0x7F))
    codepoints += [0x00A0, 0x00DF]
    glyph_names = {
        codepoint: (
            f"uni{codepoint:04X}" if codepoint <= 0xFFFF else f"u{codepoint:05X}"
        )
        for codepoint in codepoints
    }
    glyph_order = [".notdef", "space", *glyph_names.values()]
    builder.setupGlyphOrder(glyph_order)

    # Keep glyph identities distinct so source/glyph assertions can distinguish
    # transformed characters without depending on a platform font.
    cmap = dict(glyph_names)
    cmap[0x20] = "space"
    builder.setupCharacterMap(cmap)

    glyphs = {name: box_glyph() for name in glyph_names.values()}
    glyphs.update({".notdef": box_glyph(), "space": empty_glyph()})
    builder.setupGlyf(glyphs)
    builder.setupHorizontalMetrics({glyph: (600, 0) for glyph in glyph_order})
    builder.setupHorizontalHeader(ascent=800, descent=-200, lineGap=0)
    builder.setupNameTable(
        {
            "familyName": "Moli Ahem",
            "styleName": "Regular",
            "uniqueFontIdentifier": "Moli Ahem Regular 1.0",
            "fullName": "Moli Ahem Regular",
            "psName": "MoliAhem-Regular",
            "version": "Version 1.000",
        }
    )
    builder.setupOS2(
        sTypoAscender=800,
        sTypoDescender=-200,
        sTypoLineGap=0,
        usWinAscent=800,
        usWinDescent=200,
        sxHeight=800,
        sCapHeight=800,
        fsSelection=1 << 6,
    )
    builder.setupPost()
    builder.setupMaxp()
    builder.setupHead(created=OPEN_TYPE_UNIX_EPOCH, modified=OPEN_TYPE_UNIX_EPOCH)
    builder.save(TTF_PATH)


def verify_source(source: Path, expected_sha256: str) -> None:
    if not source.is_file():
        raise SystemExit(f"required fixed-font source does not exist: {source}")
    actual_sha256 = hashlib.sha256(source.read_bytes()).hexdigest()
    if actual_sha256 != expected_sha256:
        raise SystemExit(
            f"fixed-font source hash mismatch for {source}: "
            f"expected {expected_sha256}, got {actual_sha256}"
        )


def subset_font(source: Path, output: Path, codepoints: list[int]) -> None:
    font = TTFont(source, recalcTimestamp=False)
    options = Options()
    options.recalc_timestamp = False
    options.retain_gids = False
    options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
    subsetter = Subsetter(options=options)
    subsetter.populate(unicodes=codepoints)
    subsetter.subset(font)
    font["head"].created = OPEN_TYPE_UNIX_EPOCH
    font["head"].modified = OPEN_TYPE_UNIX_EPOCH
    font.save(output, reorderTables=False)


def convert(flavor: str) -> None:
    font = TTFont(TTF_PATH, recalcTimestamp=False)
    font.flavor = flavor
    font.save(FIXTURES / f"moli-ahem.{flavor}", reorderTables=False)


def build_metrics_variable_font() -> None:
    """Independent h/v advances and MVAR heights expose font-unit mistakes."""
    design = DesignSpaceDocument()
    axis = AxisDescriptor()
    axis.name, axis.tag = "Weight", "wght"
    axis.minimum, axis.default, axis.maximum = 100, 400, 700
    design.addAxis(axis)
    with tempfile.TemporaryDirectory(prefix="moli-metrics-masters-") as temporary:
        for weight, zero, water, vertical, x_height, cap_height in [
            (100, 0, 800, 400, 300, 550),
            (400, 500, 1000, 700, 400, 650),
            (700, 750, 1200, 900, 500, 750),
        ]:
            builder = FontBuilder(1000, isTTF=True)
            glyph_order = [".notdef", "space", "zero", "water", "x", "H"]
            builder.setupGlyphOrder(glyph_order)
            builder.setupCharacterMap({32: "space", 48: "zero", 0x6C34: "water", 120: "x", 72: "H"})
            glyphs = {}
            for glyph in glyph_order:
                pen = TTGlyphPen(None)
                if glyph != "space":
                    height = x_height if glyph == "x" else cap_height
                    pen.moveTo((0, 0))
                    pen.lineTo((400, 0))
                    pen.lineTo((400, height))
                    pen.lineTo((0, height))
                    pen.closePath()
                glyphs[glyph] = pen.glyph()
            builder.setupGlyf(glyphs)
            builder.setupHorizontalMetrics({
                glyph: (zero if glyph == "zero" else water if glyph == "water" else 500, 0)
                for glyph in glyph_order
            })
            builder.setupVerticalMetrics({
                glyph: (vertical + 500 if glyph == "water" else vertical, 100)
                for glyph in glyph_order
            })
            builder.setupHorizontalHeader(ascent=850, descent=-150, lineGap=0)
            builder.setupVerticalHeader(ascent=500, descent=-500, lineGap=0)
            builder.setupNameTable({
                "familyName": "Moli Metrics Variable",
                "styleName": f"Weight{weight}",
                "uniqueFontIdentifier": f"Moli Metrics Variable {weight} 1.0",
                "fullName": f"Moli Metrics Variable {weight}",
                "psName": f"MoliMetricsVariable-{weight}",
            })
            builder.setupOS2(sTypoAscender=850, sTypoDescender=-150,
                             usWinAscent=850, usWinDescent=150, sxHeight=x_height,
                             sCapHeight=cap_height, usWeightClass=weight)
            builder.setupPost()
            builder.setupMaxp()
            builder.setupHead(created=OPEN_TYPE_UNIX_EPOCH, modified=OPEN_TYPE_UNIX_EPOCH)
            master = Path(temporary) / f"{weight}.ttf"
            builder.save(master)
            source = SourceDescriptor()
            source.path = str(master)
            source.name = f"weight-{weight}"
            source.location = {"Weight": weight}
            design.addSource(source)
        font, _, _ = build_variable_font(design)
        font.recalcTimestamp = False
        font["head"].created = font["head"].modified = OPEN_TYPE_UNIX_EPOCH
        font.save(FIXTURES / "moli-metrics-variable.ttf")


def main() -> None:
    FIXTURES.mkdir(parents=True, exist_ok=True)
    verify_source(DEJAVU_SOURCE, DEJAVU_SOURCE_SHA256)
    verify_source(DROID_CJK_SOURCE, DROID_CJK_SOURCE_SHA256)
    build_ttf()
    convert("woff")
    convert("woff2")
    build_metrics_variable_font()
    subset_font(DEJAVU_SOURCE, HEBREW_EMOJI_PATH, [0x20, 0x05D0, 0x05D1, 0x1F600])
    subset_font(DROID_CJK_SOURCE, CJK_PATH, [0x20, 0x4E2D, 0x6587])


if __name__ == "__main__":
    main()
