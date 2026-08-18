#!/usr/bin/env python3
"""Build, strip, verify, and package a native Moli release artifact."""

from __future__ import annotations

import argparse
import os
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile
from pathlib import Path
from typing import Sequence


REPO_ROOT = Path(__file__).resolve().parent.parent
MANIFEST_PATH = REPO_ROOT / "moli" / "Cargo.toml"
SEMVER_PATTERN = re.compile(
    r"^(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
TARGET_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+$")


class ReleaseError(RuntimeError):
    """A release precondition or command failed."""


def display_command(command: Sequence[str]) -> str:
    return shlex.join(str(part) for part in command)


def run_checked(
    command: Sequence[str], *, capture_output: bool = False
) -> subprocess.CompletedProcess[str]:
    rendered = display_command(command)
    print(f"+ {rendered}", flush=True)
    try:
        return subprocess.run(
            [str(part) for part in command],
            cwd=REPO_ROOT,
            check=True,
            text=True,
            stdout=subprocess.PIPE if capture_output else None,
            stderr=subprocess.PIPE if capture_output else None,
        )
    except FileNotFoundError as error:
        raise ReleaseError(f"command not found: {command[0]}") from error
    except subprocess.CalledProcessError as error:
        if error.stdout:
            print(error.stdout, end="", file=sys.stderr)
        if error.stderr:
            print(error.stderr, end="", file=sys.stderr)
        raise ReleaseError(
            f"command failed with exit code {error.returncode}: {rendered}"
        ) from error


def normalize_version(raw_version: str) -> str:
    version = raw_version.removeprefix("v")
    if not SEMVER_PATTERN.fullmatch(version):
        raise ReleaseError(f"invalid semantic version: {raw_version}")
    return version


def manifest_version() -> str:
    with MANIFEST_PATH.open("rb") as manifest_file:
        manifest = tomllib.load(manifest_file)
    try:
        value = manifest["package"]["version"]
    except (KeyError, TypeError) as error:
        raise ReleaseError(f"package.version is missing from {MANIFEST_PATH}") from error
    if not isinstance(value, str):
        raise ReleaseError(f"package.version in {MANIFEST_PATH} is not a string")
    return value


def rust_host_target() -> str:
    output = run_checked(["rustc", "-vV"], capture_output=True).stdout
    for line in output.splitlines():
        if line.startswith("host: "):
            target = line.removeprefix("host: ").strip()
            if not TARGET_PATTERN.fullmatch(target):
                break
            return target
    raise ReleaseError("could not determine the Rust host target from `rustc -vV`")


def resolve_repo_path(raw_path: str) -> Path:
    path = Path(raw_path).expanduser()
    if not path.is_absolute():
        path = REPO_ROOT / path
    return path.resolve()


def default_binary_path(target: str) -> Path:
    target_dir = resolve_repo_path(os.environ.get("CARGO_TARGET_DIR", "target"))
    suffix = ".exe" if "-windows-" in target else ""
    return target_dir / "release" / f"moli{suffix}"


def build_release() -> None:
    command = ["cargo", "build", "--locked", "--release", "--package", "moli"]
    run_checked(command)


def find_strip_tool(target: str) -> str:
    configured = os.environ.get("STRIP")
    candidates = [configured] if configured else []
    if "-windows-" in target:
        candidates.append("llvm-strip")
        if program_files := os.environ.get("ProgramFiles"):
            llvm_strip = Path(program_files) / "LLVM" / "bin" / "llvm-strip.exe"
            candidates.append(str(llvm_strip))
        candidates.append("strip")
    else:
        candidates.append("strip")

    for candidate in candidates:
        if not candidate:
            continue
        resolved = shutil.which(candidate)
        if resolved:
            return resolved
        candidate_path = Path(candidate)
        if candidate_path.is_file():
            return str(candidate_path.resolve())

    names = ", ".join(candidates)
    raise ReleaseError(f"no compatible strip tool found (tried: {names})")


def strip_and_sign(binary: Path, target: str) -> tuple[int, int]:
    before = binary.stat().st_size
    run_checked([find_strip_tool(target), str(binary)])

    if target.endswith("-apple-darwin"):
        # Stripping changes a signed Mach-O file, so replace the invalidated
        # signature with an ad-hoc signature before verification and packaging.
        run_checked(["codesign", "--force", "--sign", "-", str(binary)])
        run_checked(["codesign", "--verify", "--strict", str(binary)])

    after = binary.stat().st_size
    return before, after


def binary_reported_version(binary: Path) -> str:
    output = run_checked([str(binary), "--version"], capture_output=True).stdout.strip()
    program, separator, version = output.partition(" ")
    if program != "moli" or separator != " " or not version or " " in version:
        raise ReleaseError(
            "unexpected output from packaged binary `--version`: "
            f"{output or '<empty>'}"
        )
    return version


def copy_release_materials(package_dir: Path) -> None:
    files = [
        REPO_ROOT / "README.md",
        REPO_ROOT / "docs" / "RELEASING.md",
        REPO_ROOT / "LICENSE-APACHE",
        REPO_ROOT / "LICENSE-MIT",
        REPO_ROOT / "license-metadata.json",
    ]

    for source in files:
        if not source.is_file():
            raise ReleaseError(f"required release file is missing: {source}")
        shutil.copy2(source, package_dir / source.name)

    licenses_dir = REPO_ROOT / "licenses"
    if not licenses_dir.is_dir():
        raise ReleaseError(f"required license directory is missing: {licenses_dir}")
    shutil.copytree(licenses_dir, package_dir / licenses_dir.name)


def write_archive(
    package_dir: Path, archive_path: Path, package_name: str, target: str
) -> None:
    if "-windows-" in target:
        with zipfile.ZipFile(
            archive_path,
            mode="w",
            compression=zipfile.ZIP_DEFLATED,
            compresslevel=9,
        ) as archive:
            for source in sorted(package_dir.rglob("*")):
                if source.is_file():
                    relative = source.relative_to(package_dir).as_posix()
                    archive.write(source, arcname=f"{package_name}/{relative}")
        return

    with tarfile.open(
        archive_path,
        mode="w:gz",
        format=tarfile.PAX_FORMAT,
        compresslevel=9,
    ) as archive:
        archive.add(package_dir, arcname=package_name, recursive=True)


def verify_archive(archive_path: Path, expected_binary: str, target: str) -> None:
    if "-windows-" in target:
        with zipfile.ZipFile(archive_path, mode="r") as archive:
            bad_member = archive.testzip()
            if bad_member:
                raise ReleaseError(
                    f"archive verification failed for ZIP member: {bad_member}"
                )
            members = set(archive.namelist())
    else:
        with tarfile.open(archive_path, mode="r:gz") as archive:
            members = set(archive.getnames())

    if expected_binary not in members:
        raise ReleaseError(
            f"archive verification failed: {expected_binary} is not in {archive_path.name}"
        )


def package_release(
    *, version: str, target: str, binary: Path, output_dir: Path
) -> tuple[Path, int, int]:
    package_name = f"moli-v{version}-{target}"
    extension = ".zip" if "-windows-" in target else ".tar.gz"
    archive_path = output_dir / f"moli-{target}{extension}"

    output_dir.mkdir(parents=True, exist_ok=True)
    if archive_path.exists():
        raise ReleaseError(f"release output already exists: {archive_path}")

    with tempfile.TemporaryDirectory(prefix=".moli-release-", dir=output_dir) as raw:
        staging_dir = Path(raw)
        package_dir = staging_dir / package_name
        package_dir.mkdir()

        packaged_binary_name = "moli.exe" if "-windows-" in target else "moli"
        packaged_binary = package_dir / packaged_binary_name
        shutil.copy2(binary, packaged_binary)
        if "-windows-" not in target:
            packaged_binary.chmod(packaged_binary.stat().st_mode | 0o111)

        original_size, stripped_size = strip_and_sign(packaged_binary, target)
        reported_version = binary_reported_version(packaged_binary)
        if reported_version != version:
            raise ReleaseError(
                "packaged binary version does not match the requested version: "
                f"expected {version}, got {reported_version or '<empty>'}"
            )

        copy_release_materials(package_dir)
        (package_dir / "VERSION").write_text(f"{version}\n", encoding="utf-8")

        staged_archive = staging_dir / archive_path.name
        write_archive(package_dir, staged_archive, package_name, target)
        verify_archive(
            staged_archive,
            f"{package_name}/{packaged_binary_name}",
            target,
        )

        staged_archive.rename(archive_path)

    return archive_path, original_size, stripped_size


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="release version, with optional v")
    parser.add_argument(
        "--output-dir",
        default="dist",
        help="artifact output directory relative to the repository (default: dist)",
    )
    parser.add_argument(
        "--binary",
        help="prebuilt binary path (default: target/release/moli[.exe])",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="package an existing binary without running cargo build",
    )
    parser.add_argument(
        "--expected-target",
        help="fail unless the native rustc host target matches this value",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        version = normalize_version(args.version)
        declared_version = manifest_version()
        if version != declared_version:
            raise ReleaseError(
                f"requested version {version} does not match "
                f"moli/Cargo.toml ({declared_version})"
            )

        target = rust_host_target()
        if args.expected_target and target != args.expected_target:
            raise ReleaseError(
                f"expected Rust host target {args.expected_target}, got {target}"
            )

        if not args.skip_build:
            build_release()

        binary = (
            resolve_repo_path(args.binary)
            if args.binary
            else default_binary_path(target)
        )
        if not binary.is_file():
            raise ReleaseError(f"release binary not found: {binary}")

        output_dir = resolve_repo_path(args.output_dir)
        archive, before, after = package_release(
            version=version,
            target=target,
            binary=binary,
            output_dir=output_dir,
        )

        print(f"Packaged target: {target}")
        print(f"Staged binary: {before:,} -> {after:,} bytes after strip")
        print(f"Created: {archive}")
        return 0
    except ReleaseError as error:
        print(f"release error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
