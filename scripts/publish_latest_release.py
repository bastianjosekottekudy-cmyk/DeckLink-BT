#!/usr/bin/env python3
"""Build/fetch Linux artifacts from CI and replace the current GitHub Release assets.

Default behavior (no version bump):
  - Read version from Cargo.toml workspace.package.version
  - Wait for CI build-linux artifact on the current HEAD SHA
  - Create or update GitHub release v{version}, replacing assets (--clobber)

Bump version only when explicitly requested:
  python scripts/publish_latest_release.py --bump patch|minor|major

Exit codes:
  0  success
  1  error (prints ERROR=...)
  2  conflicts / needs human judgment
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CARGO_TOML = ROOT / "Cargo.toml"
ARTIFACT_NAME = "decklink-bt-linux-x86_64"
TARBALL = f"{ARTIFACT_NAME}.tar.gz"
WORKFLOW = "ci.yml"


def emit(**kwargs: object) -> None:
    for key, value in kwargs.items():
        print(f"{key}={value}", flush=True)


def run(cmd: list[str], *, check: bool = False, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        text=True,
        capture_output=True,
        check=check,
        cwd=str(cwd or ROOT),
    )


def die(msg: str, code: int = 1) -> None:
    emit(STATUS="error", ERROR=msg)
    sys.exit(code)


def read_version() -> str:
    text = CARGO_TOML.read_text(encoding="utf-8")
    # Prefer [workspace.package] version
    m = re.search(
        r"(?ms)^\[workspace\.package\].*?^version\s*=\s*\"([^\"]+)\"",
        text,
    )
    if not m:
        m = re.search(r'(?m)^version\s*=\s*"([^"]+)"', text)
    if not m:
        die("Could not read version from Cargo.toml")
    return m.group(1)


def write_version(new_version: str) -> None:
    text = CARGO_TOML.read_text(encoding="utf-8")

    def repl_workspace(match: re.Match[str]) -> str:
        block = match.group(0)
        return re.sub(
            r'(?m)^version\s*=\s*"[^"]+"',
            f'version = "{new_version}"',
            block,
            count=1,
        )

    new_text, n = re.subn(
        r"(?ms)^\[workspace\.package\].*?(?=^\[|\Z)",
        repl_workspace,
        text,
        count=1,
    )
    if n != 1:
        die("Failed to update [workspace.package] version")
    CARGO_TOML.write_text(new_text, encoding="utf-8")


def bump_semver(version: str, part: str) -> str:
    pieces = version.split(".")
    if len(pieces) != 3 or not all(p.isdigit() for p in pieces):
        die(f"Version is not semver MAJOR.MINOR.PATCH: {version}")
    major, minor, patch = map(int, pieces)
    if part == "major":
        return f"{major + 1}.0.0"
    if part == "minor":
        return f"{major}.{minor + 1}.0"
    if part == "patch":
        return f"{major}.{minor}.{patch + 1}"
    die(f"Unknown bump part: {part}")
    raise AssertionError


def git_sha() -> str:
    r = run(["git", "rev-parse", "HEAD"])
    if r.returncode != 0:
        die((r.stderr or r.stdout or "git rev-parse failed").strip())
    return (r.stdout or "").strip()


def git_clean_enough() -> bool:
    r = run(["git", "status", "--porcelain"])
    return r.returncode == 0 and not (r.stdout or "").strip()


def wait_for_ci(sha: str, timeout_s: int) -> int:
    """Return successful workflow run id for SHA."""
    deadline = time.time() + timeout_s
    last_status = ""
    while time.time() < deadline:
        r = run(
            [
                "gh",
                "run",
                "list",
                "--workflow",
                WORKFLOW,
                "--commit",
                sha,
                "--json",
                "databaseId,status,conclusion,name,event",
                "--limit",
                "10",
            ]
        )
        if r.returncode != 0:
            die((r.stderr or r.stdout or "gh run list failed").strip())
        runs = json.loads(r.stdout or "[]")
        # Prefer completed success with build artifacts (push/workflow_dispatch)
        for run_info in runs:
            if run_info.get("conclusion") == "success" and run_info.get("status") == "completed":
                emit(CI_STATUS="success", RUN_ID=run_info["databaseId"])
                return int(run_info["databaseId"])
        for run_info in runs:
            status = run_info.get("status")
            conclusion = run_info.get("conclusion")
            last_status = f"{status}/{conclusion}"
            if status in {"in_progress", "queued", "pending", "waiting"}:
                emit(CI_STATUS=status, RUN_ID=run_info.get("databaseId", ""))
                break
            if conclusion == "failure":
                die(f"CI failed for {sha} (run {run_info.get('databaseId')})")
        time.sleep(10)
    die(f"Timed out waiting for CI on {sha} (last={last_status})")
    raise AssertionError


def download_artifact(run_id: int, dest_dir: Path) -> Path:
    dest_dir.mkdir(parents=True, exist_ok=True)
    r = run(
        [
            "gh",
            "run",
            "download",
            str(run_id),
            "--name",
            ARTIFACT_NAME,
            "--dir",
            str(dest_dir),
        ]
    )
    if r.returncode != 0:
        die((r.stderr or r.stdout or "gh run download failed").strip())
    tarball = dest_dir / TARBALL
    if not tarball.is_file():
        # Sometimes nested
        matches = list(dest_dir.rglob(TARBALL))
        if not matches:
            die(f"Artifact {TARBALL} not found after download")
        tarball = matches[0]
    return tarball


def release_exists(tag: str) -> bool:
    r = run(["gh", "release", "view", tag])
    return r.returncode == 0


def publish_release(tag: str, tarball: Path, title: str, notes: str) -> None:
    if release_exists(tag):
        # Replace assets on existing release
        r = run(
            [
                "gh",
                "release",
                "upload",
                tag,
                str(tarball),
                "--clobber",
            ]
        )
        if r.returncode != 0:
            die((r.stderr or r.stdout or "gh release upload failed").strip())
        # Refresh notes lightly
        run(
            [
                "gh",
                "release",
                "edit",
                tag,
                "--title",
                title,
                "--notes",
                notes,
                "--draft=false",
                "--latest",
            ]
        )
    else:
        r = run(
            [
                "gh",
                "release",
                "create",
                tag,
                str(tarball),
                "--title",
                title,
                "--notes",
                notes,
                "--latest",
            ]
        )
        if r.returncode != 0:
            die((r.stderr or r.stdout or "gh release create failed").strip())


def default_notes(version: str) -> str:
    return f"""## DeckLink BT v{version}

