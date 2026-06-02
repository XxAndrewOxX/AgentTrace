#!/usr/bin/env python3
"""Verify checksum and extract a Windows release zip for CI smoke tests."""

from __future__ import annotations

import hashlib
import os
import zipfile
from pathlib import Path


def main() -> None:
    version = os.environ["VERSION"]
    target = os.environ["TARGET"]
    smoke_dir = os.environ["smoke_dir"]
    package_name = f"agent-trace-v{version}-{target}"
    archive = Path("dist") / f"{package_name}.zip"
    checksum_path = Path("dist") / f"{package_name}.zip.sha256"
    expected = checksum_path.read_text(encoding="utf-8").split()[0]
    actual = hashlib.sha256(archive.read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f"Checksum mismatch for {archive.name}")
    with zipfile.ZipFile(archive) as zf:
        zf.extractall(smoke_dir)


if __name__ == "__main__":
    main()
