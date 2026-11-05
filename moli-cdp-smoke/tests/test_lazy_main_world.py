from __future__ import annotations

import json
import unittest
from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups.lazy_main_world import _Wire
from moli_cdp_smoke.raw_cdp import RawCdpClient, RawCdpError


def wire_with_response(response: dict[str, Any]) -> _Wire:
    socket = SimpleNamespace(
        send=AsyncMock(),
        recv=AsyncMock(return_value=json.dumps({"id": 1, **response})),
    )
    return _Wire(RawCdpClient(socket))  # type: ignore[arg-type]


class LazyMainWorldResponseTests(unittest.IsolatedAsyncioTestCase):
    async def test_protocol_error_is_rejected_by_real_raw_client(self) -> None:
        error = {"code": -32601, "message": "Method not found"}
        for response in ({"error": error}, {"error": error, "result": {}}):
            with self.subTest(response=response):
                wire = wire_with_response(response)
                with self.assertRaisesRegex(RawCdpError, "Method not found"):
                    await wire.call("Runtime.terminateExecution")

    async def test_missing_result_is_rejected(self) -> None:
        wire = wire_with_response({})
        with self.assertRaisesRegex(SmokeError, "result"):
            await wire.call("Runtime.terminateExecution")

    async def test_non_object_result_is_rejected(self) -> None:
        for result in (None, False, 0, "", []):
            with self.subTest(result=result):
                wire = wire_with_response({"result": result})
                with self.assertRaisesRegex(SmokeError, "result"):
                    await wire.call("Runtime.terminateExecution")

    async def test_explicit_empty_success_is_accepted(self) -> None:
        wire = wire_with_response({"result": {}})
        self.assertEqual(await wire.call("Runtime.terminateExecution"), {})

    async def test_javascript_exception_is_rejected(self) -> None:
        wire = wire_with_response({
            "result": {"exceptionDetails": {"text": "Uncaught Error: test"}},
        })
        with self.assertRaisesRegex(SmokeError, "Uncaught Error: test"):
            await wire.evaluate("page-session", "throw new Error('test')")

    async def test_evaluate_preserves_falsy_values(self) -> None:
        for value in (None, False, 0, ""):
            with self.subTest(value=value):
                wire = wire_with_response({"result": {"result": {"value": value}}})
                self.assertEqual(await wire.evaluate("page-session", "value"), value)
