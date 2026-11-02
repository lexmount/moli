from __future__ import annotations

import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, call, patch

from moli_cdp_smoke.groups import browser_endpoint, protocol, target_lifecycle


class BrowserEndpointTests(unittest.IsolatedAsyncioTestCase):
    endpoint = "http://192.0.2.10:9222"
    browser_path = "/devtools/browser/11111111-1111-4111-8111-111111111111"

    async def test_external_protocol_discovery_never_launches_a_local_browser(self) -> None:
        results = []
        with (
            patch.object(browser_endpoint, "discover_websocket_url", AsyncMock(
                return_value="ws://192.0.2.10:9222" + self.browser_path,
            )) as discover,
            patch.object(browser_endpoint, "_reject", AsyncMock()) as reject,
            patch.object(browser_endpoint, "start_browser", AsyncMock(
                side_effect=AssertionError("external endpoint must not launch a browser"),
            )) as start,
            patch.object(protocol, "connect_raw_cdp", AsyncMock(
                side_effect=RuntimeError("after discovery"),
            )),
        ):
            with self.assertRaisesRegex(RuntimeError, "after discovery"):
                await protocol.run_raw_protocol_group(self.endpoint, "http://fixture", results)
        start.assert_not_awaited()
        self.assertEqual(discover.await_count, 2)
        self.assertEqual(reject.await_count, 2)
        self.assertEqual(results[0]["name"], "browser_endpoint_uuid_discovery")

    async def test_lifecycle_checks_only_launch_browsers_for_managed_processes(self) -> None:
        managed = SimpleNamespace(process=SimpleNamespace(pid=123), engine="moli")
        for serve in (None, managed):
            with (
                self.subTest(managed=serve is not None),
                patch.object(target_lifecycle, "browser_endpoint_lifecycle", AsyncMock()) as lifecycle,
                patch.object(target_lifecycle, "connect_raw_cdp", AsyncMock(
                    side_effect=RuntimeError("after lifecycle"),
                )),
            ):
                results = []
                with self.assertRaisesRegex(RuntimeError, "after lifecycle"):
                    await target_lifecycle.run_target_lifecycle_group(
                        self.endpoint, "http://fixture", results, serve,
                    )
                if serve is None:
                    lifecycle.assert_not_awaited()
                else:
                    lifecycle.assert_awaited_once_with(self.endpoint, results)

    async def test_managed_lifecycle_checks_restart_on_the_same_port(self) -> None:
        other_endpoint = "http://127.0.0.1:9333"
        other_path = "/devtools/browser/22222222-2222-4222-8222-222222222222"
        restarted_path = "/devtools/browser/33333333-3333-4333-8333-333333333333"
        first, restarted = object(), object()
        client = SimpleNamespace(
            send=AsyncMock(return_value=1),
            recv_until_id=AsyncMock(return_value=({"result": {"product": "Moli/test"}}, [])),
            websocket=SimpleNamespace(close=AsyncMock()),
        )
        results = []
        with (
            patch.object(browser_endpoint, "discover_websocket_url", AsyncMock(side_effect=[
                "ws://192.0.2.10:9222" + self.browser_path,
                "ws://127.0.0.1:9333" + other_path,
                "ws://127.0.0.1:9333" + restarted_path,
            ])),
            patch.object(browser_endpoint, "_reject", AsyncMock()) as reject,
            patch.object(browser_endpoint, "start_browser", AsyncMock(
                side_effect=[first, restarted],
            )) as start,
            patch.object(browser_endpoint, "stop_browser", AsyncMock()) as stop,
            patch.object(browser_endpoint, "wait_for_browser_endpoint", AsyncMock(
                return_value=other_endpoint,
            )),
            patch.object(browser_endpoint, "connect_raw_cdp", AsyncMock(return_value=client)),
        ):
            await browser_endpoint.browser_endpoint_lifecycle(self.endpoint, results)
        self.assertEqual(start.await_args_list, [call(), call(9333)])
        self.assertEqual(stop.await_args_list, [call(first), call(restarted)])
        reject.assert_any_await(other_endpoint, other_path)
        client.websocket.close.assert_awaited_once()
        self.assertEqual(results[0]["name"], "browser_endpoint_uuid_restart")
