# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools==4.59.0"]
# ///
"""Derive an original variable/ligature PDF fixture from Moli's generated Ahem font."""
import copy
import struct
import zlib
from pathlib import Path

from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.colorLib.builder import buildCOLR, buildCPAL
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables._f_v_a_r import Axis
from fontTools.ttLib.tables.TupleVariation import TupleVariation
from fontTools.ttLib.tables.otTables import PaintFormat
from fontTools.ttLib.tables.sbixGlyph import Glyph as SbixGlyph
from fontTools.ttLib.tables.sbixStrike import Strike

root = Path(__file__).resolve().parents[1]
font = TTFont(root / "moli-layout/tests/fixtures/moli-ahem.ttf", recalcTimestamp=False)
# Distinct glyphs exercise multi-character ToUnicode mappings without relying
# on installed fonts or copying third-party typeface data.
for name, width in [("fi", 1200), ("ffi", 1800)]:
    glyph = copy.deepcopy(font["glyf"]["uni0066"])
    glyph.coordinates = type(glyph.coordinates)((int(x * width / 600), y) for x, y in glyph.coordinates)
    font["glyf"][name] = glyph
    font["hmtx"].metrics[name] = (width, 0)
added = ["fi", "ffi"]
for codepoint in [0x301, 0x5D0, 0x5D1, 0x5D2, 0x2605, 0x2606]:
    name = f"uni{codepoint:04X}"
    font["glyf"][name] = copy.deepcopy(font["glyf"]["uni0066"])
    font["hmtx"].metrics[name] = (600, 0)
    added.append(name)
    for table in font["cmap"].tables:
        if table.isUnicode():
            table.cmap[codepoint] = name
pen = TTGlyphPen(None)
pen.moveTo((0, 900))
pen.lineTo((200, 1100))
pen.lineTo((300, 1100))
pen.lineTo((100, 900))
pen.closePath()
font["glyf"]["uni0301"] = pen.glyph()
font["hmtx"].metrics["uni0301"] = (0, 0)
# A COLR base glyph may be empty even though its layers have visible ink.
font["glyf"]["uni2606"] = TTGlyphPen(None).glyph()
font.setGlyphOrder(list(dict.fromkeys(font.getGlyphOrder() + added)))
addOpenTypeFeaturesFromString(font, """
feature liga { sub uni0066 uni0066 uni0069 by ffi; sub uni0066 uni0069 by fi; } liga;
markClass uni0301 <anchor 100 900> @TOP;
feature mark { pos base uni0061 <anchor 300 800> mark @TOP; } mark;
""")
font["COLR"] = buildCOLR({name: [("uni0066", 0), ("uni0301", 1)] for name in ["uni2605", "uni2606"]}, glyphMap=font.getReverseGlyphMap())
font["CPAL"] = buildCPAL([[(1.0, 0.2, 0.0, 1.0), (0.0, 0.3, 1.0, 1.0)]])
font["fvar"] = newTable("fvar")
axis = Axis()
axis.axisTag, axis.minValue, axis.defaultValue, axis.maxValue = "wght", 400, 400, 900
axis.flags, axis.axisNameID = 0, font["name"].addName("Weight")
font["fvar"].axes = [axis]
font["fvar"].instances = []
font["avar"] = newTable("avar")
font["avar"].segments = {"wght": {-1.0: -1.0, 0.0: 0.0, 0.5: 0.75, 1.0: 1.0}}
font["gvar"] = newTable("gvar")
font["gvar"].variations = {}
for name in font.getGlyphOrder():
    glyph = font["glyf"][name]
    if glyph.numberOfContours > 0:
        # Widen the ink, but leave the advance unchanged. This distinguishes
        # correct font instancing from simply stretching positioned text.
        deltas = [(150 if x > 0 else 0, 0) for x, _ in glyph.coordinates]
        font["gvar"].variations[name] = [TupleVariation({"wght": (0.0, 1.0, 1.0)}, deltas + [(0, 0)] * 4)]
    else:
        font["gvar"].variations[name] = []
for record in list(font["name"].names):
    value = {1: "Moli PDF Test", 4: "Moli PDF Test", 6: "MoliPDFTest"}.get(record.nameID)
    if value is not None:
        font["name"].setName(value, record.nameID, record.platformID, record.platEncID, record.langID)
font.save(root / "moli-paint/tests/fixtures/moli-pdf.ttf")
# Static glyf metrics return an empty box for the hollow star's base glyph.
# Its COLR layers must still determine the visible representation.
for tag in ["gvar", "fvar", "avar"]:
    del font[tag]
font.save(root / "moli-paint/tests/fixtures/moli-colr.ttf")

# COLRv1 layers can be transformed beyond a generic two-em guard. The base
# remains empty; the paint graph, rather than its glyf box, supplies the ink.
font["COLR"] = buildCOLR({
    "uni2606": {
        "Format": PaintFormat.PaintTranslate,
        "dx": 0,
        "dy": 1600,
        "Paint": {
            "Format": PaintFormat.PaintColrLayers,
            "Layers": [
                {
                    "Format": PaintFormat.PaintGlyph,
                    "Glyph": glyph,
                    "Paint": {"Format": PaintFormat.PaintSolid, "PaletteIndex": palette, "Alpha": 1.0},
                }
                for glyph, palette in [("uni0066", 0), ("uni0301", 1)]
            ],
        },
    },
}, version=1, glyphMap=font.getReverseGlyphMap(), clipBoxes={"uni2606": (-100, 1500, 700, 2750)})
font.save(root / "moli-paint/tests/fixtures/moli-colr-v1.ttf")

# An original PNG strike with nonzero bearings, lying outside its monochrome
# outline. Use only standard-library PNG encoding to keep generation pinned.
bitmap = TTFont(root / "moli-layout/tests/fixtures/moli-ahem.ttf", recalcTimestamp=False)
def png_chunk(tag, payload):
    return struct.pack(">I", len(payload)) + tag + payload + struct.pack(">I", zlib.crc32(tag + payload))
png = b"\x89PNG\r\n\x1a\n"
png += png_chunk(b"IHDR", struct.pack(">IIBBBBB", 32, 48, 8, 6, 0, 0, 0))
png += png_chunk(b"IDAT", zlib.compress((b"\x00" + bytes([0, 64, 255, 255]) * 32) * 48))
png += png_chunk(b"IEND", b"")
strike = Strike(ppem=100, resolution=72)
strike.glyphs["uni0041"] = SbixGlyph(
    glyphName="uni0041", originOffsetX=-24, originOffsetY=80,
    graphicType="png ", imageData=png,
)
bitmap["sbix"] = newTable("sbix")
bitmap["sbix"].strikes = {100: strike}
bitmap.save(root / "moli-paint/tests/fixtures/moli-bitmap.ttf")
