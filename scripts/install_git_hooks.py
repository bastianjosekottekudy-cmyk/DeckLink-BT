#!/usr/bin/env python3
"""Point this repo at .githooks so post-commit publishes the Linux release.

Usage:
  python scripts/install_git_hooks.py

Exit: 0 ok, 1 error
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HOOKS = ROOT / ".githooks"


def main() -> int:
    if not (HOOKS / "post-commit").is_file():
        print(f"ERROR=missing {HOOKS / 'post-commit'}", flush=True)
        return 1
    subprocess.run(
        ["git", "config", "core.hooksPath", ".githooks"],
        cwd=str(ROOT),
        check=True,
    )
    post = HOOKS / "post-commit"
    try:
        post.chmod(post.stat().st_mode | 0o111)
    except OSError:
        pass
    print("STATUS=hooksPath=.githooks", flush=True)
    print(
        "STATUS=post-commit will run scripts/publish_release.py after crates/packaging commits",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
