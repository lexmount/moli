from __future__ import annotations

import io
import json
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest
import zipfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
PACKAGE_SCRIPT = REPO_ROOT / "scripts" / "package_npm.py"
PUBLISH_SCRIPT = REPO_ROOT / "scripts" / "publish_npm.py"
PLATFORMS = json.loads(
    (REPO_ROOT / "npm" / "platforms.json").read_text(encoding="utf-8")
)


def current_version() -> str:
    with (REPO_ROOT / "moli" / "Cargo.toml").open("rb") as manifest_file:
        return tomllib.load(manifest_file)["package"]["version"]


def add_tar_bytes(archive: tarfile.TarFile, name: str, value: bytes, mode: int) -> None:
    member = tarfile.TarInfo(name)
    member.size = len(value)
    member.mode = mode
    archive.addfile(member, io.BytesIO(value))


def write_release_archive(directory: Path, platform: dict[str, object], version: str) -> None:
    target = str(platform["target"])
    binary = str(platform["binary"])
    archive_path = directory / str(platform["archive"])
    package_root = f"moli-v{version}-{target}"
    binary_contents = f"fixture binary for {target}\n".encode()
    if archive_path.suffix == ".zip":
        with zipfile.ZipFile(archive_path, mode="w") as archive:
            archive.writestr(f"{package_root}/VERSION", f"{version}\n")
            archive.writestr(f"{package_root}/{binary}", binary_contents)
        return

    with tarfile.open(archive_path, mode="w:gz") as archive:
        add_tar_bytes(
            archive,
            f"{package_root}/VERSION",
            f"{version}\n".encode(),
            0o644,
        )
        add_tar_bytes(
            archive,
            f"{package_root}/{binary}",
            binary_contents,
            0o755,
        )


def read_tgz_json(path: Path, member: str) -> dict[str, object]:
    with tarfile.open(path, mode="r:gz") as archive:
        source = archive.extractfile(member)
        if source is None:
            raise AssertionError(f"missing {member} in {path}")
        return json.load(source)


class PackageNpmTests(unittest.TestCase):
    def test_builds_launcher_and_one_native_package_per_supported_platform(self) -> None:
        version = current_version()
        with tempfile.TemporaryDirectory(prefix="moli-npm-package-test-") as raw:
            root = Path(raw)
            input_dir = root / "release"
            output_dir = root / "npm"
            input_dir.mkdir()
            for platform in PLATFORMS:
                write_release_archive(input_dir, platform, version)

            result = subprocess.run(
                [
                    sys.executable,
                    str(PACKAGE_SCRIPT),
                    "--version",
                    version,
                    "--input-dir",
                    str(input_dir),
                    "--output-dir",
                    str(output_dir),
                ],
                cwd=REPO_ROOT,
                check=False,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

            package_set = json.loads(
                (output_dir / "npm-packages.json").read_text(encoding="utf-8")
            )
            self.assertEqual(package_set["package"], "@lexmount/moli")
            self.assertEqual(package_set["version"], version)
            self.assertEqual(len(package_set["platforms"]), len(PLATFORMS))

            main_tarball = output_dir / package_set["main"]["filename"]
            main_manifest = read_tgz_json(main_tarball, "package/package.json")
            self.assertEqual(main_manifest["version"], version)
            self.assertEqual(main_manifest["bin"], {"moli": "bin/moli.js"})
            expected_aliases = {
                package["alias"]: (
                    f"npm:@lexmount/moli@{package['version']}"
                )
                for package in package_set["platforms"]
            }
            self.assertEqual(
                main_manifest["optionalDependencies"], expected_aliases
            )
            with tarfile.open(main_tarball, mode="r:gz") as archive:
                launcher = archive.getmember("package/bin/moli.js")
                self.assertNotEqual(launcher.mode & 0o111, 0)

            platform_by_target = {
                str(platform["target"]): platform for platform in PLATFORMS
            }
            for package in package_set["platforms"]:
                platform = platform_by_target[package["target"]]
                tarball = output_dir / package["filename"]
                manifest = read_tgz_json(tarball, "package/package.json")
                self.assertEqual(manifest["name"], "@lexmount/moli")
                self.assertEqual(manifest["os"], [platform["platform"]])
                self.assertEqual(manifest["cpu"], [platform["arch"]])
                if "libc" in platform:
                    self.assertEqual(manifest["libc"], platform["libc"])

                binary_member = (
                    f"package/vendor/{platform['target']}/bin/{platform['binary']}"
                )
                with tarfile.open(tarball, mode="r:gz") as archive:
                    member = archive.getmember(binary_member)
                    source = archive.extractfile(member)
                    self.assertIsNotNone(source)
                    assert source is not None
                    self.assertEqual(
                        source.read(),
                        f"fixture binary for {platform['target']}\n".encode(),
                    )
                    if platform["platform"] != "win32":
                        self.assertNotEqual(member.mode & 0o111, 0)

            publish_dry_run = subprocess.run(
                [
                    sys.executable,
                    str(PUBLISH_SCRIPT),
                    str(output_dir / "npm-packages.json"),
                    "--main-tag",
                    "latest",
                    "--dry-run",
                ],
                cwd=REPO_ROOT,
                check=False,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            self.assertEqual(publish_dry_run.returncode, 0, publish_dry_run.stderr)

    def test_rejects_an_incomplete_release_before_creating_output(self) -> None:
        version = current_version()
        with tempfile.TemporaryDirectory(prefix="moli-npm-package-test-") as raw:
            root = Path(raw)
            input_dir = root / "release"
            output_dir = root / "npm"
            input_dir.mkdir()
            for platform in PLATFORMS[:-1]:
                write_release_archive(input_dir, platform, version)

            result = subprocess.run(
                [
                    sys.executable,
                    str(PACKAGE_SCRIPT),
                    "--version",
                    version,
                    "--input-dir",
                    str(input_dir),
                    "--output-dir",
                    str(output_dir),
                ],
                cwd=REPO_ROOT,
                check=False,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("required release archive is missing", result.stderr)
            self.assertFalse(output_dir.exists())

    def test_refuses_to_overwrite_an_existing_output_path(self) -> None:
        version = current_version()
        with tempfile.TemporaryDirectory(prefix="moli-npm-package-test-") as raw:
            root = Path(raw)
            input_dir = root / "release"
            output_dir = root / "npm"
            input_dir.mkdir()
            output_dir.mkdir()
            for platform in PLATFORMS:
                write_release_archive(input_dir, platform, version)

            result = subprocess.run(
                [
                    sys.executable,
                    str(PACKAGE_SCRIPT),
                    "--version",
                    version,
                    "--input-dir",
                    str(input_dir),
                    "--output-dir",
                    str(output_dir),
                ],
                cwd=REPO_ROOT,
                check=False,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("npm output path already exists", result.stderr)


if __name__ == "__main__":
    unittest.main()
