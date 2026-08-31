from __future__ import annotations

import re
from dataclasses import dataclass

from .assertions import SmokeError


@dataclass(frozen=True)
class MoliPdfInfo:
    page_count: int
    media_boxes: tuple[tuple[float, float], ...]
    xref_offset: int


def assert_pdf_envelope(data: bytes, label: str) -> None:
    if not data.startswith(b"%PDF-"):
        raise SmokeError(f"{label} did not start with a PDF header")
    if not data.rstrip().endswith(b"%%EOF"):
        raise SmokeError(f"{label} did not end with a PDF EOF marker")


def assert_moli_pdf_text_layer(data: bytes, label: str) -> None:
    assert_pdf_envelope(data, label)
    for marker in (
        b"/Subtype /Type0",
        b"/Encoding /Identity-H",
        b"/CIDToGIDMap /Identity",
        b"/ToUnicode",
        b"beginbfchar",
        b"3 Tr",
    ):
        if marker not in data:
            raise SmokeError(f"{label} did not contain the {marker!r} text-layer marker")
    if b"/FontFile2" not in data and b"/FontFile3" not in data:
        raise SmokeError(f"{label} did not embed a font program")


def inspect_moli_pdf(data: bytes, label: str) -> MoliPdfInfo:
    assert_pdf_envelope(data, label)
    if b"\xff\xd8" not in data or b"\xff\xd9" not in data:
        raise SmokeError(f"{label} did not contain the rendered JPEG payload")
    startxref = re.search(rb"startxref\s+(\d+)\s+%%EOF\s*$", data)
    if startxref is None:
        raise SmokeError(f"{label} did not contain a terminal startxref offset")
    xref_offset = int(startxref.group(1))
    if data[xref_offset : xref_offset + 4] != b"xref":
        raise SmokeError(f"{label} startxref did not point at the xref table")

    pages = re.search(rb"/Type /Pages /Kids \[[^]]*] /Count (\d+)", data)
    if pages is None:
        raise SmokeError(f"{label} did not contain the Moli Pages tree")
    page_count = int(pages.group(1))
    media_boxes = tuple(
        (float(width), float(height))
        for width, height in re.findall(
            rb"/MediaBox \[0 0 ([0-9.]+) ([0-9.]+)]",
            data,
        )
    )
    if len(media_boxes) != page_count:
        raise SmokeError(
            f"{label} Pages count/media boxes disagree: {page_count} vs {media_boxes!r}"
        )
    return MoliPdfInfo(
        page_count=page_count,
        media_boxes=media_boxes,
        xref_offset=xref_offset,
    )
