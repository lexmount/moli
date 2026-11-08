from __future__ import annotations

import unittest
from copy import deepcopy

from moli_benchmark.raw_cdp import RecordedCdpMessage
from moli_benchmark.sequential_navigate import (
    NavigationIdentity,
    find_exact_lifecycle_record,
    find_navigation_progress_record,
    make_recovery_url,
    network_event_order_violations,
    navigation_order_violations,
    normalize_public_url,
    parse_top_100_domains,
    select_seed_urls,
    selected_engines,
    summarize_navigation_resource_samples,
    summarize_network_activity,
)


def record(sequence: int, payload: dict[str, object]) -> RecordedCdpMessage:
    return RecordedCdpMessage(
        sequence=sequence,
        received_monotonic=float(sequence),
        received_epoch=float(sequence),
        payload=payload,
    )


def lifecycle(
    sequence: int,
    *,
    frame_id: str,
    loader_id: str,
    name: str,
    session_id: str = "SID-1",
) -> RecordedCdpMessage:
    return record(
        sequence,
        {
            "sessionId": session_id,
            "method": "Page.lifecycleEvent",
            "params": {
                "frameId": frame_id,
                "loaderId": loader_id,
                "name": name,
            },
        },
    )


def navigation_started(
    sequence: int,
    *,
    frame_id: str,
    loader_id: str,
    url: str = "https://next.test/",
    session_id: str = "SID-1",
) -> RecordedCdpMessage:
    return record(
        sequence,
        {
            "sessionId": session_id,
            "method": "Page.frameStartedNavigating",
            "params": {
                "frameId": frame_id,
                "loaderId": loader_id,
                "url": url,
            },
        },
    )


def network_event(
    sequence: int,
    *,
    method: str,
    request_id: str,
    session_id: str = "SID-1",
) -> RecordedCdpMessage:
    params: dict[str, object] = {"requestId": request_id}
    if method == "Network.requestWillBeSent":
        params.update(
            {
                "type": "XHR",
                "request": {"url": f"https://example.test/{request_id}"},
            }
        )
    elif method == "Network.responseReceived":
        params["type"] = "XHR"
    return record(
        sequence,
        {
            "sessionId": session_id,
            "method": method,
            "params": params,
        },
    )


def error_document_events() -> list[RecordedCdpMessage]:
    # Chromium 145: one failed network request followed by the internal error
    # Document's completion, correlated by its frame, loader and unreachable URL.
    return [
        record(1, {"sessionId": "SID-1", "method": "Network.requestWillBeSent", "params": {
            "requestId": "REQ-error", "frameId": "FRAME-error", "loaderId": "LOADER-error",
            "type": "Document", "request": {"url": "https://example.test/reset"},
        }}),
        record(2, {"sessionId": "SID-1", "method": "Network.loadingFailed", "params": {
            "requestId": "REQ-error", "type": "Document", "canceled": False,
            "errorText": "net::ERR_CONNECTION_RESET",
        }}),
        record(3, {"sessionId": "SID-1", "method": "Page.frameNavigated", "params": {"frame": {
            "id": "FRAME-error", "loaderId": "LOADER-error", "url": "chrome-error://chromewebdata/",
            "unreachableUrl": "https://example.test/reset",
        }}}),
        record(4, {"sessionId": "SID-1", "method": "Network.loadingFinished", "params": {
            "requestId": "REQ-error", "encodedDataLength": 0,
        }}),
    ]


