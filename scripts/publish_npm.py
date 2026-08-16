#!/usr/bin/env python3
"""Publish a verified Moli npm package set, with the launcher published last."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import shlex
import subprocess
import sys
import tarfile
from pathlib import Path
from typing import Any


DIST_TAG_PATTERN = re.compile(r"^[A-Za-z][A-Za-z0-9._-]*$")
TOOL_VERSION_PATTERN = re.compile(
    r"^v?(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z.-]+)?$"
)
MINIMUM_TRUSTED_NPM_VERSION = (11, 5, 1)
MINIMUM_TRUSTED_NODE_VERSION = (22, 14, 0)
MAXIMUM_PACKAGE_JSON_SIZE = 1024 * 1024


class NpmPublishError(RuntimeError):
    """An npm package set was invalid or could not be published safely."""


def package_integrity(path: Path) -> str:
    digest = hashlib.sha512()
    with path.open("rb") as package_file:
        for chunk in iter(lambda: package_file.read(1024 * 1024), b""):
            digest.update(chunk)
    return "sha512-" + base64.b64encode(digest.digest()).decode("ascii")


def load_manifest(path: Path) -> dict[str, Any]:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise NpmPublishError(
            f"could not read npm package manifest {path}: {error}"
        ) from error
    if not isinstance(manifest, dict) or manifest.get("schemaVersion") != 1:
        raise NpmPublishError("unsupported npm package manifest schema")
    if not isinstance(manifest.get("package"), str) or not isinstance(
        manifest.get("version"), str
    ):
        raise NpmPublishError("npm package manifest is missing package identity")
    if not isinstance(manifest.get("main"), dict) or not isinstance(
        manifest.get("platforms"), list
    ):
        raise NpmPublishError("npm package manifest is missing package entries")
    if not manifest["platforms"]:
        raise NpmPublishError("npm package manifest contains no platform packages")
    return manifest


def validate_dist_tag(tag: str) -> str:
    if not DIST_TAG_PATTERN.fullmatch(tag):
        raise NpmPublishError(f"invalid npm dist-tag: {tag}")
    return tag


def tool_version(command: str) -> tuple[int, int, int]:
    try:
        result = subprocess.run(
            [command, "--version"],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except FileNotFoundError as error:
        raise NpmPublishError(
            f"{command} is required to publish npm packages"
        ) from error
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() or error.stdout.strip()
        raise NpmPublishError(
            f"could not determine {command} version: {detail}"
        ) from error

    raw_version = result.stdout.strip()
    match = TOOL_VERSION_PATTERN.fullmatch(raw_version)
    if match is None:
        raise NpmPublishError(f"could not parse {command} version: {raw_version}")
    return tuple(int(component) for component in match.groups())


def require_trusted_publishing_toolchain() -> None:
    requirements = (
        ("node", MINIMUM_TRUSTED_NODE_VERSION),
        ("npm", MINIMUM_TRUSTED_NPM_VERSION),
    )
    for command, minimum in requirements:
        actual = tool_version(command)
        if actual < minimum:
            minimum_display = ".".join(str(component) for component in minimum)
            actual_display = ".".join(str(component) for component in actual)
            raise NpmPublishError(
                f"npm trusted publishing requires {command} >= {minimum_display}; "
                f"found {actual_display}"
            )


def validate_package_entry(
    entry: dict[str, Any], package_dir: Path
) -> tuple[Path, str, str]:
    for field in ("name", "version", "filename", "integrity"):
        if not isinstance(entry.get(field), str) or not entry[field]:
            raise NpmPublishError(f"npm package entry has invalid {field}")
    filename = entry["filename"]
    if Path(filename).name != filename:
        raise NpmPublishError(
            f"npm package filename must not contain a path: {filename}"
        )
    tarball = package_dir / filename
    if not tarball.is_file():
        raise NpmPublishError(f"npm package tarball is missing: {tarball}")
    actual_integrity = package_integrity(tarball)
    if actual_integrity != entry["integrity"]:
        raise NpmPublishError(
            f"npm package integrity mismatch for {filename}: "
            f"expected {entry['integrity']}, got {actual_integrity}"
        )

    try:
        with tarfile.open(tarball, mode="r:gz") as archive:
            package_json_entry = archive.getmember("package/package.json")
            if (
                not package_json_entry.isfile()
                or package_json_entry.size > MAXIMUM_PACKAGE_JSON_SIZE
            ):
                raise NpmPublishError(
                    f"npm package has an invalid package/package.json: {filename}"
                )
            package_json = archive.extractfile(package_json_entry)
            if package_json is None:
                raise NpmPublishError(
                    f"npm package is missing package/package.json: {filename}"
                )
            with package_json:
                identity = json.load(package_json)
    except (
        KeyError,
        OSError,
        UnicodeDecodeError,
        tarfile.TarError,
        json.JSONDecodeError,
    ) as error:
        raise NpmPublishError(
            f"could not read npm package identity from {filename}: {error}"
        ) from error
    if not isinstance(identity, dict):
        raise NpmPublishError(f"npm package identity is invalid in {filename}")
    if (
        identity.get("name") != entry["name"]
        or identity.get("version") != entry["version"]
    ):
        raise NpmPublishError(
            f"npm package identity mismatch for {filename}: expected "
            f"{entry['name']}@{entry['version']}"
        )
    return tarball, entry["name"], entry["version"]


def npm_view_integrity(name: str, version: str) -> str | None:
    try:
        result = subprocess.run(
            ["npm", "view", f"{name}@{version}", "dist.integrity", "--json"],
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except FileNotFoundError as error:
        raise NpmPublishError("npm is required to publish npm packages") from error
    if result.returncode != 0:
        if "E404" in result.stderr or "E404" in result.stdout:
            return None
        detail = result.stderr.strip() or result.stdout.strip()
        raise NpmPublishError(f"npm view failed for {name}@{version}: {detail}")
    try:
        integrity = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise NpmPublishError(
            f"npm view returned invalid JSON for {name}@{version}: {error}"
        ) from error
    if not isinstance(integrity, str) or not integrity:
        raise NpmPublishError(f"npm view returned no integrity for {name}@{version}")
    return integrity


def run_checked(command: list[str], description: str) -> None:
    print("+ " + shlex.join(command), flush=True)
    try:
        subprocess.run(command, check=True)
    except FileNotFoundError as error:
        raise NpmPublishError("npm is required to publish npm packages") from error
    except subprocess.CalledProcessError as error:
        raise NpmPublishError(
            f"{description} failed with exit code {error.returncode}"
        ) from error


def publish_package(
    entry: dict[str, Any],
    package_dir: Path,
    *,
    dist_tag: str,
    dry_run: bool,
) -> None:
    tarball, name, version = validate_package_entry(entry, package_dir)
    dist_tag = validate_dist_tag(dist_tag)

    if dry_run:
        command = [
            "npm",
            "publish",
            str(tarball),
            "--access",
            "public",
            "--tag",
            dist_tag,
            "--dry-run",
        ]
        run_checked(command, f"npm publish dry run for {name}@{version}")
        return

    published_integrity = npm_view_integrity(name, version)
    if published_integrity is not None:
        if published_integrity != entry["integrity"]:
            raise NpmPublishError(
                f"refusing to reuse {name}@{version}: registry integrity differs"
            )
        print(f"Already published with matching integrity: {name}@{version}")
    else:
        command = [
            "npm",
            "publish",
            str(tarball),
            "--access",
            "public",
            "--tag",
            dist_tag,
        ]
        run_checked(command, f"npm publish for {name}@{version}")


def publish_package_set(
    manifest_path: Path,
    *,
    main_tag: str,
    trusted_publishing: bool,
    dry_run: bool,
) -> None:
    if trusted_publishing and not dry_run:
        require_trusted_publishing_toolchain()

    manifest = load_manifest(manifest_path)
    package_dir = manifest_path.parent
    package_name = manifest["package"]
    version = manifest["version"]

    platform_entries: list[tuple[dict[str, Any], str]] = []
    for entry in manifest["platforms"]:
        if not isinstance(entry, dict):
            raise NpmPublishError("npm platform package entry must be an object")
        if entry.get("name") != package_name:
            raise NpmPublishError("npm platform package name does not match package set")
        dist_tag = entry.get("distTag")
        if not isinstance(dist_tag, str):
            raise NpmPublishError("npm platform package is missing its dist-tag")
        validate_dist_tag(dist_tag)
        validate_package_entry(entry, package_dir)
        platform_entries.append((entry, dist_tag))

    platform_versions = [entry["version"] for entry, _ in platform_entries]
    if len(platform_versions) != len(set(platform_versions)):
        raise NpmPublishError("npm platform package versions must be unique")

    main = manifest["main"]
    if main.get("name") != package_name or main.get("version") != version:
        raise NpmPublishError("main npm package identity does not match package set")
    validate_dist_tag(main_tag)
    validate_package_entry(main, package_dir)

    for entry, dist_tag in platform_entries:
        publish_package(
            entry,
            package_dir,
            dist_tag=dist_tag,
            dry_run=dry_run,
        )

    publish_package(
        main,
        package_dir,
        dist_tag=main_tag,
        dry_run=dry_run,
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", help="path to npm-packages.json")
    parser.add_argument(
        "--main-tag",
        default="latest",
        help="dist-tag for the launcher package (default: latest)",
    )
    parser.add_argument(
        "--trusted-publishing",
        action="store_true",
        help="require a supported Node/npm OIDC toolchain before publishing",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="validate every tarball with npm publish --dry-run",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        publish_package_set(
            Path(args.manifest).resolve(),
            main_tag=args.main_tag,
            trusted_publishing=args.trusted_publishing,
            dry_run=args.dry_run,
        )
        return 0
    except NpmPublishError as error:
        print(f"npm publish error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
