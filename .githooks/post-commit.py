#!/usr/bin/env python3
"""post-commit: rebuild Linux release and replace GitHub latest asset.

Runs when HEAD touches crates/, packaging/, Cargo.*, or release scripts
(or always if DECKLINK_PUBLISH_ALWAYS=1).
Does not bump Cargo.toml version.
Skip with: git commit --no-verify   or   DECKLINK_SKIP_PUBLISH=1
"""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PUBLISH = ROOT / "scripts" / "publish_release.py"

WATCH_PREFIXES = (
    "crates/",
    "packaging/",
    "Cargo.toml",
    "Cargo.lock",
    "scripts/publish_release.py",
    "scripts/install-deck.sh",
    "scripts/uninstall-deck.sh",
)


def head_touches_release_paths() -> bool:
    r = subprocess.run(
        ["git", "diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    files = [ln.strip().replace("\\", "/") for ln in (r.stdout or "").splitlines()]
    for f in files:
        for prefix in WATCH_PREFIXES:
            if f == prefix or f.startswith(prefix):
                return True
    return False


def main() -> int:
    if os.environ.get("DECKLINK_SKIP_PUBLISH", "").strip() in {"1", "true", "yes"}:
        print("STATUS=publish skipped (DECKLINK_SKIP_PUBLISH)", flush=True)
        return 0
    always = os.environ.get("DECKLINK_PUBLISH_ALWAYS", "").strip() in {"1", "true", "yes"}
    if not always and not head_touches_release_paths():
        print("STATUS=publish skipped (no release-path changes in HEAD)", flush=True)
        return 0
    if not PUBLISH.is_file():
        print(f"ERROR=missing {PUBLISH}", flush=True)
        return 1
    print("STATUS=publishing DeckLink BT release after commit…", flush=True)
    return subprocess.call([sys.executable, str(PUBLISH)], cwd=str(ROOT))


if __name__ == "__main__":
    raise SystemExit(main())
