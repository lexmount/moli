from __future__ import annotations

import unittest

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups.tracing import _verify_start_end_race_wire_order


class _TraceWire:
    def __init__(self, messages):
        self.messages = list(messages)
        self.next_id = 0

    async def send(self, *_args):
        self.next_id += 1
        return self.next_id

    async def recv(self):
        return self.messages.pop(0)


START = {"id": 1, "result": {}}
END = {"id": 2, "result": {}}
INTERRUPTED = {"id": 1, "error": {
    "code": -32000, "message": "Tracing was stopped before start has been completed."
}}
DATA = {"method": "Tracing.dataCollected", "params": {"value": []}}
COMPLETE = {"method": "Tracing.tracingComplete", "params": {}}


class TracingWireOrderTests(unittest.IsolatedAsyncioTestCase):
    async def test_completed_start_keeps_normal_order(self):
        results = []
        await _verify_start_end_race_wire_order(_TraceWire([START, END, DATA, COMPLETE]), results)
        self.assertEqual(results[0]["outcome"], "started")

    async def test_interrupted_start_keeps_its_strict_order(self):
        results = []
        await _verify_start_end_race_wire_order(_TraceWire([END, INTERRUPTED, DATA, COMPLETE]), results)
        self.assertEqual(results[0]["outcome"], "interrupted")

    async def test_completed_start_cannot_follow_end_response(self):
        with self.assertRaisesRegex(SmokeError, "acknowledge start before end"):
            await _verify_start_end_race_wire_order(_TraceWire([END, START, DATA, COMPLETE]), [])

    async def test_interruption_must_report_the_specific_error(self):
        wrong = {"id": 1, "error": {"code": -32000, "message": "another failure"}}
        with self.assertRaisesRegex(SmokeError, "Tracing.start error"):
            await _verify_start_end_race_wire_order(_TraceWire([END, wrong, DATA, COMPLETE]), [])

    async def test_data_cannot_overtake_either_response(self):
        with self.assertRaises(SmokeError):
            await _verify_start_end_race_wire_order(_TraceWire([END, DATA, INTERRUPTED, COMPLETE]), [])

    async def test_terminal_response_cannot_be_duplicated(self):
        with self.assertRaisesRegex(SmokeError, "duplicate Tracing response"):
            await _verify_start_end_race_wire_order(_TraceWire([START, START, END, DATA, COMPLETE]), [])
