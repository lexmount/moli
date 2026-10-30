from __future__ import annotations

from copy import deepcopy
import unittest

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups.native_drag_semantics import (
    _assert_payload,
    _fragment_summary,
)


DOCUMENT_URL = "http://127.0.0.1:8000/"
IMAGE_URL = "http://127.0.0.1:8000/chromium-resource-image.png"
LINK_URL = "https://drag-source.test/details?x=1&y=2"


def observed(items: list[dict]) -> tuple[dict, dict, dict]:
    strings = {item["mimeType"]: item["data"] for item in items}
    types = list(strings)
    return ({"strings": strings.copy(), "types": types.copy()},
            {"items": items, "dragOperationsMask": -1},
            {"strings": strings.copy(), "types": types.copy()})


def selection() -> tuple[dict, dict, dict]:
    return observed([
        {"mimeType": "text/html", "data": '<b style="font-size:16px">Rich</b><i> text</i>', "baseURL": DOCUMENT_URL},
        {"mimeType": "text/plain", "data": "Rich text"},
    ])


def link() -> tuple[dict, dict, dict]:
    return observed([
        {"mimeType": "text/uri-list", "data": LINK_URL, "title": "Link label"},
        {"mimeType": "text/plain", "data": LINK_URL},
        {"mimeType": "text/html", "data": '<a href="https://drag-source.test/details?x=1&amp;y=2" title="Anchor title">Link <b>label</b></a>', "baseURL": LINK_URL},
    ])


def image() -> tuple[dict, dict, dict]:
    return observed([
        {"mimeType": "text/uri-list", "data": LINK_URL, "title": "Picture alt"},
        {"mimeType": "text/html", "data": f'<img src="{IMAGE_URL}" alt="Picture alt" title="Image title">',
         "baseURL": "about:blank#blocked"},
    ])


class NativeDragSemanticsTests(unittest.TestCase):
    def check(self, kind: str, values: tuple[dict, dict, dict], *, overridden: bool = False) -> None:
        _assert_payload(kind, *values, DOCUMENT_URL, IMAGE_URL, overridden=overridden)

    def test_calibrated_payloads_cover_formats_and_their_private_metadata(self) -> None:
        for kind, fixture in (("selection", selection), ("link", link), ("linked-image", image)):
            with self.subTest(kind=kind):
                self.check(kind, fixture())

    def test_selection_requires_html_formatting_and_document_base_url(self) -> None:
        for change in (
            lambda packet: packet["items"][0].update(data="Rich text"),
            lambda packet: packet["items"][0].update(baseURL="https://drag-source.test/assets/"),
        ):
            values = selection()
            change(values[1])
            with self.assertRaises(SmokeError):
                self.check("selection", values)

    def test_native_link_title_and_image_href_are_not_substituted(self) -> None:
        values = link()
        values[1]["items"][0]["title"] = "Anchor title"
        with self.assertRaises(SmokeError):
            self.check("link", values)
        values = image()
        values[1]["items"][0]["data"] = IMAGE_URL
        with self.assertRaises(SmokeError):
            self.check("linked-image", values)

    def test_mask_normalization_and_drop_payload_loss_are_failures(self) -> None:
        values = selection()
        values[1]["dragOperationsMask"] = 19
        with self.assertRaises(SmokeError):
            self.check("selection", values)
        values = selection()
        del values[2]["strings"]["text/html"]
        with self.assertRaises(SmokeError):
            self.check("selection", values)

    def test_author_replacement_clears_stale_title_and_base_url(self) -> None:
        values = observed([
            {"mimeType": "text/plain", "data": LINK_URL},
            {"mimeType": "text/uri-list", "data": "https://override.test/changed"},
            {"mimeType": "text/html", "data": "<em>author changed</em>", "baseURL": "about:blank#blocked"},
        ])
        self.check("link", values, overridden=True)
        retained_position = observed([values[1]["items"][1], values[1]["items"][0], values[1]["items"][2]])
        self.check("link", retained_position, overridden=True)
        for field, old in (("title", "Link label"), ("baseURL", LINK_URL)):
            changed = deepcopy(values)
            changed[1]["items"][1 if field == "title" else 2][field] = old
            with self.subTest(field=field), self.assertRaises(SmokeError):
                self.check("link", changed, overridden=True)

    def test_semantic_html_checks_do_not_require_chromium_style_annotation_bytes(self) -> None:
        first = _fragment_summary('<span><b style="color:rgb(0,0,0)">Rich</b><em> text</em></span>')
        second = _fragment_summary("<b>Rich</b><i> text</i>")
        self.assertEqual(first, second)
        self.assertEqual(first["bold"], "Rich")
        self.assertEqual(first["italic"], " text")


if __name__ == "__main__":
    unittest.main()
