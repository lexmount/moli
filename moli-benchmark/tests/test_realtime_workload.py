from __future__ import annotations

import hashlib
import json
import unittest

from moli_benchmark.realtime_workload import PHASES, fixture, validate_container


class RealtimeWorkloadTests(unittest.TestCase):
    def test_original_block_fixtures_remain_byte_identical(self):
        # Captured from the committed 3dd188c786 workload, before adding containers.
        goldens = [
            (
                False,
                False,
                "ef2e66b15e75f24c679f54730c86e29785e28715ca47b24cd5c63de0642d5b07",
            ),
            (
                True,
                False,
                "190c46aad58d884cd7923b823e8f2f030c066f8fc8269b94dc5224796b8bd895",
            ),
            (
                False,
                True,
                "d1a1b7335c74d2a63bb8e1039dd9aed0b703ceb99bc5058140561319aaafc5c5",
            ),
            (
                True,
                True,
                "6d0a435370f7be57872b5253a19b311f86890303e6cb72b3902e65ad6ef66cfe",
            ),
        ]
        for positioned, font, expected in goldens:
            with self.subTest(positioned=positioned, font=font):
                self.assertEqual(
                    hashlib.sha256(fixture(1024, positioned, font)).hexdigest(),
                    expected,
                )
                self.assertEqual(
                    fixture(1024, positioned, font),
                    fixture(1024, positioned, font, container="blocks"),
                )

    def test_original_measured_tasks_remain_byte_identical(self):
        self.assertEqual(
            hashlib.sha256(json.dumps(PHASES, sort_keys=True).encode()).hexdigest(),
            "5727a8c26538fff10fe9c5dbafc4dbb19156c331fbcbed7268cf3c2a89c5eed4",
        )

    def test_container_observation_rejects_the_wrong_layout(self):
        value = {"display": "grid", "width": 1280, "items": 64, "rows": 0, "cells": 0}
        validate_container(value, "grid", 64)
        with self.assertRaises(AssertionError):
            validate_container(dict(value, display="block"), "grid", 64)
        with self.assertRaises(AssertionError):
            validate_container(dict(value, width=100), "grid", 64)
        validate_container(dict(value, width=100), "grid", 64, real_geometry=False)
        with self.assertRaises(AssertionError):
            validate_container(
                {
                    "display": "table",
                    "width": 1280,
                    "items": 65,
                    "rows": 16,
                    "cells": 65,
                },
                "table",
                65,
            )

    def test_unknown_container_cannot_silently_run_the_block_case(self):
        with self.assertRaises(ValueError):
            fixture(64, container="unknown")
