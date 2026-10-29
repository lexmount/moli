from __future__ import annotations

import unittest
from typing import Any

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups.tracing import _assert_cpu_profile_events


def _profile(pid: int = 11) -> dict[str, Any]:
    return {"name": "Profile", "pid": pid, "tid": pid, "id": "0x1",
            "args": {"data": {"source": "Internal", "startTime": 1}}}


def _chunk(pid: int = 11, *, nodes: list[Any] | None = None,
           samples: list[int] | None = None) -> dict[str, Any]:
    if nodes is None:
        nodes = [{"id": 1, "callFrame": {"functionName": "(root)"}},
                 {"id": 2, "parent": 1, "callFrame": {"functionName": "hot"}}]
    samples = [2] if samples is None else samples
    return {"name": "ProfileChunk", "pid": pid, "tid": pid + 1, "id": "0x1",
            "args": {"data": {"source": "Internal", "cpuProfile": {
                "nodes": nodes, "samples": samples,
            }, "timeDeltas": [100] * len(samples)}}}


class CpuTraceProfileTests(unittest.TestCase):
    def test_profile_ids_cannot_be_borrowed_from_another_process(self) -> None:
        with self.assertRaisesRegex(SmokeError, "ProfileChunk ids"):
            _assert_cpu_profile_events([_profile(11), _chunk(22)], {"hot"})

    def test_process_local_ids_can_be_reused_and_chunk_thread_can_differ(self) -> None:
        summary = _assert_cpu_profile_events([
            _profile(11), _profile(22), _chunk(11), _chunk(22),
        ], {"hot"})
        self.assertEqual(summary.profile_count, 2)
        self.assertEqual(summary.sample_count, 2)
        self.assertEqual(summary.function_samples, {"hot": 2})

    def test_function_declaration_without_a_sample_does_not_pass(self) -> None:
        chunk = _chunk(nodes=[
            {"id": 1, "callFrame": {"functionName": "(root)"}},
            {"id": 2, "parent": 1, "callFrame": {"functionName": "hot"}},
            {"id": 3, "parent": 1, "callFrame": {"functionName": "(program)"}},
        ], samples=[3])
        with self.assertRaisesRegex(SmokeError, "did not sample.*hot"):
            _assert_cpu_profile_events([_profile(), chunk], {"hot"})

    def test_sampled_descendant_attributes_its_parent_function(self) -> None:
        chunk = _chunk(nodes=[
            {"id": 1, "callFrame": {"functionName": "(root)"}},
            {"id": 2, "parent": 1, "callFrame": {"functionName": "hot"}},
            {"id": 3, "parent": 2, "callFrame": {"functionName": "helper"}},
        ], samples=[3, 3])
        summary = _assert_cpu_profile_events([_profile(), chunk], {"hot"})
        self.assertEqual(summary.function_samples, {"hot": 2})

    def test_nodes_from_later_chunks_and_children_edges_are_retained(self) -> None:
        events = [_profile(),
                  _chunk(nodes=[{"id": 1, "children": [2], "callFrame": {"functionName": "hot"}}]),
                  _chunk(nodes=[{"id": 2, "callFrame": {"functionName": "helper"}}], samples=[])]
        self.assertEqual(_assert_cpu_profile_events(events, {"hot"}).function_samples, {"hot": 1})

    def test_unknown_or_cyclic_sample_stacks_fail_with_profile_identity(self) -> None:
        for chunk in [_chunk(samples=[99]),
                      _chunk(nodes=[{"id": 2, "parent": 2, "callFrame": {"functionName": "hot"}}])]:
            with self.subTest(chunk=chunk), self.assertRaisesRegex(SmokeError, "invalid sample stack profile="):
                _assert_cpu_profile_events([_profile(), chunk], {"hot"})

    def test_missing_functions_and_misaligned_samples_still_fail(self) -> None:
        with self.assertRaisesRegex(SmokeError, "did not report named hot functions"):
            _assert_cpu_profile_events([_profile(), _chunk()], {"missing"})
        chunk = _chunk()
        chunk["args"]["data"]["timeDeltas"] = []
        with self.assertRaisesRegex(SmokeError, "equal lengths"):
            _assert_cpu_profile_events([_profile(), chunk], {"hot"})


if __name__ == "__main__":
    unittest.main()