Steam Deck as a driverless BLE HOGP gamepad.

### Install (Steam Deck Desktop Mode)
1. Download `{TARBALL}` below
2. Extract and run `./scripts/install-deck.sh ./{TARBALL}`
3. Launch **DeckLink BT** from Gaming Mode → Start Advertising → pair from host

Assets are replaced on each publish for this version unless you bump with `--bump`.
"""


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--bump",
        choices=["patch", "minor", "major"],
        help="Explicitly bump Cargo.toml version before publishing",
    )
    p.add_argument(
        "--timeout",
        type=int,
        default=1800,
        help="Seconds to wait for CI (default 1800)",
    )
    p.add_argument(
        "--skip-wait-ci",
        action="store_true",
        help="Use newest successful CI run on this commit if already done",
    )
    p.add_argument(
        "--dry-run",
        action="store_true",
        help="Print actions without uploading",
    )
    return p.parse_args()


def main() -> None:
    args = parse_args()
    version = read_version()
    bumped = False

    if args.bump:
        new_version = bump_semver(version, args.bump)
        write_version(new_version)
        version = new_version
        bumped = True
        emit(VERSION_BUMPED=version)
        # Commit bump if dirty — require clean commit separately; ask on ambiguity
        if not git_clean_enough():
            die(
                f"Version bumped to {version} in Cargo.toml but working tree has other changes. "
                "Commit explicitly, then re-run publish.",
                code=2,
            )
        msg = f"Bump version to {version}."
        r = run(["git", "add", "Cargo.toml"])
        if r.returncode != 0:
            die((r.stderr or "git add failed").strip())
        r = run(["git", "commit", "-m", msg])
        if r.returncode != 0:
            die((r.stderr or r.stdout or "git commit failed").strip())
        r = run(["git", "push", "origin", "HEAD"])
        if r.returncode != 0:
            die((r.stderr or r.stdout or "git push failed").strip(), code=2)

    tag = f"v{version}"
    sha = git_sha()
    emit(STATUS="start", VERSION=version, TAG=tag, SHA=sha, BUMPED=str(bumped).lower())

    if args.dry_run:
        emit(STATUS="dry_run", ACTION=f"would wait CI then replace release {tag} assets")
        return

    run_id = wait_for_ci(sha, args.timeout)
    with tempfile.TemporaryDirectory(prefix="decklink-release-") as tmp:
        tarball = download_artifact(run_id, Path(tmp))
        emit(ARTIFACT=str(tarball))
        title = f"DeckLink BT v{version}"
        publish_release(tag, tarball, title, default_notes(version))

    # Ensure tag points at this SHA (move tag if needed for same version republish)
    run(["git", "tag", "-f", tag, sha])
    r = run(["git", "push", "-f", "origin", f"refs/tags/{tag}"])
    if r.returncode != 0:
        # Non-fatal if release assets already updated; tag push may need force
        emit(TAG_PUSH="failed", ERROR=(r.stderr or r.stdout or "").strip())
    else:
        emit(TAG_PUSH="ok")

    url_r = run(["gh", "release", "view", tag, "--json", "url", "-q", ".url"])
    emit(
        STATUS="ok",
        RELEASE_URL=(url_r.stdout or "").strip(),
        TAG=tag,
        VERSION=version,
    )


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        die("interrupted")
