#!/usr/bin/env python3
"""Package a built agent-trace binary for GitHub Releases.

The script expects the binary to have already been built with:

    cargo build --locked --release --target <target-triple>

It produces one archive plus a matching .sha256 file under dist/.
"""

from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import tarfile
import zipfile


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Package agent-trace release artifact")
    parser.add_argument("--version", required=True, help="Package version without leading v")
    parser.add_argument("--target", required=True, help="Rust target triple")
    parser.add_argument("--profile", default="release", help="Cargo profile name")
    parser.add_argument("--dist-dir", default="dist", help="Output directory")
    return parser.parse_args()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def binary_path(target: str, profile: str) -> Path:
    exe = "agent-trace.exe" if "windows" in target else "agent-trace"
    target_path = Path("target") / target / profile / exe
    if target_path.exists():
        return target_path

    native_path = Path("target") / profile / exe
    if native_path.exists():
        return native_path

    raise FileNotFoundError(f"built binary not found at {target_path} or {native_path}")


def copy_payload(staging_dir: Path, built_binary: Path) -> None:
    staging_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(built_binary, staging_dir / built_binary.name)

    if os.name != "nt":
        mode = (staging_dir / built_binary.name).stat().st_mode
        (staging_dir / built_binary.name).chmod(mode | 0o755)

    for filename in ("README.md", "LICENSE", "CHANGELOG.md"):
        source = Path(filename)
        if source.exists():
            shutil.copy2(source, staging_dir / filename)

    install_source = Path("docs") / "INSTALL.md"
    if install_source.exists():
        shutil.copy2(install_source, staging_dir / "INSTALL.md")


def make_tarball(staging_dir: Path, archive_path: Path) -> None:
    with tarfile.open(archive_path, "w:gz") as archive:
        for path in sorted(staging_dir.iterdir()):
            archive.add(path, arcname=path.name)


def make_zip(staging_dir: Path, archive_path: Path) -> None:
    with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(staging_dir.iterdir()):
            archive.write(path, arcname=path.name)


def main() -> None:
    args = parse_args()
    version = args.version.removeprefix("v")
    target = args.target
    dist_dir = Path(args.dist_dir)
    package_name = f"agent-trace-v{version}-{target}"
    staging_dir = dist_dir / package_name

    if staging_dir.exists():
        shutil.rmtree(staging_dir)
    dist_dir.mkdir(parents=True, exist_ok=True)

    copy_payload(staging_dir, binary_path(target, args.profile))

    if "windows" in target:
        archive_path = dist_dir / f"{package_name}.zip"
        make_zip(staging_dir, archive_path)
    else:
        archive_path = dist_dir / f"{package_name}.tar.gz"
        make_tarball(staging_dir, archive_path)

    checksum = sha256_file(archive_path)
    checksum_path = archive_path.with_suffix(archive_path.suffix + ".sha256")
    checksum_path.write_text(f"{checksum}  {archive_path.name}\n", encoding="utf-8")
    shutil.rmtree(staging_dir)

    print(archive_path)
    print(checksum_path)


if __name__ == "__main__":
    main()
