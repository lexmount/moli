from __future__ import annotations

import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "check_linux_compat.py"
spec = importlib.util.spec_from_file_location("check_linux_compat", SCRIPT)
compat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compat)


def needs(*names: str) -> str:
    return "Version needs section '.gnu.version_r' contains 2 entries:\n" + "\n".join(
        f"  0x0010: Name: {name} Flags: none Version: 2" for name in names
    )


class LinuxCompatibilityTests(unittest.TestCase):
    def test_accepts_jammy_limits(self) -> None:
        self.assertEqual(
            compat.check_versions(
                needs("GLIBC_2.35", "GLIBCXX_3.4.30", "CXXABI_1.3.13")
            ),
            compat.MAX_VERSIONS,
        )

    def test_compares_versions_numerically(self) -> None:
        self.assertEqual(
            compat.check_versions(needs("GLIBC_2.9", "GLIBC_2.34", "GLIBCXX_3.4.9")),
            {"GLIBC": (2, 34), "GLIBCXX": (3, 4, 9)},
        )

    def test_rejects_newer_abi_in_each_family(self) -> None:
        for name in ["GLIBC_2.36", "GLIBC_2.38", "GLIBCXX_3.4.31", "CXXABI_1.3.14"]:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, name):
                compat.check_versions(needs("GLIBC_2.17", name))

    def test_rejects_relr_and_private_abi(self) -> None:
        for name in ["GLIBC_ABI_DT_RELR", "GLIBC_PRIVATE", "CXXABI_UNKNOWN"]:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, name):
                compat.check_versions(needs("GLIBC_2.17", name))

    def test_accepts_jammy_named_cxx_abis(self) -> None:
        compat.check_versions(needs("GLIBC_2.17", "CXXABI_TM_1", "CXXABI_FLOAT128"))

    def test_ignores_version_definitions(self) -> None:
        output = "Version definition section:\nName: GLIBC_9.99\n" + needs("GLIBC_2.35")
        self.assertEqual(compat.check_versions(output), {"GLIBC": (2, 35)})

    def test_fails_closed_without_glibc_requirements(self) -> None:
        for output in [
            "",
            "No version information found in this file.",
            needs("GCC_3.0"),
        ]:
            with self.subTest(output=output), self.assertRaises(ValueError):
                compat.check_versions(output)

    def test_smoke_requires_version_and_javascript_execution(self) -> None:
        with patch.object(
            compat,
            "run_checked",
            side_effect=[
                "moli 1.0.0\n",
                '{"title":"jammy-smoke","text":"script-ran","answer":42}',
            ],
        ) as run:
            compat.smoke_test(Path("/moli"))
        self.assertEqual(run.call_count, 2)
        self.assertEqual(run.call_args_list[0].args[0], ["/moli", "--version"])
        self.assertIn("--eval", run.call_args_list[1].args[0])

    def test_smoke_rejects_incorrect_result(self) -> None:
        with (
            patch.object(compat, "run_checked", side_effect=["moli 1.0.0", "{}"]),
            self.assertRaisesRegex(ValueError, "unexpected output"),
        ):
            compat.smoke_test(Path("/moli"))

    def test_smoke_propagates_loader_failure(self) -> None:
        with (
            patch.object(
                compat,
                "run_checked",
                side_effect=subprocess.CalledProcessError(1, "moli"),
            ),
            self.assertRaises(subprocess.CalledProcessError),
        ):
            compat.smoke_test(Path("/moli"))


if __name__ == "__main__":
    unittest.main()
