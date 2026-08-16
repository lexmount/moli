#!/usr/bin/env python3
"""Build npm launcher and native platform packages from Moli release archives."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent
MANIFEST_PATH = REPO_ROOT / "moli" / "Cargo.toml"
NPM_SOURCE_DIR = REPO_ROOT / "npm"
PLATFORMS_PATH = NPM_SOURCE_DIR / "platforms.json"
PACKAGE_NAME = "@lexmount/moli"
SEMVER_PATTERN = re.compile(
    r"^(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


class NpmPackageError(RuntimeError):
    """An npm package input or build step was invalid."""


@dataclass(frozen=True)
class PlatformDefinition:
    id: str
    platform: str
    arch: str
    target: str
    package: str
    archive: str
    binary: str
    libc: tuple[str, ...] = ()


def normalize_version(raw_version: str) -> str:
    version = raw_version.removeprefix("v")
    if not SEMVER_PATTERN.fullmatch(version):
        raise NpmPackageError(f"invalid semantic version: {raw_version}")
    return version


def manifest_version() -> str:
    with MANIFEST_PATH.open("rb") as manifest_file:
        manifest = tomllib.load(manifest_file)
    try:
        version = manifest["package"]["version"]
    except (KeyError, TypeError) as error:
        raise NpmPackageError(
            f"package.version is missing from {MANIFEST_PATH}"
        ) from error
    if not isinstance(version, str):
        raise NpmPackageError(
            f"package.version in {MANIFEST_PATH} is not a string"
        )
    return version


def resolve_repo_path(raw_path: str) -> Path:
    path = Path(raw_path).expanduser()
    if not path.is_absolute():
        path = REPO_ROOT / path
    return path.resolve()


def load_platforms() -> list[PlatformDefinition]:
    try:
        raw_platforms = json.loads(PLATFORMS_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise NpmPackageError(f"could not read {PLATFORMS_PATH}: {error}") from error
    if not isinstance(raw_platforms, list) or not raw_platforms:
        raise NpmPackageError(f"{PLATFORMS_PATH} must contain a non-empty array")

    platforms: list[PlatformDefinition] = []
    for index, raw in enumerate(raw_platforms):
        if not isinstance(raw, dict):
            raise NpmPackageError(f"platform entry {index} must be an object")
        raw_libc = raw.get("libc", [])
        if not isinstance(raw_libc, list):
            raise NpmPackageError(f"platform entry {index} libc must be an array")
        try:
            definition = PlatformDefinition(
                id=raw["id"],
                platform=raw["platform"],
                arch=raw["arch"],
                target=raw["target"],
                package=raw["package"],
                archive=raw["archive"],
                binary=raw["binary"],
                libc=tuple(raw_libc),
            )
        except (KeyError, TypeError) as error:
            raise NpmPackageError(f"invalid platform entry {index}: {error}") from error
        values = (
            definition.id,
            definition.platform,
            definition.arch,
            definition.target,
            definition.package,
            definition.archive,
            definition.binary,
            *definition.libc,
        )
        if not all(isinstance(value, str) and value for value in values):
            raise NpmPackageError(
                f"platform entry {index} contains an empty or non-string value"
            )
        expected_package = f"{PACKAGE_NAME}-{definition.id}"
        if definition.package != expected_package:
            raise NpmPackageError(
                f"platform entry {index} package must be {expected_package}"
            )
        platforms.append(definition)

    for field in ("id", "target", "package", "archive"):
        values = [getattr(platform, field) for platform in platforms]
        if len(values) != len(set(values)):
            raise NpmPackageError(f"platform definitions contain duplicate {field} values")
    node_platforms = [(platform.platform, platform.arch) for platform in platforms]
    if len(node_platforms) != len(set(node_platforms)):
        raise NpmPackageError(
            "platform definitions contain duplicate platform/architecture pairs"
        )
    return platforms


def platform_version(version: str, platform_id: str) -> str:
    core, separator, build = version.partition("+")
    variant = f"{core}-{platform_id}"
    if separator:
        variant = f"{variant}+{build}"
    if not SEMVER_PATTERN.fullmatch(variant):
        raise NpmPackageError(f"invalid npm platform version: {variant}")
    return variant


def repository_metadata() -> dict[str, str]:
    return {
        "type": "git",
        "url": "git+https://github.com/lexmount/moli.git",
    }


def copy_project_file(destination: Path, relative_path: str) -> None:
    source = REPO_ROOT / relative_path
    if not source.is_file():
        raise NpmPackageError(f"required npm package file is missing: {source}")
    shutil.copy2(source, destination / source.name)


def copy_package_metadata(destination: Path, *, third_party: bool) -> None:
    for relative_path in ("README.md", "LICENSE", "LICENSE-APACHE", "LICENSE-MIT"):
        copy_project_file(destination, relative_path)
    if not third_party:
        return

    notices_root = destination / "third_party"
    notices_root.mkdir()
    for name in ("licenses", "notices"):
        source = REPO_ROOT / "third_party" / name
        if not source.is_dir():
            raise NpmPackageError(
                f"required third-party license directory is missing: {source}"
            )
        shutil.copytree(source, notices_root / name)


def write_json(path: Path, value: Any) -> None:
    path.write_text(
        json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )


def main_package_manifest(
    version: str, platforms: list[PlatformDefinition]
) -> dict[str, Any]:
    optional_dependencies = {
        platform.package: f"npm:{PACKAGE_NAME}@{platform_version(version, platform.id)}"
        for platform in platforms
    }
    return {
        "name": PACKAGE_NAME,
        "version": version,
        "description": (
            "A structured-first headless browser engine for AI agents"
        ),
        "license": "MIT OR Apache-2.0",
        "type": "module",
        "bin": {"moli": "bin/moli.js"},
        "engines": {"node": ">=18"},
        "files": [
            "bin",
            "lib",
            "platforms.json",
            "README.md",
            "LICENSE",
            "LICENSE-APACHE",
            "LICENSE-MIT",
        ],
        "repository": repository_metadata(),
        "homepage": "https://github.com/lexmount/moli",
        "bugs": {"url": "https://github.com/lexmount/moli/issues"},
        "publishConfig": {"access": "public"},
        "optionalDependencies": optional_dependencies,
    }


def platform_package_manifest(
    version: str, definition: PlatformDefinition
) -> dict[str, Any]:
    manifest: dict[str, Any] = {
        "name": PACKAGE_NAME,
        "version": platform_version(version, definition.id),
        "description": f"Moli native binary for {definition.id}",
        "license": "MIT OR Apache-2.0",
        "engines": {"node": ">=18"},
        "os": [definition.platform],
        "cpu": [definition.arch],
        "files": [
            "vendor",
            "README.md",
            "LICENSE",
            "LICENSE-APACHE",
            "LICENSE-MIT",
            "third_party",
        ],
        "repository": repository_metadata(),
        "homepage": "https://github.com/lexmount/moli",
        "bugs": {"url": "https://github.com/lexmount/moli/issues"},
        "publishConfig": {"access": "public"},
    }
    if definition.libc:
        manifest["libc"] = list(definition.libc)
    return manifest


def extract_platform_binary(
    archive_path: Path,
    version: str,
    definition: PlatformDefinition,
    destination: Path,
) -> None:
    package_root = f"moli-v{version}-{definition.target}"
    binary_member = f"{package_root}/{definition.binary}"
    version_member = f"{package_root}/VERSION"
    try:
        if archive_path.suffix == ".zip":
            with zipfile.ZipFile(archive_path, mode="r") as archive:
                archived_version = archive.read(version_member)
                with archive.open(binary_member, mode="r") as source, destination.open(
                    "wb"
                ) as output:
                    shutil.copyfileobj(source, output)
        else:
            with tarfile.open(archive_path, mode="r:gz") as archive:
                version_entry = archive.getmember(version_member)
                binary_entry = archive.getmember(binary_member)
                if not version_entry.isfile() or not binary_entry.isfile():
                    raise NpmPackageError(
                        f"release archive contains a non-file package member: {archive_path}"
                    )
                version_source = archive.extractfile(version_entry)
                binary_source = archive.extractfile(binary_entry)
                if version_source is None or binary_source is None:
                    raise NpmPackageError(
                        f"could not read package members from {archive_path}"
                    )
                with version_source:
                    archived_version = version_source.read()
                with binary_source, destination.open("wb") as output:
                    shutil.copyfileobj(binary_source, output)
    except (KeyError, OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        raise NpmPackageError(f"could not unpack {archive_path}: {error}") from error

    try:
        decoded_version = archived_version.decode("utf-8").strip()
    except UnicodeDecodeError as error:
        raise NpmPackageError(
            f"release archive VERSION is not UTF-8: {archive_path}"
        ) from error
    if decoded_version != version:
        raise NpmPackageError(
            f"release archive version mismatch for {archive_path.name}: "
            f"expected {version}, got {decoded_version or '<empty>'}"
        )
    destination.chmod(0o644 if definition.platform == "win32" else 0o755)


def package_integrity(path: Path) -> str:
    digest = hashlib.sha512()
    with path.open("rb") as package_file:
        for chunk in iter(lambda: package_file.read(1024 * 1024), b""):
            digest.update(chunk)
    return "sha512-" + base64.b64encode(digest.digest()).decode("ascii")


def npm_pack(package_dir: Path, output_dir: Path) -> dict[str, Any]:
    command = [
        "npm",
        "pack",
        "--json",
        "--pack-destination",
        str(output_dir),
    ]
    try:
        result = subprocess.run(
            command,
            cwd=package_dir,
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except FileNotFoundError as error:
        raise NpmPackageError("npm is required to build npm packages") from error
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() or error.stdout.strip()
        raise NpmPackageError(f"npm pack failed: {detail}") from error

    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise NpmPackageError(f"npm pack returned invalid JSON: {error}") from error
    if not isinstance(payload, list) or len(payload) != 1 or not isinstance(payload[0], dict):
        raise NpmPackageError("npm pack did not describe exactly one package")

    packed = payload[0]
    required_fields = (
        "name",
        "version",
        "filename",
        "integrity",
        "shasum",
        "size",
        "unpackedSize",
        "files",
    )
    if any(field not in packed for field in required_fields):
        raise NpmPackageError("npm pack output is missing required metadata")
    for field in ("name", "version", "filename", "integrity", "shasum"):
        if not isinstance(packed[field], str) or not packed[field]:
            raise NpmPackageError(f"npm pack output contains invalid {field}")
    if Path(packed["filename"]).name != packed["filename"]:
        raise NpmPackageError("npm pack output filename must not contain a path")
    tarball = output_dir / packed["filename"]
    if not tarball.is_file():
        raise NpmPackageError(f"npm pack did not create {tarball}")
    actual_integrity = package_integrity(tarball)
    if packed["integrity"] != actual_integrity:
        raise NpmPackageError(
            f"npm package integrity mismatch for {tarball.name}: "
            f"expected {packed['integrity']}, got {actual_integrity}"
        )
    return packed


def packed_file_paths(packed: dict[str, Any]) -> set[str]:
    files = packed["files"]
    if not isinstance(files, list):
        raise NpmPackageError("npm pack file metadata must be an array")
    paths: set[str] = set()
    for file in files:
        if not isinstance(file, dict) or not isinstance(file.get("path"), str):
            raise NpmPackageError("npm pack returned invalid file metadata")
        paths.add(file["path"])
    return paths


def compact_pack_metadata(packed: dict[str, Any]) -> dict[str, Any]:
    return {
        field: packed[field]
        for field in (
            "name",
            "version",
            "filename",
            "integrity",
            "shasum",
            "size",
            "unpackedSize",
        )
    }


def build_main_package(
    stage: Path,
    output_dir: Path,
    version: str,
    platforms: list[PlatformDefinition],
) -> dict[str, Any]:
    shutil.copytree(NPM_SOURCE_DIR / "bin", stage / "bin")
    shutil.copytree(NPM_SOURCE_DIR / "lib", stage / "lib")
    shutil.copy2(PLATFORMS_PATH, stage / "platforms.json")
    (stage / "bin" / "moli.js").chmod(0o755)
    copy_package_metadata(stage, third_party=False)
    write_json(stage / "package.json", main_package_manifest(version, platforms))

    packed = npm_pack(stage, output_dir)
    if packed["name"] != PACKAGE_NAME or packed["version"] != version:
        raise NpmPackageError("npm packed an unexpected main package identity")
    expected_files = {
        "bin/moli.js",
        "lib/platform.js",
        "platforms.json",
        "package.json",
    }
    missing = expected_files - packed_file_paths(packed)
    if missing:
        raise NpmPackageError(
            f"main npm package is missing files: {', '.join(sorted(missing))}"
        )
    return compact_pack_metadata(packed)


def build_platform_package(
    stage: Path,
    output_dir: Path,
    input_dir: Path,
    version: str,
    definition: PlatformDefinition,
) -> dict[str, Any]:
    archive_path = input_dir / definition.archive
    if not archive_path.is_file():
        raise NpmPackageError(f"required release archive is missing: {archive_path}")

    binary = stage / "vendor" / definition.target / "bin" / definition.binary
    binary.parent.mkdir(parents=True)
    extract_platform_binary(archive_path, version, definition, binary)
    copy_package_metadata(stage, third_party=True)
    write_json(stage / "package.json", platform_package_manifest(version, definition))

    packed = npm_pack(stage, output_dir)
    expected_version = platform_version(version, definition.id)
    if packed["name"] != PACKAGE_NAME or packed["version"] != expected_version:
        raise NpmPackageError(
            f"npm packed an unexpected package identity for {definition.id}"
        )
    expected_binary = f"vendor/{definition.target}/bin/{definition.binary}"
    if expected_binary not in packed_file_paths(packed):
        raise NpmPackageError(
            f"platform npm package is missing binary: {expected_binary}"
        )
    metadata = compact_pack_metadata(packed)
    metadata.update(
        {
            "alias": definition.package,
            "target": definition.target,
            "distTag": definition.id,
        }
    )
    return metadata


def build_packages(
    *, version: str, input_dir: Path, output_dir: Path
) -> dict[str, Any]:
    declared_version = manifest_version()
    if version != declared_version:
        raise NpmPackageError(
            f"requested version {version} does not match "
            f"moli/Cargo.toml ({declared_version})"
        )
    if not input_dir.is_dir():
        raise NpmPackageError(f"release input directory does not exist: {input_dir}")
    if output_dir.exists():
        raise NpmPackageError(f"npm output path already exists: {output_dir}")
    output_dir.parent.mkdir(parents=True, exist_ok=True)

    platforms = load_platforms()
    for definition in platforms:
        archive_path = input_dir / definition.archive
        if not archive_path.is_file():
            raise NpmPackageError(f"required release archive is missing: {archive_path}")

    with tempfile.TemporaryDirectory(
        prefix=".moli-npm-", dir=output_dir.parent
    ) as temporary:
        work_dir = Path(temporary)
        packed_dir = work_dir / "packed"
        packed_dir.mkdir()

        platform_packages: list[dict[str, Any]] = []
        for definition in platforms:
            with tempfile.TemporaryDirectory(
                prefix=f"{definition.id}-", dir=work_dir
            ) as platform_stage:
                platform_packages.append(
                    build_platform_package(
                        Path(platform_stage),
                        packed_dir,
                        input_dir,
                        version,
                        definition,
                    )
                )

        with tempfile.TemporaryDirectory(prefix="main-", dir=work_dir) as main_stage:
            main_package = build_main_package(
                Path(main_stage), packed_dir, version, platforms
            )

        release_manifest = {
            "schemaVersion": 1,
            "package": PACKAGE_NAME,
            "version": version,
            "main": main_package,
            "platforms": platform_packages,
        }
        write_json(packed_dir / "npm-packages.json", release_manifest)
        packed_dir.rename(output_dir)
    return release_manifest


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="release version, with optional v")
    parser.add_argument(
        "--input-dir",
        default="dist",
        help="directory containing native release archives (default: dist)",
    )
    parser.add_argument(
        "--output-dir",
        default="dist/npm",
        help="new directory for npm tarballs (default: dist/npm)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        version = normalize_version(args.version)
        output_dir = resolve_repo_path(args.output_dir)
        manifest = build_packages(
            version=version,
            input_dir=resolve_repo_path(args.input_dir),
            output_dir=output_dir,
        )
        print(f"Created npm package set for {manifest['package']}@{version}")
        for platform in manifest["platforms"]:
            print(f"Created: {output_dir / platform['filename']}")
        print(f"Created: {output_dir / manifest['main']['filename']}")
        print(f"Created: {output_dir / 'npm-packages.json'}")
        return 0
    except NpmPackageError as error:
        print(f"npm package error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
