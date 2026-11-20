from __future__ import annotations

import io
import os
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

INSTALLER = Path(__file__).with_name("install.sh")


class InstallerDestinationTests(unittest.TestCase):
    def run_installer(self, root: Path) -> subprocess.CompletedProcess[str]:
        tools = root / "tools"
        tools.mkdir()
        archive = root / "release.tar.gz"
        binary = b"#!/bin/sh\nprintf 'fixture version\\n'\n"
        with tarfile.open(archive, "w:gz") as output:
            entry = tarfile.TarInfo("moli-fixture/moli")
            entry.size = len(binary)
            entry.mode = 0o755
            output.addfile(entry, io.BytesIO(binary))
        (tools / "curl").write_text(
            '#!/bin/sh\nwhile [ "$#" -gt 0 ]; do\n'
            '  if [ "$1" = -o ]; then cp "$FIXTURE_ARCHIVE" "$2"; exit; fi\n'
            "  shift\ndone\nexit 1\n"
        )
        (tools / "uname").write_text(
            '#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo x86_64;; *) exit 1;; esac\n'
        )
        for tool in tools.iterdir():
            tool.chmod(0o755)
        env = dict(
            os.environ,
            PATH=str(tools) + os.pathsep + os.environ["PATH"],
            HOME=str(root / "home"),
            TMPDIR=str(root),
            MOLI_INSTALL_DIR=str(root / "bin"),
            FIXTURE_ARCHIVE=str(archive),
            MOLI_RELEASE_BASE_URL="https://fixture.invalid/release",
        )
        return subprocess.run(
            ["sh", str(INSTALLER)], env=env, capture_output=True, text=True, timeout=10
        )

    def test_directory_destination_is_rejected_without_mutation(self) -> None:
        for symlink in (False, True):
            with (
                self.subTest(symlink=symlink),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                destination = root / "bin" / "moli"
                destination.parent.mkdir()
                contents = root / "existing-directory" if symlink else destination
                contents.mkdir()
                if symlink:
                    destination.symlink_to(contents, target_is_directory=True)
                sentinel = contents / "keep"
                sentinel.write_text("original")
                result = self.run_installer(root)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertNotIn("Installed moli", result.stdout)
                self.assertEqual(sentinel.read_text(), "original")
                self.assertEqual(list(contents.iterdir()), [sentinel])
                self.assertEqual(destination.is_symlink(), symlink)
                self.assertEqual(list(destination.parent.glob(".moli.install.*")), [])

    def test_fresh_install_and_existing_binary_upgrade(self) -> None:
        for existing in (False, True):
            with (
                self.subTest(existing=existing),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                destination = root / "bin" / "moli"
                if existing:
                    destination.parent.mkdir()
                    destination.write_text("old version")
                result = self.run_installer(root)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(destination.is_file())
                self.assertTrue(os.access(destination, os.X_OK))
                check = subprocess.run(
                    [str(destination)], capture_output=True, text=True, check=True
                )
                self.assertEqual(check.stdout, "fixture version\n")


if __name__ == "__main__":
    unittest.main()
