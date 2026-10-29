from __future__ import annotations

import unittest
from typing import Any

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.helpers import wait_for_document_request


URL = "http://fixture.test/plain"


def _response(request: str = "wanted", loader: str = "new") -> dict[str, Any]:
    return {"method": "Network.responseReceived", "params": {
        "requestId": request, "loaderId": loader, "type": "Document", "response": {"url": URL},
    }}


def _finished(request: str = "wanted") -> dict[str, Any]:
    return {"method": "Network.loadingFinished", "params": {"requestId": request}}


class DocumentNetworkTerminalTests(unittest.IsolatedAsyncioTestCase):
    async def test_same_url_with_old_loader_cannot_complete_the_new_document(self) -> None:
        events = [_response("old-request", "old"), _finished("old-request"), _response()]
        with self.assertRaisesRegex(SmokeError, "loader=new.*request=wanted"):
            await wait_for_document_request(events, URL, loader_id="new", timeout_ms=10)

    async def test_response_headers_do_not_replace_the_response_body_terminal(self) -> None:
        with self.assertRaisesRegex(SmokeError, "Network terminal.*request=wanted"):
            await wait_for_document_request([_response()], URL, loader_id="new", timeout_ms=10)

    async def test_matching_request_terminal_completes_without_retrying_commands(self) -> None:
        events = [_response("old-request", "old"), _finished("old-request"), _response(), _finished()]
        self.assertEqual(await wait_for_document_request(events, URL, loader_id="new"), "wanted")

    async def test_failure_before_response_headers_is_reported_with_original_details(self) -> None:
        events = [{"method": "Network.requestWillBeSent", "params": {
            "requestId": "wanted", "loaderId": "new", "type": "Document", "request": {"url": URL},
        }}, {"method": "Network.loadingFailed", "params": {
            "requestId": "wanted", "errorText": "net::ERR_CONNECTION_RESET",
        }}]
        with self.assertRaisesRegex(SmokeError, "request=wanted.*net::ERR_CONNECTION_RESET"):
            await wait_for_document_request(events, URL, loader_id="new")

    async def test_an_unrelated_terminal_does_not_complete_the_request(self) -> None:
        with self.assertRaisesRegex(SmokeError, "request=wanted"):
            await wait_for_document_request([_response(), _finished("peer")], URL,
                                             loader_id="new", timeout_ms=10)

    async def test_duplicate_terminals_are_not_silently_accepted(self) -> None:
        with self.assertRaisesRegex(SmokeError, "duplicate Network terminals"):
            await wait_for_document_request([_response(), _finished(), _finished()], URL, loader_id="new")


if __name__ == "__main__":
    unittest.main()