class SequentialNavigateTests(unittest.TestCase):
    def test_navigation_resource_summary_reports_windows_slope_and_quarters(self) -> None:
        mib = 1024 * 1024
        samples = [
            {
                "index": 0,
                "rss_bytes": 100 * mib,
                "pss_bytes": 90 * mib,
                "fd_count": 20,
                "thread_count": 8,
                "process_count": 1,
                "error": None,
            }
        ]
        samples.extend(
            {
                "index": index,
                "rss_bytes": (100 + index) * mib,
                "pss_bytes": (90 + index / 2) * mib,
                "fd_count": 20 + index // 50,
                "thread_count": 8,
                "process_count": 1,
                "error": None,
            }
            for index in range(1, 201)
        )

        summary = summarize_navigation_resource_samples(
            samples,
            {
                "sample_count": 500,
                "peak_rss_bytes": 305 * mib,
                "peak_pss_bytes": 195 * mib,
                "peak_fd_count": 24,
                "peak_thread_count": 8,
                "observer_error": None,
                "late_sample_count": 0,
            },
        )

        self.assertEqual(summary["sample_count"], 200)
        self.assertTrue(summary["initial_sample_present"])
        self.assertEqual(summary["sample_errors"], 0)
        self.assertEqual(summary["rss_bytes"]["first_window_average"], 105.5 * mib)
        self.assertEqual(summary["rss_bytes"]["last_window_average"], 295.5 * mib)
        self.assertEqual(
            summary["rss_bytes"]["first_to_last_window_delta"],
            190 * mib,
        )
        self.assertAlmostEqual(
            summary["rss_bytes"]["warm_slope_per_100_navigations"],
            100 * mib,
        )
        self.assertEqual(
            [
                (quarter["start_index"], quarter["end_index"], quarter["sample_count"])
                for quarter in summary["quarters"]
            ],
            [(1, 50, 50), (51, 100, 50), (101, 150, 50), (151, 200, 50)],
        )
        self.assertEqual(summary["periodic"]["peak_rss_bytes"], 305 * mib)

    def test_url_normalization_preserves_explicit_data_url(self) -> None:
        url = "data:text/html,<title>fixture</title>"

        self.assertEqual(normalize_public_url(url), url)

    def test_seed_parser_reads_only_top_100_section(self) -> None:
        markdown = """
## Earlier
1. `ignored.example`
## Top 100
1. `csdn.net` — CSDN
2. `zol.com.cn` — ZOL
## Maintenance
1. `also-ignored.example`
"""
        self.assertEqual(parse_top_100_domains(markdown), ["csdn.net", "zol.com.cn"])

    def test_seed_selection_pins_repros_and_is_deterministic(self) -> None:
        domains = ["alpha.test", "csdn.net", "beta.test", "zol.com.cn", "gamma.test"]
        first = select_seed_urls(domains, seed=17, count=4)
        second = select_seed_urls(domains, seed=17, count=4)

        self.assertEqual(first, second)
        self.assertEqual(first[:2], ["https://csdn.net", "https://zol.com.cn"])
        self.assertEqual(len(set(first)), 4)

    def test_lifecycle_match_requires_exact_session_frame_and_loader(self) -> None:
        identity = NavigationIdentity(frame_id="FRAME-NEW", loader_id="LOADER-NEW")
        records = [
            lifecycle(1, frame_id="FRAME-OLD", loader_id="LOADER-OLD", name="DOMContentLoaded"),
            lifecycle(2, frame_id="FRAME-NEW", loader_id="LOADER-OLD", name="DOMContentLoaded"),
            lifecycle(
                3,
                frame_id="FRAME-NEW",
                loader_id="LOADER-NEW",
                name="DOMContentLoaded",
                session_id="SID-OLD",
            ),
            lifecycle(4, frame_id="FRAME-NEW", loader_id="LOADER-NEW", name="DOMContentLoaded"),
        ]

        matched = find_exact_lifecycle_record(
            records,
            session_id="SID-1",
            identity=identity,
            name="DOMContentLoaded",
        )

        self.assertIsNotNone(matched)
        self.assertEqual(matched.sequence if matched else None, 4)

    def test_navigation_progress_uses_earliest_lifecycle_or_same_frame_successor(self) -> None:
        identity = NavigationIdentity(frame_id="FRAME", loader_id="LOADER-INITIAL")
        successor = navigation_started(
            2,
            frame_id="FRAME",
            loader_id="LOADER-SUCCESSOR",
        )
        records = [
            navigation_started(1, frame_id="OTHER", loader_id="LOADER-IGNORED"),
            successor,
            lifecycle(3, frame_id="FRAME", loader_id="LOADER-INITIAL", name="DOMContentLoaded"),
        ]

        progress = find_navigation_progress_record(
            records,
            session_id="SID-1",
            identity=identity,
            lifecycle_name="DOMContentLoaded",
        )

        self.assertIsNotNone(progress)
        self.assertEqual(progress.kind if progress else None, "successor")
        self.assertEqual(progress.record.sequence if progress else None, 2)
        self.assertEqual(
            progress.identity if progress else None,
            NavigationIdentity(frame_id="FRAME", loader_id="LOADER-SUCCESSOR"),
        )

    def test_navigation_progress_keeps_lifecycle_when_it_precedes_successor(self) -> None:
        identity = NavigationIdentity(frame_id="FRAME", loader_id="LOADER-INITIAL")
        records = [
            lifecycle(1, frame_id="FRAME", loader_id="LOADER-INITIAL", name="DOMContentLoaded"),
            navigation_started(2, frame_id="FRAME", loader_id="LOADER-SUCCESSOR"),
        ]

        progress = find_navigation_progress_record(
            records,
            session_id="SID-1",
            identity=identity,
            lifecycle_name="DOMContentLoaded",
        )

        self.assertIsNotNone(progress)
        self.assertEqual(progress.kind if progress else None, "lifecycle")
        self.assertEqual(progress.record.sequence if progress else None, 1)

    def test_order_check_rejects_lifecycle_before_navigate_response(self) -> None:
        response = record(3, {"id": 7, "result": {}})
        dcl = lifecycle(2, frame_id="FRAME", loader_id="LOADER", name="DOMContentLoaded")
        load = lifecycle(4, frame_id="FRAME", loader_id="LOADER", name="load")

        self.assertEqual(
            navigation_order_violations(response, dcl, load),
            ["DOMContentLoaded was observable before Page.navigate response"],
        )

    def test_order_check_accepts_response_then_dcl_then_load(self) -> None:
        response = record(2, {"id": 7, "result": {}})
        dcl = lifecycle(3, frame_id="FRAME", loader_id="LOADER", name="DOMContentLoaded")
        load = lifecycle(4, frame_id="FRAME", loader_id="LOADER", name="load")

        self.assertEqual(navigation_order_violations(response, dcl, load), [])

    def test_recovery_url_carries_unique_marker(self) -> None:
        url = make_recovery_url("marker-123")

        self.assertTrue(url.startswith("data:text/html;charset=utf-8,"))
        self.assertIn("marker-123", url)

    def test_engine_selection_can_run_each_engine_or_both(self) -> None:
        self.assertEqual(selected_engines("moli"), ("moli",))
        self.assertEqual(selected_engines("chromium"), ("chromium",))
        self.assertEqual(selected_engines("both"), ("moli", "chromium"))

        with self.assertRaisesRegex(ValueError, "unsupported engine"):
            selected_engines("unknown")

    def test_network_summary_reports_terminal_and_inflight_requests(self) -> None:
        records = [
            record(
                1,
                {
                    "sessionId": "SID-1",
                    "method": "Network.requestWillBeSent",
                    "params": {
                        "requestId": "REQ-finished",
                        "frameId": "FRAME",
                        "loaderId": "LOADER",
                        "type": "Document",
                        "request": {"url": "https://example.test/"},
                        "initiator": {"type": "other"},
                    },
                },
            ),
            record(
                2,
                {
                    "sessionId": "SID-1",
                    "method": "Network.loadingFinished",
                    "params": {"requestId": "REQ-finished"},
                },
            ),
            record(
                3,
                {
                    "sessionId": "SID-1",
                    "method": "Network.requestWillBeSent",
                    "params": {
                        "requestId": "REQ-inflight",
                        "frameId": "FRAME",
                        "loaderId": "LOADER",
                        "type": "Script",
                        "request": {"url": "https://example.test/blocker.js"},
                        "initiator": {"type": "parser"},
                    },
                },
            ),
            record(
                4,
                {
                    "sessionId": "SID-other",
                    "method": "Network.loadingFailed",
                    "params": {"requestId": "REQ-inflight", "errorText": "ignored"},
                },
            ),
        ]

        summary = summarize_network_activity(records, session_id="SID-1", started=0.0)

        self.assertEqual(summary["request_count"], 2)
        self.assertEqual(summary["finished_count"], 1)
        self.assertEqual(summary["failed_count"], 0)
        self.assertEqual(summary["inflight_count"], 1)
        self.assertEqual(summary["requests"][1]["request_id"], "REQ-inflight")
        self.assertEqual(summary["requests"][1]["type"], "Script")
        self.assertIsNone(summary["requests"][1]["terminal"])

    def test_network_order_audit_accepts_success_failure_and_redirect_order(self) -> None:
        records = [
            network_event(1, method="Network.requestWillBeSent", request_id="REQ-ok"),
            network_event(2, method="Network.requestWillBeSent", request_id="REQ-ok"),
            network_event(3, method="Network.responseReceived", request_id="REQ-ok"),
            network_event(4, method="Network.loadingFinished", request_id="REQ-ok"),
            network_event(5, method="Network.requestWillBeSent", request_id="REQ-fail"),
            network_event(6, method="Network.loadingFailed", request_id="REQ-fail"),
            network_event(
                7,
                method="Network.loadingFinished",
                request_id="REQ-other-session",
                session_id="SID-other",
            ),
        ]

        self.assertEqual(
            network_event_order_violations(records, session_id="SID-1"),
            [],
        )

    def test_network_order_audit_reports_success_terminal_before_start_and_response(self) -> None:
        records = [
            network_event(1, method="Network.loadingFinished", request_id="REQ-late"),
            network_event(2, method="Network.requestWillBeSent", request_id="REQ-late"),
            network_event(3, method="Network.responseReceived", request_id="REQ-late"),
        ]

        violations = network_event_order_violations(records, session_id="SID-1")

        self.assertEqual(
            [violation["kind"] for violation in violations],
            ["terminal_before_start", "successful_terminal_before_response"],
        )
        self.assertTrue(
            all(violation["request_id"] == "REQ-late" for violation in violations)
        )

    def test_network_order_audit_reports_response_order_and_duplicate_terminal(self) -> None:
        records = [
            network_event(1, method="Network.responseReceived", request_id="REQ-response"),
            network_event(2, method="Network.requestWillBeSent", request_id="REQ-response"),
            network_event(3, method="Network.requestWillBeSent", request_id="REQ-duplicate"),
            network_event(4, method="Network.loadingFailed", request_id="REQ-duplicate"),
            network_event(5, method="Network.loadingFailed", request_id="REQ-duplicate"),
        ]

        violations = network_event_order_violations(records, session_id="SID-1")

        self.assertEqual(
            [(violation["request_id"], violation["kind"]) for violation in violations],
            [
                ("REQ-response", "response_before_start"),
                ("REQ-duplicate", "duplicate_terminal"),
            ],
        )

    def test_network_order_audit_accepts_correlated_error_document_completion(self) -> None:
        self.assertEqual(
            network_event_order_violations(error_document_events(), session_id="SID-1"), [],
        )

    def test_network_order_audit_accepts_error_document_after_redirect(self) -> None:
        events = error_document_events()
        initial = deepcopy(events[0].payload)
        initial["params"]["request"]["url"] = "https://example.test/redirect"
        records = [record(1, initial)] + [record(event.sequence + 1, event.payload) for event in events]
        self.assertEqual(network_event_order_violations(records, session_id="SID-1"), [])

    def test_network_order_audit_does_not_exempt_unrelated_or_invalid_completions(self) -> None:
        for case in [
            "missing_commit", "other_session", "other_frame", "other_loader", "other_url",
            "normal_document", "missing_request_frame", "missing_request_loader", "xhr",
            "canceled", "extra_finished", "extra_failed", "finished_before_commit",
        ]:
            with self.subTest(case=case):
                events = error_document_events()
                request = events[0].payload["params"]
                frame_event = events[2].payload
                frame = frame_event["params"]["frame"]
                if case == "missing_commit":
                    del events[2]
                elif case == "other_session":
                    frame_event["sessionId"] = "SID-other"
                elif case == "other_frame":
                    frame["id"] = "FRAME-other"
                elif case == "other_loader":
                    frame["loaderId"] = "LOADER-other"
                elif case == "other_url":
                    frame["unreachableUrl"] = "https://example.test/other"
                elif case == "normal_document":
                    frame["url"] = "https://example.test/reset"
                elif case == "missing_request_frame":
                    del request["frameId"]
                elif case == "missing_request_loader":
                    del request["loaderId"]
                elif case == "xhr":
                    request["type"] = "XHR"
                elif case == "canceled":
                    events[1].payload["params"]["canceled"] = True
                elif case == "extra_finished":
                    events.append(network_event(5, method="Network.loadingFinished", request_id="REQ-error"))
                elif case == "extra_failed":
                    events.append(network_event(5, method="Network.loadingFailed", request_id="REQ-error"))
                elif case == "finished_before_commit":
                    events = events[:2] + [record(3, events[3].payload), record(4, events[2].payload)]
                self.assertEqual(
                    {violation["kind"] for violation in network_event_order_violations(events, session_id="SID-1")},
                    {"successful_terminal_before_response", "duplicate_terminal"},
                )

    def test_network_order_audit_still_rejects_duplicate_successful_terminals(self) -> None:
        records = [
            network_event(1, method="Network.requestWillBeSent", request_id="REQ-ok"),
            network_event(2, method="Network.responseReceived", request_id="REQ-ok"),
            network_event(3, method="Network.loadingFinished", request_id="REQ-ok"),
            network_event(4, method="Network.loadingFinished", request_id="REQ-ok"),
        ]
        self.assertEqual(
            [violation["kind"] for violation in network_event_order_violations(records, session_id="SID-1")],
            ["duplicate_terminal"],
        )

    def test_error_document_completion_keeps_the_network_failure_in_summary(self) -> None:
        summary = summarize_network_activity(error_document_events(), session_id="SID-1", started=0.0)
        self.assertEqual(summary["failed_count"], 1)
        self.assertEqual(summary["finished_count"], 0)
        self.assertEqual(summary["inflight_count"], 0)
        request = summary["requests"][0]
        self.assertEqual(request["terminal"], "failed")
        self.assertEqual(request["terminal_sequence"], 2)
        self.assertEqual(request["error_text"], "net::ERR_CONNECTION_RESET")


if __name__ == "__main__":
    unittest.main()
