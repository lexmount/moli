from __future__ import annotations

import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPTS_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS_DIR))

import release  # noqa: E402


class ReleaseVersionTests(unittest.TestCase):
    def test_reads_standard_cli_version_flag(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[], returncode=0, stdout="moli 1.2.3\n", stderr=""
        )
        with patch.object(release, "run_checked", return_value=completed) as run:
            version = release.binary_reported_version(Path("/tmp/moli"))

        self.assertEqual(version, "1.2.3")
        run.assert_called_once_with(["/tmp/moli", "--version"], capture_output=True)

    def test_rejects_an_unexpected_version_response(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[], returncode=0, stdout="1.2.3\n", stderr=""
        )
        with patch.object(release, "run_checked", return_value=completed):
            with self.assertRaisesRegex(
                release.ReleaseError, "unexpected `--version` response"
            ):
                release.binary_reported_version(Path("/tmp/moli"))


if __name__ == "__main__":
    unittest.main()
