# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "brotli==1.1.0",
#   "fonttools==4.59.0",
# ]
# ///
"""Generate deterministic fonts for layout and system font matching tests."""

import argparse
import hashlib
import os
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.subset import Options, Subsetter
from fontTools.ttLib import TTFont


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "moli-layout" / "tests" / "fixtures"
TTF_PATH = FIXTURES / "moli-ahem.ttf"
SYSTEM_FONT_TTF_PATH = ROOT / "moli-system-fonts" / "tests" / "fixtures" / "moli-ahem.ttf"
HEBREW_EMOJI_PATH = FIXTURES / "moli-hebrew-emoji.ttf"
CJK_PATH = FIXTURES / "moli-cjk.ttf"
LIGATURE_PATH = FIXTURES / "moli-ligatures.ttf"
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


def build_ligature_ttf() -> None:
    """Small original font with known Latin, combining, and RTL substitutions."""
    widths = {
        ".notdef": 600, "space": 300, "f": 600, "i": 400,
        "a": 600, "acutecomb": 0, "x": 600, "y": 600,
        "lam": 550, "alef": 350, "f_f": 1177, "f_i": 887,
        "f_f_i": 1389, "aacute": 617, "lam_alef": 863,
    }
    builder = FontBuilder(1000, isTTF=True)
    builder.setupGlyphOrder(list(widths))
    builder.setupCharacterMap({
        0x20: "space", ord("f"): "f", ord("i"): "i", ord("a"): "a",
        ord("x"): "x", ord("y"): "y", 0x0301: "acutecomb",
        0x0644: "lam", 0x0627: "alef",
    })
    builder.setupGlyf({
        name: empty_glyph() if name in {"space", "acutecomb"} else box_glyph()
        for name in widths
    })
    builder.setupHorizontalMetrics({name: (width, 0) for name, width in widths.items()})
    builder.setupHorizontalHeader(ascent=800, descent=-200, lineGap=0)
    builder.setupNameTable({
        "familyName": "Moli Ligatures", "styleName": "Regular",
        "uniqueFontIdentifier": "Moli Ligatures Regular 1.0",
        "fullName": "Moli Ligatures Regular", "psName": "MoliLigatures-Regular",
        "version": "Version 1.000",
    })
    builder.setupOS2(sTypoAscender=800, sTypoDescender=-200, sTypoLineGap=0,
                    usWinAscent=800, usWinDescent=200, sxHeight=800,
                    sCapHeight=800, fsSelection=1 << 6)
    builder.setupPost()
    builder.setupMaxp()
    builder.setupHead(created=OPEN_TYPE_UNIX_EPOCH, modified=OPEN_TYPE_UNIX_EPOCH)
    addOpenTypeFeaturesFromString(builder.font, """
        languagesystem DFLT dflt;
        languagesystem latn dflt;
        languagesystem arab dflt;
        feature liga {
            sub f f i by f_f_i;
            sub f f by f_f;
            sub f i by f_i;
        } liga;
        feature ccmp { sub a acutecomb by aacute; } ccmp;
        feature rlig { sub lam alef by lam_alef; } rlig;
    """)
    builder.save(LIGATURE_PATH)


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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ligatures-only", action="store_true",
                        help="generate only the original shared-glyph fixture")
    args = parser.parse_args()
    FIXTURES.mkdir(parents=True, exist_ok=True)
    if args.ligatures_only:
        build_ligature_ttf()
        return
    verify_source(DEJAVU_SOURCE, DEJAVU_SOURCE_SHA256)
    verify_source(DROID_CJK_SOURCE, DROID_CJK_SOURCE_SHA256)
    build_ttf()
    SYSTEM_FONT_TTF_PATH.parent.mkdir(parents=True, exist_ok=True)
    SYSTEM_FONT_TTF_PATH.write_bytes(TTF_PATH.read_bytes())
    convert("woff")
    convert("woff2")
    subset_font(DEJAVU_SOURCE, HEBREW_EMOJI_PATH, [0x20, 0x05D0, 0x05D1, 0x1F600])
    subset_font(DROID_CJK_SOURCE, CJK_PATH, [0x20, 0x4E2D, 0x6587])
    build_ligature_ttf()


if __name__ == "__main__":
    main()
