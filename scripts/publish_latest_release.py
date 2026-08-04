#!/usr/bin/env python3
"""Compatibility wrapper — use scripts/publish_release.py (local WSL build, no CI)."""
from __future__ import annotations

import runpy
import sys
from pathlib import Path

print(
    "STATUS=note redirecting to scripts/publish_release.py (no GitHub Actions)",
    flush=True,
)
sys.argv[0] = str(Path(__file__).resolve().parent / "publish_release.py")
runpy.run_path(str(Path(__file__).resolve().parent / "publish_release.py"), run_name="__main__")
