from __future__ import annotations

from wpt_cross_test_support import *


class ClearSiteDataCookieFixtureTests(WptCrossTestCase):
    def test_cookie_fixture_preserves_response_header_order(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            resources = root_path / "resources"
            resources.mkdir()
            (resources / "testharness.js").write_text("// testharness", encoding="utf-8")
            with WptFixtureServer(root_path) as server:
                endpoint = f"{server.base_url}/clear-site-data/support/clear-site-data-cookie.py"
                for query, directive, order in (
                    ("location=before&target=cookies", '"cookies"', ["Set-Cookie", "Clear-Site-Data"]),
                    ("location=after", '"*"', ["Clear-Site-Data", "Set-Cookie"]),
                ):
                    with self.subTest(query=query), urlopen(f"{endpoint}?{query}", timeout=2) as response:
                        self.assertEqual(response.headers["Clear-Site-Data"], directive)
                        self.assertEqual(response.headers["Set-Cookie"], "testSetWithClear=true")
                        self.assertEqual(
                            [name for name in response.headers.keys() if name in order],
                            order,
                        )
                        self.assertIn(b"window.opener.postMessage(document.cookie", response.read())
