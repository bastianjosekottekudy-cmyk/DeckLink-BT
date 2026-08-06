#!/usr/bin/env python3
"""Build DeckLink BT Linux tarball locally (WSL) and replace GitHub Release assets.

No GitHub Actions. No automatic version bump — reuses Cargo.toml version / tag v{version}.

Usage:
  python scripts/publish_release.py
  python scripts/publish_release.py --skip-build
  python scripts/publish_release.py --bump patch|minor|major

Exit codes:
  0 ok
  1 error
  2 needs human judgment (dirty tree after bump, etc.)
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CARGO_TOML = ROOT / "Cargo.toml"
DIST = ROOT / "dist"
STAGE_NAME = "decklink-bt-linux-x86_64"
TARBALL_NAME = f"{STAGE_NAME}.tar.gz"
TARBALL = DIST / TARBALL_NAME


def emit(**kwargs: object) -> None:
    for key, value in kwargs.items():
        print(f"{key}={value}", flush=True)


def run(
    cmd: list[str],
    *,
    check: bool = True,
    cwd: Path | None = None,
    env: dict | None = None,
) -> subprocess.CompletedProcess[str]:
    print("+", " ".join(cmd), flush=True)
    merged = os.environ.copy()
    if env:
        merged.update(env)
    return subprocess.run(
        cmd,
        cwd=str(cwd or ROOT),
        text=True,
        capture_output=True,
        check=check,
        env=merged,
    )


def die(msg: str, code: int = 1) -> None:
    emit(STATUS="error", ERROR=msg)
    sys.exit(code)


def read_version() -> str:
    text = CARGO_TOML.read_text(encoding="utf-8")
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
        return re.sub(
            r'(?m)^version\s*=\s*"[^"]+"',
            f'version = "{new_version}"',
            match.group(0),
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
    return f"{major}.{minor}.{patch + 1}"


def git_clean_enough() -> bool:
    r = run(["git", "status", "--porcelain"], check=False)
    return r.returncode == 0 and not (r.stdout or "").strip()


def wsl_available() -> bool:
    r = run(["wsl", "-e", "true"], check=False)
    return r.returncode == 0


def windows_to_wsl_path(path: Path) -> str:
    # Forward slashes so WSL does not eat backslashes when argv is parsed.
    win = str(path.resolve()).replace("\\", "/")
    r = run(["wsl", "wslpath", "-a", win], check=False)
    if r.returncode != 0:
        die((r.stderr or r.stdout or "wslpath failed").strip())
    out = (r.stdout or "").strip()
    if not out.startswith("/"):
        die(f"wslpath returned unexpected path: {out!r}")
    return out


def build_linux_tarball() -> Path:
    """Build release binary in WSL Ubuntu and package dist/*.tar.gz."""
    DIST.mkdir(parents=True, exist_ok=True)
    if os.name == "nt":
        if not wsl_available():
            die("WSL required on Windows to build the Linux Steam Deck binary")
        script = ROOT / "scripts" / "wsl_build_release.sh"
        if not script.is_file():
            die(f"missing {script}")
        # Normalize line endings for bash (force LF; Windows text mode would re-add CRLF).
        raw = script.read_bytes().replace(b"\r\n", b"\n").replace(b"\r", b"\n")
        script.write_bytes(raw)
        wsl_script = windows_to_wsl_path(script)
        print(f"+ wsl bash {wsl_script}", flush=True)
        proc = subprocess.run(
            ["wsl", "bash", "-c", f"sed -i 's/\\r$//' '{wsl_script}' && bash '{wsl_script}'"],
            cwd=str(ROOT),
            text=True,
        )
        if proc.returncode != 0:
            die(f"WSL build failed exit={proc.returncode}")
    else:
        # Native Linux (e.g. building on Deck / Ubuntu)
        run(["cargo", "test", "-p", "decklink-hid", "-p", "decklink-profiles"])
        run(["cargo", "build", "--release", "-p", "decklink-app"])
        stage = DIST / STAGE_NAME
        if stage.exists():
            shutil.rmtree(stage)
        stage.mkdir(parents=True)
        bin_src = ROOT / "target" / "release" / "decklink-bt"
        if not bin_src.is_file():
            die(f"missing binary {bin_src}")
        shutil.copy2(bin_src, stage / "decklink-bt")
        for name in ("README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE"):
            p = ROOT / name
            if p.is_file():
                shutil.copy2(p, stage / name)
        for name in ("scripts", "packaging"):
            shutil.copytree(ROOT / name, stage / name, dirs_exist_ok=True)
        if TARBALL.exists():
            TARBALL.unlink()
        with tarfile.open(TARBALL, "w:gz") as tar:
            tar.add(stage, arcname=STAGE_NAME)

    if not TARBALL.is_file():
        die(f"missing tarball {TARBALL}")
    emit(STATUS="built", ARTIFACT=str(TARBALL), BYTES=TARBALL.stat().st_size)
    return TARBALL


def release_exists(tag: str) -> bool:
    r = run(["gh", "release", "view", tag], check=False)
    return r.returncode == 0


def default_notes(version: str) -> str:
    return f"""## DeckLink v{version}

Steam Deck as a Wi-Fi gamepad / keyboard+mouse for Windows (ViGEmBus host).

### Install
1. **PC:** Install ViGEmBus, run `decklink-host.exe` (UDP 31415)
2. **Deck:** Download `{TARBALL_NAME}`, extract, `bash scripts/install-deck.sh ./{TARBALL_NAME}`
3. Open **DeckLink**, enter PC LAN IP, Connect

Same release tag is reused; assets are replaced on every publish. Version is not bumped automatically.
"""


def publish_release(tag: str, tarball: Path, version: str) -> None:
    title = f"DeckLink v{version}"
    notes = default_notes(version)
    if release_exists(tag):
        r = run(
            ["gh", "release", "upload", tag, str(tarball), "--clobber"],
            check=False,
        )
        if r.returncode != 0:
            die((r.stderr or r.stdout or "gh release upload failed").strip())
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
            ],
            check=False,
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
            ],
            check=False,
        )
        if r.returncode != 0:
            die((r.stderr or r.stdout or "gh release create failed").strip())
    emit(STATUS="uploaded", TAG=tag)


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--bump", choices=["patch", "minor", "major"])
    p.add_argument("--skip-build", action="store_true")
    p.add_argument("--dry-run", action="store_true")
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
        if not git_clean_enough():
            die(
                f"Version bumped to {version} but working tree has other changes. "
                "Commit explicitly, then re-run publish.",
                code=2,
            )
        run(["git", "add", "Cargo.toml"])
        run(["git", "commit", "-m", f"Bump version to {version}."])
        r = run(["git", "push", "origin", "HEAD"], check=False)
        if r.returncode != 0:
            die((r.stderr or r.stdout or "git push failed").strip(), code=2)

    tag = f"v{version}"
    emit(STATUS="start", VERSION=version, TAG=tag, BUMPED=str(bumped).lower())

    if args.dry_run:
        emit(STATUS="dry_run", ACTION=f"would build locally and replace {tag}")
        return

    if args.skip_build:
        if not TARBALL.is_file():
            die("artifacts missing; run without --skip-build")
        tarball = TARBALL
    else:
        tarball = build_linux_tarball()

    publish_release(tag, tarball, version)

    sha = run(["git", "rev-parse", "HEAD"], check=False)
    head = (sha.stdout or "").strip() if sha.returncode == 0 else ""
    if head:
        run(["git", "tag", "-f", tag, head], check=False)
        run(["git", "push", "-f", "origin", f"refs/tags/{tag}"], check=False)

    url_r = run(
        ["gh", "release", "view", tag, "--json", "url", "-q", ".url"],
        check=False,
    )
    emit(
        STATUS="ok",
        RELEASE_URL=(url_r.stdout or "").strip(),
        TAG=tag,
        VERSION=version,
        TARBALL=f"https://github.com/bastianjosekottekudy-cmyk/DeckLink-BT/releases/latest/download/{TARBALL_NAME}",
    )


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as e:
        die(f"command failed exit={e.returncode}: {(e.stderr or e.stdout or '')[:500]}")
    except KeyboardInterrupt:
        die("interrupted")
