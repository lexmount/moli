from __future__ import annotations

import base64
import hashlib
import json
import subprocess
import sys
import tarfile
import tempfile
import unittest
from io import BytesIO
from pathlib import Path
from unittest.mock import patch


SCRIPTS_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS_DIR))

import publish_npm  # noqa: E402


def integrity(value: bytes) -> str:
    digest = hashlib.sha512(value).digest()
    return "sha512-" + base64.b64encode(digest).decode("ascii")


def write_package(path: Path, name: str, version: str) -> None:
    package_json = json.dumps({"name": name, "version": version}).encode()
    member = tarfile.TarInfo("package/package.json")
    member.size = len(package_json)
    with tarfile.open(path, mode="w:gz") as archive:
        archive.addfile(member, BytesIO(package_json))


class PublishNpmTests(unittest.TestCase):
    def test_publishes_all_platform_packages_before_the_launcher(self) -> None:
        manifest = {
            "schemaVersion": 1,
            "package": "@lexmount/moli",
            "version": "1.0.0",
            "platforms": [
                {
                    "name": "@lexmount/moli",
                    "version": "1.0.0-linux-x64",
                    "filename": "linux.tgz",
                    "integrity": "unused",
                    "distTag": "linux-x64",
                },
                {
                    "name": "@lexmount/moli",
                    "version": "1.0.0-darwin-arm64",
                    "filename": "darwin.tgz",
                    "integrity": "unused",
                    "distTag": "darwin-arm64",
                },
            ],
            "main": {
                "name": "@lexmount/moli",
                "version": "1.0.0",
                "filename": "main.tgz",
                "integrity": "unused",
            },
        }
        with tempfile.TemporaryDirectory(prefix="moli-npm-publish-test-") as raw:
            package_dir = Path(raw)
            for entry in [*manifest["platforms"], manifest["main"]]:
                tarball = package_dir / entry["filename"]
                write_package(tarball, entry["name"], entry["version"])
                entry["integrity"] = publish_npm.package_integrity(tarball)
            manifest_path = package_dir / "npm-packages.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            calls: list[tuple[str, str]] = []

            def record(entry, _package_dir, *, dist_tag, dry_run):
                self.assertFalse(dry_run)
                calls.append((entry["version"], dist_tag))

            with (
                patch.object(
                    publish_npm, "require_trusted_publishing_toolchain"
                ) as require_toolchain,
                patch.object(publish_npm, "publish_package", side_effect=record),
            ):
                publish_npm.publish_package_set(
                    manifest_path,
                    main_tag="latest",
                    trusted_publishing=True,
                    dry_run=False,
                )
            require_toolchain.assert_called_once_with()

        self.assertEqual(
            calls,
            [
                ("1.0.0-linux-x64", "linux-x64"),
                ("1.0.0-darwin-arm64", "darwin-arm64"),
                ("1.0.0", "latest"),
            ],
        )

    def test_rejects_changed_tarball_contents(self) -> None:
        original = b"original package"
        with tempfile.TemporaryDirectory(prefix="moli-npm-publish-test-") as raw:
            package_dir = Path(raw)
            tarball = package_dir / "package.tgz"
            tarball.write_bytes(b"changed package")
            entry = {
                "name": "@lexmount/moli",
                "version": "1.0.0",
                "filename": tarball.name,
                "integrity": integrity(original),
            }
            with self.assertRaisesRegex(
                publish_npm.NpmPublishError, "integrity mismatch"
            ):
                publish_npm.validate_package_entry(entry, package_dir)

    def test_rejects_tarball_paths_outside_the_package_directory(self) -> None:
        with tempfile.TemporaryDirectory(prefix="moli-npm-publish-test-") as raw:
            entry = {
                "name": "@lexmount/moli",
                "version": "1.0.0",
                "filename": "../package.tgz",
                "integrity": "unused",
            }
            with self.assertRaisesRegex(
                publish_npm.NpmPublishError, "must not contain a path"
            ):
                publish_npm.validate_package_entry(entry, Path(raw))

    def test_rejects_package_identity_mismatch(self) -> None:
        with tempfile.TemporaryDirectory(prefix="moli-npm-publish-test-") as raw:
            package_dir = Path(raw)
            tarball = package_dir / "package.tgz"
            write_package(tarball, "@lexmount/not-moli", "1.0.0")
            entry = {
                "name": "@lexmount/moli",
                "version": "1.0.0",
                "filename": tarball.name,
                "integrity": publish_npm.package_integrity(tarball),
            }
            with self.assertRaisesRegex(
                publish_npm.NpmPublishError, "identity mismatch"
            ):
                publish_npm.validate_package_entry(entry, package_dir)

    def test_trusted_publishing_rejects_old_npm(self) -> None:
        versions = {"node": (24, 15, 0), "npm": (11, 5, 0)}
        with (
            patch.object(publish_npm, "tool_version", side_effect=versions.__getitem__),
            self.assertRaisesRegex(
                publish_npm.NpmPublishError,
                r"npm trusted publishing requires npm >= 11\.5\.1; found 11\.5\.0",
            ),
        ):
            publish_npm.require_trusted_publishing_toolchain()

    def test_trusted_publishing_rejects_old_node(self) -> None:
        versions = {"node": (22, 13, 9), "npm": (11, 18, 0)}
        with (
            patch.object(publish_npm, "tool_version", side_effect=versions.__getitem__),
            self.assertRaisesRegex(
                publish_npm.NpmPublishError,
                r"npm trusted publishing requires node >= 22\.14\.0; found 22\.13\.9",
            ),
        ):
            publish_npm.require_trusted_publishing_toolchain()

    def test_tool_version_accepts_node_prefix_and_prerelease(self) -> None:
        completed = subprocess.CompletedProcess(
            ["node", "--version"], 0, stdout="v24.15.0-rc.1\n", stderr=""
        )
        with patch.object(publish_npm.subprocess, "run", return_value=completed):
            self.assertEqual(publish_npm.tool_version("node"), (24, 15, 0))


if __name__ == "__main__":
    unittest.main()
