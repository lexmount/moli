from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.targets import collect_target_binaries
from moli_benchmark.versions import collect_versions


class VersionsTests(unittest.TestCase):
    def test_collect_versions_uses_moli_dash_version(self) -> None:
        commands: list[list[str]] = []

        def run(command: list[str], timeout: float = 5.0) -> str:
            del timeout
            commands.append(command)
            return "moli 1.0.2"

        with tempfile.TemporaryDirectory() as temp_dir:
            binary = Path(temp_dir) / "moli"
            binary.write_bytes(b"test binary")
            with patch("moli_benchmark.versions._run", side_effect=run):
                versions = collect_versions(binary)

        self.assertIn([str(binary), "--version"], commands)
        self.assertNotIn([str(binary), "version"], commands)
        self.assertEqual(versions["moli"]["version"], "moli 1.0.2")

    def test_target_metadata_uses_moli_dash_version(self) -> None:
        moli = Path("/tmp/test-moli")

        with (
            patch("moli_benchmark.targets.moli_binary", return_value=moli),
            patch("moli_benchmark.targets.lightpanda_binary", return_value=None),
            patch("moli_benchmark.targets.chrome_binary", return_value=None),
            patch("moli_benchmark.targets.obscura_binary", return_value=None),
            patch(
                "moli_benchmark.targets._binary_info",
                side_effect=lambda path, args: {"path": path, "version_args": args},
            ) as binary_info,
        ):
            targets = collect_target_binaries()

        self.assertEqual(binary_info.call_args_list[0].args, (moli, ("--version",)))
        self.assertEqual(targets["moli"]["version_args"], ("--version",))


if __name__ == "__main__":
    unittest.main()
