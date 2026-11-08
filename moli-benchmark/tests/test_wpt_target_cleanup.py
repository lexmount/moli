import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock

from moli_benchmark.raw_cdp import RawCdpError
from moli_benchmark.wpt_cross.runner import _close_target


class WptTargetCloseRaceTests(unittest.IsolatedAsyncioTestCase):
    async def test_concurrent_target_removal_is_accepted_after_confirmation(self):
        client = SimpleNamespace(
            send=AsyncMock(side_effect=[1, 2]),
            recv_until_id=AsyncMock(side_effect=[
                RawCdpError("No target with given id found"),
                ({"result": {"targetInfos": [{"targetId": "unrelated"}]}}, []),
            ]),
        )
        await _close_target(client, "case-target")

    async def test_close_errors_for_a_live_target_are_preserved(self):
        error = RawCdpError("close rejected")
        client = SimpleNamespace(
            send=AsyncMock(side_effect=[1, 2]),
            recv_until_id=AsyncMock(side_effect=[
                error,
                ({"result": {"targetInfos": [{"targetId": "case-target"}]}}, []),
            ]),
        )
        with self.assertRaises(RawCdpError) as caught:
            await _close_target(client, "case-target")
        self.assertIs(caught.exception, error)
