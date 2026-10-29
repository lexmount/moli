from __future__ import annotations

import unittest
from typing import Any

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups.browser_semantics import _event_source_network_summary
from moli_cdp_smoke.groups.network import _fetch_network_event_summary


REQUEST = "request"


def event(method: str, **params: Any) -> dict[str, Any]:
    return {"method": f"Network.{method}", "params": {"requestId": REQUEST, **params}}


def fetch_trace(chunks: list[tuple[int, int]]) -> list[dict[str, Any]]:
    return [
        event("requestWillBeSent"),
        event("responseReceived"),
        *(event("dataReceived", dataLength=decoded, encodedDataLength=encoded)
          for decoded, encoded in chunks),
        event("loadingFinished"),
    ]


class FetchNetworkEventsTests(unittest.TestCase):
    def test_counters_can_arrive_together_or_in_separate_chunks(self) -> None:
        # Observed on pinned Chromium 145.0.7632.6: a normal 45-byte JSON
        # response can split into (45, 0), (0, 45). Gzip does this frequently.
        for chunks, decoded, encoded in [
            ([(45, 45)], 45, 45),
            ([(45, 0), (0, 45)], 45, 45),
            ([(45, 0), (0, 35)], 45, 35),
            ([(1044, 0), (31737, 82)], 32781, 82),
        ]:
            with self.subTest(chunks=chunks):
                summary = _fetch_network_event_summary(fetch_trace(chunks), REQUEST)
                self.assertEqual(summary["dataLength"], decoded)
                self.assertEqual(summary["encodedDataLength"], encoded)
                self.assertEqual(summary["eventMethods"][-1], "Network.loadingFinished")

    def test_other_requests_cannot_supply_missing_byte_counts(self) -> None:
        events = fetch_trace([(45, 0)])
        events.insert(-1, event("dataReceived", requestId="other", dataLength=0, encodedDataLength=45))
        with self.assertRaisesRegex(SmokeError, "positive byte totals"):
            _fetch_network_event_summary(events, REQUEST)

    def test_nonempty_body_requires_both_positive_totals(self) -> None:
        for chunks in [[(45, 0)], [(0, 45)], [(0, 0)], []]:
            with self.subTest(chunks=chunks), self.assertRaises(SmokeError):
                _fetch_network_event_summary(fetch_trace(chunks), REQUEST)

    def test_invalid_chunk_lengths_are_rejected_before_summing(self) -> None:
        for key in ["dataLength", "encodedDataLength"]:
            for value in [-1, None, "45", True]:
                events = fetch_trace([(45, 45), (1, 1)])
                events[2]["params"][key] = value
                with self.subTest(key=key, value=value), self.assertRaisesRegex(SmokeError, "invalid Fetch"):
                    _fetch_network_event_summary(events, REQUEST)

    def test_data_must_follow_response_and_precede_one_successful_terminal(self) -> None:
        valid = fetch_trace([(45, 45)])
        for events in [
            [valid[0], valid[2], valid[1], valid[3]],
            [valid[0], valid[1], valid[3], valid[2]],
            [*valid, valid[-1]],
            [*valid[:-1], event("loadingFailed", errorText="net::ERR_ABORTED", canceled=True)],
        ]:
            with self.subTest(events=events), self.assertRaises(SmokeError):
                _fetch_network_event_summary(events, REQUEST)


class EventSourceNetworkEventsTests(unittest.TestCase):
    def test_close_accepts_committed_eof_or_an_explicit_abort(self) -> None:
        for terminal in [
            event("loadingFinished"),
            event("loadingFailed", errorText="net::ERR_ABORTED", canceled=True),
        ]:
            events = [event("requestWillBeSent"), event("responseReceived"), terminal]
            with self.subTest(terminal=terminal):
                summary = _event_source_network_summary(events, REQUEST)
                self.assertEqual(summary["terminal"], terminal)
                self.assertEqual(summary["eventMethods"], [entry["method"] for entry in events])

    def test_close_does_not_accept_unrelated_network_errors(self) -> None:
        for params in [
            {"errorText": "net::ERR_CONNECTION_RESET", "canceled": True},
            {"errorText": "net::ERR_ABORTED", "canceled": False},
            {"errorText": "net::ERR_ABORTED"},
        ]:
            events = [event("requestWillBeSent"), event("responseReceived"), event("loadingFailed", **params)]
            with self.subTest(params=params), self.assertRaises(SmokeError):
                _event_source_network_summary(events, REQUEST)

    def test_close_requires_ordered_response_and_exactly_one_terminal(self) -> None:
        request, response, finished = event("requestWillBeSent"), event("responseReceived"), event("loadingFinished")
        aborted = event("loadingFailed", errorText="net::ERR_ABORTED", canceled=True)
        for events in [
            [request, response],
            [request, finished, response],
            [request, response, finished, aborted],
            [request, response, finished, finished],
        ]:
            with self.subTest(events=events), self.assertRaises(SmokeError):
                _event_source_network_summary(events, REQUEST)

    def test_other_request_terminals_are_ignored(self) -> None:
        terminal = event("loadingFinished")
        summary = _event_source_network_summary([
            event("requestWillBeSent"), event("responseReceived"),
            event("loadingFailed", requestId="other", errorText="net::ERR_FAILED"), terminal,
        ], REQUEST)
        self.assertEqual(summary["terminal"], terminal)


if __name__ == "__main__":
    unittest.main()
