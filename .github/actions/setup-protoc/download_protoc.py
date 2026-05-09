#!/usr/bin/env python3
"""Install an official protoc release for the current GitHub runner."""

from __future__ import annotations

import os
import platform
import re
import shutil
import stat
import sys
import urllib.request
import zipfile
from pathlib import Path


def asset_suffix(system: str, machine: str) -> str:
    system = system.lower()
    machine = machine.lower()

    if system == "linux":
        if machine in {"x86_64", "amd64"}:
            return "linux-x86_64"
        if machine in {"aarch64", "arm64"}:
            return "linux-aarch_64"
        if machine in {"ppc64le", "powerpc64le"}:
            return "linux-ppcle_64"
        if machine in {"s390x", "s390_64"}:
            return "linux-s390_64"
    if system == "darwin":
        if machine in {"x86_64", "amd64"}:
            return "osx-x86_64"
        if machine in {"aarch64", "arm64"}:
            return "osx-aarch_64"
    if system == "windows":
        if machine in {"x86_64", "amd64"}:
            return "win64"
        if machine in {"x86", "i386", "i686"}:
            return "win32"

    raise RuntimeError(f"unsupported runner platform: {system}/{machine}")


def extract_zip_safely(archive_path: Path, destination: Path) -> None:
    destination_root = destination.resolve()
    with zipfile.ZipFile(archive_path) as archive:
        for member in archive.infolist():
            target = (destination / member.filename).resolve()
            if not str(target).startswith(f"{destination_root}{os.sep}") and target != destination_root:
                raise RuntimeError(f"archive member escapes install directory: {member.filename!r}")
            archive.extract(member, destination)


def main() -> int:
    version = os.environ.get("PROTOC_VERSION", "29.3")
    if not re.fullmatch(r"\d+(?:\.\d+){1,3}", version):
        raise RuntimeError(f"invalid protoc version: {version!r}")

    runner_temp = Path(os.environ.get("RUNNER_TEMP", ".")).resolve()
    suffix = asset_suffix(platform.system(), platform.machine())
    archive_name = f"protoc-{version}-{suffix}.zip"
    url = (
        "https://github.com/protocolbuffers/protobuf/releases/download/"
        f"v{version}/{archive_name}"
    )
    archive_path = runner_temp / archive_name
    install_dir = runner_temp / f"protoc-{version}"

    print(f"Downloading {url}")
    with urllib.request.urlopen(url, timeout=60) as response:
        archive_path.write_bytes(response.read())

    if install_dir.exists():
        shutil.rmtree(install_dir)
    extract_zip_safely(archive_path, install_dir)

    bin_dir = install_dir / "bin"
    binary = bin_dir / ("protoc.exe" if platform.system().lower() == "windows" else "protoc")
    if not binary.is_file():
        raise RuntimeError(f"downloaded archive did not contain {binary}")
    if platform.system().lower() != "windows":
        binary.chmod(binary.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    github_path = os.environ.get("GITHUB_PATH")
    if github_path:
        with open(github_path, "a", encoding="utf-8") as path_file:
            path_file.write(f"{bin_dir}\n")

    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        with open(github_output, "a", encoding="utf-8") as output_file:
            output_file.write(f"version={version}\n")
            output_file.write(f"path={bin_dir}\n")

    print(f"Installed protoc {version} to {bin_dir}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # noqa: BLE001 - surface a concise CI failure.
        print(f"setup-protoc failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
