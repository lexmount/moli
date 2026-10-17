from __future__ import annotations

import asyncio
import errno
import io
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from moli_benchmark import sampling, target_serve


def stat(pid=10, tid=None, start=100, ticks=5):
    return {
        "status": "ok",
        "identity": f"{pid}:{start}" if tid is None else f"{pid}:{tid}:{start}",
        "pid": pid,
        "tid": tid,
        "start_ticks": start,
        "state": "S",
        "ppid": 1,
        "user_ticks": ticks,
        "system_ticks": 0,
        "total_ticks": ticks,
    }


def cpu(threads, ticks=5, identity="10:100"):
    root = {**stat(ticks=ticks), "identity": identity}
    return {
        "threads": threads,
        "observations": [{"valid": True}],
        "enumeration_error": None,
        "root_before": root,
        "root_after": root,
    }


class InstrumentationEvidenceTests(unittest.TestCase):
    def test_resource_error_keeps_errno_and_missing_is_not_zero(self):
        with patch.object(
            sampling.Path,
            "read_text",
            side_effect=OSError(errno.EACCES, "denied", "/proc/10/status"),
        ):
            read = sampling._text_read(sampling.Path("/proc/10/status"))
        self.assertEqual(read["error"]["errno"], errno.EACCES)
        self.assertEqual(sampling._field_read(read, "VmRSS:")["status"], "os_error")
        self.assertEqual(
            sampling._field_read({"status": "ok", "raw": "Pss: 0 kB"}, "Pss:", 1024)[
                "value"
            ],
            0,
        )
        self.assertEqual(
            sampling._field_read({"status": "ok", "raw": "Rss: 1 kB"}, "Pss:")[
                "status"
            ],
            "missing_field",
        )
        self.assertEqual(
            sampling._field_read({"status": "ok", "raw": "Pss: -1 kB"}, "Pss:")[
                "status"
            ],
            "parse_error",
        )

    def test_stat_keeps_raw_and_uses_last_comm_parenthesis(self):
        fields = ["S", "1"] + ["0"] * 18
        fields[11], fields[12], fields[19] = "7", "9", "123"
        raw = "10 (name with ) spaces) " + " ".join(fields)
        with patch.object(sampling.Path, "read_text", return_value=raw):
            value = sampling._stat_read(10)
        self.assertEqual(value["raw"], raw)
        self.assertEqual(value["identity"], "10:123")
        self.assertEqual(value["total_ticks"], 16)
        with patch.object(
            sampling.Path, "read_text", return_value=raw.replace("10 (", "11 (", 1)
        ):
            self.assertEqual(sampling._stat_read(10)["status"], "parse_error")

    def test_changed_root_identity_invalidates_process_total_and_thread_delta(self):
        result = sampling.cpu_delta(
            cpu({"10:7:100": 100}),
            cpu({"10:7:100": 200}, identity="10:200"),
            {"ticks_per_second": 100},
            "10:100",
        )
        self.assertIsNone(result["cpu_ms"])
        self.assertIsNone(result["process_cpu"]["delta_ticks"])

    def test_tid_reuse_and_retired_threads_do_not_corrupt_independent_process_total(
        self,
    ):
        before = cpu({"10:7:100": 1_000_000}, ticks=10)
        after = cpu({"10:7:200": 200_000}, ticks=13)
        result = sampling.cpu_delta(before, after, {"ticks_per_second": 100}, "10:100")
        self.assertEqual(result["retired_threads"], ["10:7:100"])
        self.assertEqual(result["created_threads"], ["10:7:200"])
        self.assertEqual(result["cpu_ms"], 0.2)
        self.assertEqual(result["process_cpu"]["delta_ticks"], 3)
        self.assertEqual(result["process_cpu"]["cpu_ms"], 30)
        self.assertEqual(result["thread_cpu_observed"]["before"], before)
        self.assertEqual(result["thread_cpu_observed"]["after"], after)

    def test_incomplete_thread_map_never_silently_substitutes_process_cpu(self):
        before, after = cpu({"10:7:100": 1}), cpu({}, ticks=6)
        after["observations"] = [{"valid": False}]
        result = sampling.cpu_delta(before, after, {"ticks_per_second": 100}, "10:100")
        self.assertIsNone(result["cpu_ms"])
        self.assertEqual(result["process_cpu"]["cpu_ms"], 10)
        self.assertFalse(result["thread_cpu_observed"]["boundary_read_complete"])

    def test_regression_is_not_clamped_and_clock_failure_is_not_100hz(self):
        value = sampling.cpu_delta(
            cpu({"10:7:100": 20}),
            cpu({"10:7:100": 5}, ticks=4),
            {"ticks_per_second": 100},
            "10:100",
        )
        self.assertIsNone(value["cpu_ms"])
        self.assertIsNone(value["process_cpu"]["delta_ticks"])
        self.assertEqual(
            value["thread_cpu_observed"]["counter_regressions"], ["10:7:100"]
        )
        with patch.object(
            sampling.os, "sysconf", side_effect=OSError(errno.EINVAL, "bad clock")
        ):
            clock = sampling.clock_ticks_evidence()
        self.assertIsNone(clock["ticks_per_second"])
        self.assertEqual(clock["error"]["errno"], errno.EINVAL)
        value = sampling.cpu_delta(cpu({}), cpu({}, ticks=6), clock, "10:100")
        self.assertIsNone(value["process_cpu"]["cpu_ms"])
        self.assertEqual(value["process_cpu"]["delta_ticks"], 1)

    def test_zero_ticks_is_not_claimed_as_zero_execution_or_ns_precision(self):
        value = sampling.cpu_delta(
            cpu({"10:7:100": 100}),
            cpu({"10:7:100": 200}),
            {"ticks_per_second": 100, "counter_tick_ms": 10},
            "10:100",
        )
        self.assertEqual(value["process_cpu"]["cpu_ms"], 0)
        self.assertEqual(value["cpu_ms"], 0.0001)
        self.assertEqual(
            value["process_cpu"]["zero_ticks_mean"],
            "no_exported_tick_change_not_zero_cpu",
        )
        self.assertEqual(
            value["process_cpu"]["remote_process_clock"]["status"],
            "unsupported_not_runtime_validated",
        )

    def test_affinity_has_actual_values_identity_and_errno(self):
        task = SimpleNamespace(name="7")
        with (
            patch.object(sampling.Path, "iterdir", return_value=[task]),
            patch.object(
                sampling, "_stat_read", side_effect=lambda pid, tid=None: stat(pid, tid)
            ),
            patch.object(sampling.os, "sched_getaffinity", return_value={0}),
        ):
            result = sampling.affinity_snapshot(10, 0, "10:100")
        self.assertTrue(result["all_observed_match"])
        self.assertEqual(result["threads"][0]["before"]["identity"], "10:7:100")
        with (
            patch.object(sampling.Path, "iterdir", return_value=[task]),
            patch.object(
                sampling, "_stat_read", side_effect=lambda pid, tid=None: stat(pid, tid)
            ),
            patch.object(
                sampling.os,
                "sched_getaffinity",
                side_effect=OSError(errno.ESRCH, "gone"),
            ),
        ):
            result = sampling.affinity_snapshot(10, 0, "10:100")
        self.assertFalse(result["boundary_complete"])
        self.assertEqual(result["threads"][0]["error"]["errno"], errno.ESRCH)

    def test_cpu_snapshot_retains_disappeared_thread_observation(self):
        tasks = [SimpleNamespace(name="7"), SimpleNamespace(name="8")]

        def read(path):
            if str(path).endswith("8/schedstat"):
                return {
                    "status": "os_error",
                    "raw": None,
                    "error": {"errno": errno.ENOENT},
                }
            return {"status": "ok", "raw": "123 0 0"}

        with (
            patch.object(sampling.Path, "iterdir", return_value=tasks),
            patch.object(
                sampling, "_stat_read", side_effect=lambda pid, tid=None: stat(pid, tid)
            ),
            patch.object(sampling, "_text_read", side_effect=read),
        ):
            result = sampling.cpu_snapshot(10)
        self.assertEqual(result["enumerated_tids"], [7, 8])
        self.assertEqual(result["threads"], {"10:7:100": 123})
        self.assertFalse(result["observations"][1]["valid"])
        self.assertEqual(
            result["observations"][1]["runtime"]["error"]["errno"], errno.ENOENT
        )

    def test_sampler_seals_after_inflight_sample_and_snapshots_are_independent(self):
        entered, release = threading.Event(), threading.Event()

        def snapshot(*_args, **_kwargs):
            entered.set()
            self.assertTrue(release.wait(1))
            return {
                "processes": [{"identity_valid": True, "identity_after": stat()}],
                "process_count": 1,
                "pss_bytes": 3,
                "pss_process_count": 1,
                "rss_bytes": 4,
                "thread_count": 1,
                "fd_count": 1,
            }

        sampler = sampling.ResourceSampler(10, interval_seconds=0.01)
        with (
            patch.object(sampling, "snapshot_resources", side_effect=snapshot),
            patch.object(
                sampling, "clock_ticks_evidence", return_value={"ticks_per_second": 100}
            ),
        ):
            sampler.start()
            self.assertTrue(entered.wait(1))
            sampler._stop.set()
            release.set()
            summary = sampler.stop()
        self.assertTrue(summary["sealed"])
        self.assertFalse(summary["thread_alive_after_stop"])
        self.assertEqual(summary["sample_count"], 1)
        sample = sampler.export_samples()
        sample[0]["pss_bytes"] = 999
        self.assertEqual(sampler.export_samples()[0]["pss_bytes"], 3)
        self.assertLessEqual(
            summary["last_committed_monotonic_ns"],
            summary["thread_exited_monotonic_ns"],
        )
        self.assertLessEqual(
            summary["thread_exited_monotonic_ns"], summary["join_finished_monotonic_ns"]
        )

    def test_join_timeout_remains_a_failure_after_later_success(self):
        sampler = sampling.ResourceSampler(10)
        thread = Mock()
        thread.is_alive.return_value = True
        sampler._thread = thread
        first = sampler.stop()
        self.assertFalse(first["sealed"])
        self.assertTrue(first["join_ever_timed_out"])
        thread.is_alive.return_value = False
        last = sampler.stop()
        self.assertTrue(last["sealed"])
        self.assertTrue(last["join_ever_timed_out"])

    def test_log_read_errors_and_eof_are_distinct(self):
        state, logs = {}, []
        target_serve._read_available(io.BytesIO(b"a\nb\n"), "stdout", logs, state)
        self.assertTrue(state["eof"])
        self.assertEqual(state["lines_read"], 2)
        target_serve._read_available(
            SimpleNamespace(
                readline=Mock(side_effect=OSError(errno.EIO, "read failed"))
            ),
            "stderr",
            [],
            state := {},
        )
        self.assertFalse(state["eof"])
        self.assertEqual(state["error"]["errno"], errno.EIO)

    def test_stop_seals_before_term_and_logs_require_eof(self):
        calls = []
        process = SimpleNamespace(
            pid=10, returncode=None, poll=lambda: process.returncode
        )

        def stop():
            calls.append("stop")
            self.assertIsNone(process.returncode)
            return {
                "sealed": True,
                "thread_alive_after_stop": False,
                "join_ever_timed_out": False,
                "pss_complete": True,
            }

        def terminate(_):
            calls.append("terminate")
            process.returncode = 143
            return {"exited": True, "returncode": 143}

        thread = SimpleNamespace(join=lambda timeout: None, is_alive=lambda: False)
        handle = SimpleNamespace(
            process=process,
            root_identity=stat(),
            sampler=SimpleNamespace(stop=stop, export_samples=list),
            logs=[],
            log_threads=[thread, thread],
            log_state={
                name: {"eof": True, "error": None, "lines_read": 0}
                for name in ("stdout", "stderr")
            },
            port_lease=SimpleNamespace(close=lambda: calls.append("release")),
            temp_dir=None,
        )
        with (
            patch.object(target_serve, "_stat_read", return_value=stat()),
            patch.object(target_serve, "_terminate_process", side_effect=terminate),
        ):
            result = target_serve.stop_target_serve(
                handle, include_resource_samples=True
            )
        self.assertEqual(calls, ["stop", "terminate", "release"])
        self.assertTrue(result["stop_protocol_verified"])
        self.assertTrue(result["cleanup_complete"])
        self.assertEqual(result["returncode"], 143)
        self.assertEqual(result["descendants_exited"], "unknown")

    def test_async_close_failure_does_not_skip_stop_or_overwrite_workload_error(self):
        async def failed_close():
            raise OSError(errno.EIO, "socket close failed")

        record = {"ok": False, "error": "original workload failure"}
        with patch.object(
            target_serve, "stop_target_serve", return_value={"cleanup_complete": True}
        ) as stop:
            asyncio.run(
                target_serve.finish_target_serve_run(
                    SimpleNamespace(websocket=SimpleNamespace(close=failed_close)),
                    object(),
                    record,
                )
            )
        stop.assert_called_once()
        self.assertEqual(record["error"], "original workload failure")
        self.assertEqual(record["cleanup_errors"][0]["errno"], errno.EIO)
        self.assertFalse(record["ok"])
        self.assertFalse(record["safe_to_continue"])

    def test_unobserved_tree_root_stays_in_selected_set(self):
        failed = {
            "status": "os_error",
            "identity": None,
            "error": {"errno": errno.ENOENT},
        }
        with (
            patch.object(sampling.Path, "glob", return_value=[]),
            patch.object(sampling, "_stat_read", return_value=failed),
        ):
            pids, records, evidence = sampling._process_tree_evidence(10)
        self.assertEqual(pids, [10])
        self.assertEqual(records[10], failed)
        self.assertEqual(evidence["all_descendants_observed"], "unknown")

    def test_resource_identity_change_prevents_apparently_complete_totals(self):
        with (
            patch.object(
                sampling,
                "_process_tree_evidence",
                return_value=([10], {10: stat()}, {}),
            ),
            patch.object(sampling, "_stat_read", return_value=stat(start=200)),
            patch.object(
                sampling,
                "_text_read",
                return_value={
                    "status": "ok",
                    "raw": "Pss: 1 kB\nVmRSS: 1 kB\nThreads: 1",
                },
            ),
            patch.object(
                sampling, "_fd_read", return_value={"status": "ok", "value": 1}
            ),
        ):
            value = sampling.snapshot_resources(10, include_lifetime_cpu=False)
        self.assertEqual(value["process_count"], 1)
        self.assertIsNone(value["pss_bytes"])
        self.assertEqual(value["pss_process_count"], 0)
        self.assertFalse(value["processes"][0]["identity_valid"])

    def test_readiness_failure_carries_stop_and_eof_evidence(self):
        process = SimpleNamespace(
            pid=10, returncode=None, stdout=io.BytesIO(), stderr=io.BytesIO()
        )
        process.poll = lambda: process.returncode
        process.terminate = lambda: setattr(process, "returncode", 143)
        process.wait = lambda timeout: process.returncode

        class Thread:
            def __init__(self, *, target, args, daemon):
                self.target, self.args = target, args

            def start(self):
                self.target(*self.args)

            def join(self, timeout):
                pass

            def is_alive(self):
                return False

        sampler = SimpleNamespace(
            start=lambda: None,
            stop=lambda: {
                "sealed": True,
                "thread_alive_after_stop": False,
                "pss_complete": False,
            },
            export_samples=list,
        )
        lease = SimpleNamespace(port=23456, release_socket=Mock(), close=Mock())
        with (
            patch.object(target_serve, "reserve_port", return_value=lease),
            patch.object(target_serve, "_stat_read", return_value=stat()),
            patch.object(target_serve.subprocess, "Popen", return_value=process),
            patch.object(target_serve.threading, "Thread", Thread),
            patch.object(target_serve, "ResourceSampler", return_value=sampler),
            self.assertRaises(target_serve.TargetServeReadinessTimeout) as caught,
        ):
            target_serve.start_target_serve("moli", Path("/unused"), 0)
        value = caught.exception.evidence
        self.assertTrue(value["sampler_sealed_before_termination"])
        self.assertTrue(value["cleanup_complete"])
        self.assertEqual(value["returncode"], 143)
        self.assertEqual(len(value["log_readers"]), 2)
        self.assertEqual(value["resources"]["samples"], [])
        lease.close.assert_called_once()

    def test_fixture_cleanup_joins_even_when_close_fails(self):
        calls = []

        def failed():
            calls.append("close")
            raise OSError(errno.EIO, "close")

        value = target_serve.stop_fixture_server(
            SimpleNamespace(
                shutdown=lambda: calls.append("shutdown"), server_close=failed
            ),
            SimpleNamespace(
                join=lambda timeout: calls.append("join"), is_alive=lambda: False
            ),
        )
        self.assertEqual(calls, ["shutdown", "close", "join"])
        self.assertFalse(value["complete"])
        self.assertEqual(value["errors"][0]["errno"], errno.EIO)


if __name__ == "__main__":
    unittest.main()
