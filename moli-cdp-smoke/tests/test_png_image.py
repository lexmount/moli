from __future__ import annotations

import struct
import unittest
import zlib

from moli_cdp_smoke.fixture import TRANSPARENT_PNG
from moli_cdp_smoke.png_image import PNG_SIGNATURE, decode_png


def _chunk(kind: bytes, payload: bytes) -> bytes:
    checksum = zlib.crc32(kind)
    checksum = zlib.crc32(payload, checksum) & 0xFFFFFFFF
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", checksum)


def _rgba_png(width: int, height: int, scanlines: bytes) -> bytes:
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (
        PNG_SIGNATURE
        + _chunk(b"IHDR", ihdr)
        + _chunk(b"IDAT", zlib.compress(scanlines))
        + _chunk(b"IEND", b"")
    )


class DecodePngTests(unittest.TestCase):
    def test_unfilters_rgba_scanlines(self) -> None:
        png = _rgba_png(
            2,
            2,
            b"\x00\xff\x00\x00\xff\x00\xff\x00\xff"
            b"\x01\x00\x00\xff\xff\xff\x00\x00\x00",
        )
        decoded = decode_png(png)

        self.assertEqual((decoded.width, decoded.height), (2, 2))
        self.assertEqual(decoded.pixel(0, 0), (255, 0, 0, 255))
        self.assertEqual(decoded.pixel(1, 0), (0, 255, 0, 255))
        self.assertEqual(decoded.pixel(0, 1), (0, 0, 255, 255))
        self.assertEqual(decoded.pixel(1, 1), (255, 0, 255, 255))
        self.assertEqual(decoded.distinct_color_count(), 4)


class ResourceFixturePngTests(unittest.TestCase):
    def test_resource_png_has_valid_chunk_crcs_and_unchanged_scanline(self) -> None:
        self.assertEqual(TRANSPARENT_PNG[:8], PNG_SIGNATURE)
        self.assertEqual(len(TRANSPARENT_PNG), 68)
        chunks = {}
        offset = len(PNG_SIGNATURE)
        while offset < len(TRANSPARENT_PNG):
            length = struct.unpack_from(">I", TRANSPARENT_PNG, offset)[0]
            kind = TRANSPARENT_PNG[offset + 4:offset + 8]
            payload = TRANSPARENT_PNG[offset + 8:offset + 8 + length]
            expected = struct.unpack_from(">I", TRANSPARENT_PNG, offset + 8 + length)[0]
            self.assertEqual(zlib.crc32(kind + payload) & 0xFFFFFFFF, expected,
                             f"the shared image fixture must have a valid {kind!r} CRC")
            chunks[kind] = payload
            offset += 12 + length
        self.assertEqual(offset, len(TRANSPARENT_PNG))
        self.assertEqual(list(chunks), [b"IHDR", b"IDAT", b"IEND"])
        self.assertEqual(struct.unpack(">IIBBBBB", chunks[b"IHDR"]), (1, 1, 8, 4, 0, 0, 0))
        self.assertEqual(zlib.decompress(chunks[b"IDAT"]), b"\x01\xff\xff")


if __name__ == "__main__":
    unittest.main()
