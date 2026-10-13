from __future__ import annotations

import asyncio
import errno
import io
import subprocess
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from moli_benchmark import sampling, target_serve


class TargetStartFailureEvidenceTests(unittest.TestCase):
    def run_start_failure(
        self, stage, *, fail_release=False, startup_failure=None, term_timeout=False
    ):
        failure = startup_failure or RuntimeError(f"initial {stage} failure")
        calls = []
        process = SimpleNamespace(
            pid=10, returncode=None, stdout=io.BytesIO(), stderr=io.BytesIO()
        )
        process.poll = lambda: process.returncode

        def terminate():
            calls.append("terminate")
            if not term_timeout:
                process.returncode = -15

        def wait(timeout):
            calls.append("wait")
            if process.returncode is None:
                raise subprocess.TimeoutExpired("fake-engine", timeout)
            return process.returncode

        def kill():
            calls.append("kill")
            process.returncode = -9

        def release():
            calls.append("release_port")
            if fail_release:
                raise OSError(errno.EIO, "port release failed")

        process.terminate, process.wait, process.kill = terminate, wait, kill
        lease = SimpleNamespace(
            port=23456,
            release_socket=lambda: calls.append("release_socket"),
            close=release,
        )

        class Thread:
            def __init__(self, *, target, args=(), daemon, name=None):
                self.target, self.args = target, args
                self.label = args[1] if args else "sampler"
                self.started = False

            def start(self):
                calls.append(f"start_{self.label}")
                if stage == f"start_{self.label}":
                    raise failure
                self.started = True
                # Only fake stdout/stderr BytesIO readers may run. Never sample.
                if self.label == "sampler":
                    raise AssertionError("test must fail before starting a sampler")
                self.target(*self.args)

            def join(self, timeout):
                calls.append(f"join_{self.label}")
                if not self.started:
                    raise RuntimeError("cannot join thread before it is started")

            def is_alive(self):
                return False

        class Sampler(sampling.ResourceSampler):
            def stop(self):
                calls.append("stop_sampler")
                return super().stop()

        def sampler_factory(pid, **kwargs):
            calls.append("create_sampler")
            if stage == "create_sampler":
                raise failure
            return Sampler(pid, **kwargs)

        with (
            patch.object(target_serve, "reserve_port", return_value=lease),
            patch.object(
                target_serve,
                "_stat_read",
                return_value={"identity": "10:100", "state": "S"},
            ),
            patch.object(target_serve.subprocess, "Popen", return_value=process),
            patch.object(target_serve.threading, "Thread", Thread),
            patch.object(target_serve, "ResourceSampler", side_effect=sampler_factory),
            patch.object(
                sampling, "clock_ticks_evidence", return_value={"ticks_per_second": 100}
            ),
            patch.object(
                target_serve,
                "probe_url",
                side_effect=AssertionError("readiness must not run"),
            ),
            self.assertRaises(type(failure)) as caught,
        ):
            target_serve.start_target_serve("moli", Path("/never-executed"), 10)
        self.assertIs(caught.exception, failure)
        evidence = failure.evidence
        self.assertEqual(evidence["startup_error"]["message"], str(failure))
        self.assertIn("terminate", calls)
        self.assertIn("release_port", calls)
        self.assertLess(calls.index("terminate"), calls.index("release_port"))
        self.assertFalse(evidence["stop_protocol_verified"])
        self.assertFalse(evidence["sampler_sealed_before_termination"])
        self.assertFalse(evidence["cleanup_complete"])
        return failure, calls, evidence

    def test_sampler_start_failure_records_unstarted_join_but_terminates_and_releases(
        self,
    ):
        _failure, calls, evidence = self.run_start_failure("start_sampler")
        self.assertEqual(evidence["startup_stage"], "start_sampler")
        self.assertTrue(evidence["sampler_constructed"])
        self.assertEqual(calls.count("stop_sampler"), 2)
        self.assertLess(calls.index("stop_sampler"), calls.index("terminate"))
        self.assertEqual(evidence["resources"]["samples"], [])
        self.assertEqual(evidence["returncode"], -15)
        self.assertEqual(
            [item["action"] for item in evidence["errors"]],
            ["stop_and_seal_sampler", "sampler_join_after_process_cleanup"],
        )
        self.assertTrue(
            all(item["type"] == "RuntimeError" for item in evidence["errors"])
        )

    def test_partial_log_start_failure_records_missing_eof_and_does_not_invent_sampler(
        self,
    ):
        _failure, calls, evidence = self.run_start_failure("start_stderr")
        self.assertEqual(evidence["startup_stage"], "start_stderr_reader")
        self.assertFalse(evidence["sampler_constructed"])
        self.assertNotIn("create_sampler", calls)
        self.assertIsNone(evidence["resources"]["samples"])
        self.assertTrue(evidence["log_readers"][0]["eof"])
        self.assertNotIn("eof", evidence["log_readers"][1])
        self.assertEqual(evidence["errors"][0]["action"], "join_stderr")
        self.assertIn("cannot join", evidence["errors"][0]["message"])

    def test_sampler_constructor_failure_still_cleans_engine_and_both_readers(self):
        _failure, calls, evidence = self.run_start_failure("create_sampler")
        self.assertEqual(evidence["startup_stage"], "create_sampler")
        self.assertNotIn("stop_sampler", calls)
        self.assertFalse(evidence["sampler_constructed"])
        self.assertIsNone(evidence["resources"]["samples"])
        self.assertTrue(all(reader["eof"] for reader in evidence["log_readers"]))

    def test_cleanup_errors_and_term_timeout_do_not_mask_initial_failure(self):
        _failure, calls, evidence = self.run_start_failure(
            "start_sampler", fail_release=True, term_timeout=True
        )
        self.assertIn("kill", calls)
        self.assertEqual(evidence["returncode"], -9)
        self.assertEqual(evidence["errors"][-1]["action"], "release_port")
        self.assertEqual(evidence["errors"][-1]["errno"], errno.EIO)
        self.assertEqual(
            evidence["termination"]["actions"][1]["error"]["type"], "TimeoutExpired"
        )

    def test_startup_cancellation_keeps_same_object_with_cleanup_evidence(self):
        cancellation = asyncio.CancelledError("startup canceled")
        failure, _calls, evidence = self.run_start_failure(
            "start_sampler", startup_failure=cancellation
        )
        self.assertIs(failure, cancellation)
        self.assertEqual(evidence["startup_error"]["type"], "CancelledError")
        self.assertEqual(evidence["returncode"], -15)

    def test_handle_construction_failure_uses_existing_term_policy_and_records_cleanup_failure(
        self,
    ):
        failure = RuntimeError("cannot initialize handle")
        process = SimpleNamespace(
            pid=10, poll=Mock(side_effect=OSError(errno.EIO, "poll failed"))
        )
        lease = SimpleNamespace(
            port=23456,
            release_socket=Mock(),
            close=Mock(side_effect=OSError(errno.EIO, "close failed")),
        )
        with (
            patch.object(target_serve, "reserve_port", return_value=lease),
            patch.object(target_serve.subprocess, "Popen", return_value=process),
            patch.object(target_serve, "TargetServeHandle", side_effect=failure),
            patch.object(
                target_serve,
                "_terminate_process",
                return_value={"exited": True, "returncode": -15},
            ) as terminate,
            self.assertRaises(RuntimeError) as caught,
        ):
            target_serve.start_target_serve("moli", Path("/never-executed"), 10)
        self.assertIs(caught.exception, failure)
        terminate.assert_called_once_with(process)
        lease.close.assert_called_once()
        self.assertEqual(failure.evidence["startup_stage"], "create_handle")
        self.assertEqual(
            [item["action"] for item in failure.evidence["errors"]],
            ["release_port", "root_returncode"],
        )
        self.assertIsNone(failure.evidence["returncode"])
        self.assertFalse(failure.evidence["cleanup_complete"])


if __name__ == "__main__":
    unittest.main()
