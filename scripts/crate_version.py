#!/usr/bin/env python3
"""Print the package version from Cargo.toml."""

from __future__ import annotations

import tomllib
from pathlib import Path


def main() -> None:
    cargo_toml = Path(__file__).resolve().parent.parent / "Cargo.toml"
    with cargo_toml.open("rb") as handle:
        print(tomllib.load(handle)["package"]["version"])


if __name__ == "__main__":
    main()
