#!/usr/bin/env bash
# Prepare a version bump in Cargo.toml and CHANGELOG.md.
# Mutates files in place — run on a clean commit or review the diff before committing.
#
# Usage:
#   ./scripts/bump_version.sh 0.2.0
#   ./scripts/bump_version.sh 0.2.0 --date 2026-06-15
#
# The script updates files only. Commit, tag, and push manually:
#   git commit -am "chore: release v0.2.0"
#   git tag v0.2.0
#   git push origin main --tags

set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 <version> [--date YYYY-MM-DD]" >&2
  exit 1
fi

NEW_VERSION="$1"
shift

DATE="$(date +%Y-%m-%d)"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --date)
      DATE="${2:?--date requires YYYY-MM-DD}"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "Invalid version: $NEW_VERSION (expected semver like 0.2.0)" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

NEW_VERSION="$NEW_VERSION" python3 - <<'PY'
import os
from pathlib import Path
import re

new_version = os.environ["NEW_VERSION"]

cargo = Path("Cargo.toml")
text = cargo.read_text(encoding="utf-8")
updated, count = re.subn(
    r'^(version = ")[^"]+(")',
    rf'\g<1>{new_version}\2',
    text,
    count=1,
    flags=re.MULTILINE,
)
if count != 1:
    raise SystemExit("Failed to update version in Cargo.toml")
cargo.write_text(updated, encoding="utf-8")

lock = Path("Cargo.lock")
lock_text = lock.read_text(encoding="utf-8")
lock_updated, lock_count = re.subn(
    r'(name = "agent-trace"\nversion = ")[^"]+(")',
    rf'\g<1>{new_version}\2',
    lock_text,
    count=1,
)
if lock_count != 1:
    raise SystemExit("Failed to update version in Cargo.lock")
lock.write_text(lock_updated, encoding="utf-8")
PY

NEW_VERSION="$NEW_VERSION" DATE="$DATE" python3 - <<'PY'
import os
from pathlib import Path
import re

new_version = os.environ["NEW_VERSION"]
date = os.environ["DATE"]

changelog = Path("CHANGELOG.md")
text = changelog.read_text(encoding="utf-8")
pattern = r"(## \[Unreleased\]\n)(.*?)(?=\n## \[|\Z)"
match = re.search(pattern, text, re.DOTALL)
if not match:
    raise SystemExit("Could not find [Unreleased] section in CHANGELOG.md")

unreleased_body = match.group(2).rstrip()
new_section = f"## [{new_version}] - {date}\n"
if unreleased_body:
    new_section += unreleased_body + "\n"

replacement = f"## [Unreleased]\n\n{new_section}"
updated = text[: match.start()] + replacement + text[match.end() :]
changelog.write_text(updated, encoding="utf-8")
PY

cargo check --locked

cat <<EOF
Updated Cargo.toml, CHANGELOG.md, and Cargo.lock for v${NEW_VERSION}.

Next steps:
  1. Review the CHANGELOG section for v${NEW_VERSION}
  2. git add Cargo.toml CHANGELOG.md Cargo.lock
  3. git commit -m "chore: release v${NEW_VERSION}"
  4. git tag v${NEW_VERSION}
  5. git push origin main --tags
EOF
