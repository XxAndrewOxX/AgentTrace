#!/usr/bin/env python3
"""Extract a version section from CHANGELOG.md for GitHub Release notes."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Extract a CHANGELOG.md version section")
    parser.add_argument(
        "--version",
        required=True,
        help="Release version without a leading v (for example: 0.1.0)",
    )
    parser.add_argument(
        "--changelog",
        default="CHANGELOG.md",
        help="Path to CHANGELOG.md",
    )
    return parser.parse_args()


def extract_section(changelog_path: Path, version: str) -> str:
    content = changelog_path.read_text(encoding="utf-8")
    version = version.removeprefix("v")
    pattern = rf"^## \[{re.escape(version)}\][^\n]*\n.*?(?=^## \[|\Z)"
    match = re.search(pattern, content, re.MULTILINE | re.DOTALL)
    if not match:
        raise ValueError(f"No CHANGELOG section found for version {version}")
    return match.group(0).strip()


def main() -> None:
    args = parse_args()
    try:
        section = extract_section(Path(args.changelog), args.version)
    except (FileNotFoundError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)
    print(section)


if __name__ == "__main__":
    main()
