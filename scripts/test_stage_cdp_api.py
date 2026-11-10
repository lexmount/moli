"""Run: PYTHONPATH=moli-benchmark python -m unittest discover -s scripts -p test_stage_cdp_api.py"""
from __future__ import annotations

import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, patch


class StageCdpApiTests(unittest.IsolatedAsyncioTestCase):
    async def test_current_navigation_observation_contract(self) -> None:
        for script in ("run-detailed-stage-compare.py", "run-three-way-compare.py"):
            spec = importlib.util.spec_from_file_location("stage_compare", Path(__file__).with_name(script))
            assert spec is not None and spec.loader is not None
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            for headers_only in (False, True):
                with self.subTest(script=script, headers_only=headers_only):
                    methods: dict[int, str] = {}

                    async def send(method, *args, **kwargs):
                        message_id = len(methods) + 1
                        methods[message_id] = method
                        return message_id

                    async def response(message_id, **kwargs):
                        method = methods[message_id]
                        result = {
                            "Target.createTarget": {"targetId": "target"},
                            "Target.attachToTarget": {"sessionId": "session"},
                            "Page.navigate": {"frameId": "frame", "loaderId": "loader"},
                            "Runtime.evaluate": {"result": {"value": "<html>ok</html>"}},
                        }.get(method, {})
                        seen = []
                        if method == "Page.navigate":
                            if headers_only:
                                seen = [{"method": "Network.responseReceived", "sessionId": "session", "params": {
                                    "type": "Document", "frameId": "frame", "loaderId": "loader", "requestId": "request",
                                    "response": {"mimeType": "image/png", "status": 200, "url": "https://example.test/"},
                                }}]
                            else:
                                seen = [{"method": "Page.lifecycleEvent", "sessionId": "session", "params": {
                                    "frameId": "frame", "loaderId": "loader", "name": "DOMContentLoaded",
                                }}]
                        return {"result": result}, seen

                    client = SimpleNamespace(send=send, recv_until_id=response, websocket=SimpleNamespace(close=AsyncMock()))
                    # Use the real CDP response and navigation observation
                    # helpers; only the transport boundary is simulated.
                    with patch.object(module, "_wait_for_cdp", AsyncMock(return_value=client)):
                        timing = await module._dump_dcl_with_timing("unused", None, "https://example.test/", 5)
                    self.assertEqual(timing["html_length"], 0 if headers_only else 15)
                    self.assertEqual("Runtime.evaluate" in methods.values(), not headers_only)
                    client.websocket.close.assert_awaited_once()


if __name__ == "__main__":
    unittest.main()
