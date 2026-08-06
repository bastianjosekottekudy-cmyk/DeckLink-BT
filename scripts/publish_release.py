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
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CARGO_TOML = ROOT / "Cargo.toml"
DIST = ROOT / "dist"
STAGE_NAME = "decklink-bt-linux-x86_64"
TARBALL_NAME = f"{STAGE_NAME}.tar.gz"
TARBALL = DIST / TARBALL_NAME
HOST_STAGE_NAME = "decklink-host-windows-x86_64"
HOST_ZIP_NAME = f"{HOST_STAGE_NAME}.zip"
HOST_ZIP = DIST / HOST_ZIP_NAME
VIGEM_MSI_NAME = "ViGEmBusSetup_x64.msi"
VIGEM_MSI_URL = (
    "https://github.com/nefarius/ViGEmBus/releases/download/"
    "setup-v1.17.333/ViGEmBusSetup_x64.msi"
)


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


def ensure_vigem_msi(cache: Path) -> Path:
    """Download official ViGEmBus MSI into cache if missing."""
    cache.mkdir(parents=True, exist_ok=True)
    dest = cache / VIGEM_MSI_NAME
    if dest.is_file() and dest.stat().st_size > 100_000:
        return dest
    print(f"+ download {VIGEM_MSI_URL}", flush=True)
    tmp = dest.with_suffix(".msi.part")
    try:
        urllib.request.urlretrieve(VIGEM_MSI_URL, tmp)
        tmp.replace(dest)
    except Exception as e:
        if tmp.is_file():
            tmp.unlink()
        die(f"failed to download ViGEmBus MSI: {e}")
    if not dest.is_file() or dest.stat().st_size < 100_000:
        die("downloaded ViGEmBus MSI looks truncated")
    emit(STATUS="vigem_msi", BYTES=dest.stat().st_size)
    return dest


def build_windows_host_zip() -> Path:
    """Build decklink-host.exe and zip it with ViGEmBus MSI for auto-install."""
    if os.name != "nt":
        die("Windows host zip must be built on Windows")
    DIST.mkdir(parents=True, exist_ok=True)
    run(["cargo", "build", "--release", "-p", "decklink-host"])
    exe = ROOT / "target" / "release" / "decklink-host.exe"
    if not exe.is_file():
        die(f"missing {exe}")

    msi = ensure_vigem_msi(DIST / "drivers")
    stage = DIST / HOST_STAGE_NAME
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    shutil.copy2(exe, stage / "decklink-host.exe")
    shutil.copy2(msi, stage / VIGEM_MSI_NAME)
    for name in ("README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE"):
        p = ROOT / name
        if p.is_file():
            shutil.copy2(p, stage / name)
    readme_host = stage / "HOST-README.txt"
    readme_host.write_text(
        "DeckLink Host (Windows)\n"
        "=======================\n\n"
        "1. Extract this zip anywhere.\n"
        "2. Run decklink-host.exe\n"
        "3. On first launch, approve UAC to install ViGEmBus "
        f"({VIGEM_MSI_NAME} is included).\n"
        "4. Allow firewall UDP 31415 if prompted.\n"
        "5. On the Steam Deck open DeckLink and tap Connect "
        "(no IP needed).\n\n"
        "ViGEmBus is GPL-3 (Nefarius): https://github.com/nefarius/ViGEmBus\n",
        encoding="utf-8",
    )

    if HOST_ZIP.exists():
        HOST_ZIP.unlink()
    with zipfile.ZipFile(HOST_ZIP, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for path in sorted(stage.rglob("*")):
            if path.is_file():
                zf.write(path, arcname=f"{HOST_STAGE_NAME}/{path.relative_to(stage).as_posix()}")
    emit(STATUS="built_host", ARTIFACT=str(HOST_ZIP), BYTES=HOST_ZIP.stat().st_size)
    return HOST_ZIP


def default_notes(version: str) -> str:
    return f"""## DeckLink v{version}

Steam Deck as a Wi-Fi Xbox controller / keyboard+mouse for Windows.

### Install
1. **PC:** Download `{HOST_ZIP_NAME}`, extract, run `decklink-host.exe`
   - ViGEmBus MSI is bundled; first launch installs it (UAC)
   - Allow firewall UDP **31415** if prompted
2. **Deck:** Download `{TARBALL_NAME}`, extract, `bash scripts/install-deck.sh ./{TARBALL_NAME}`
3. Open **DeckLink** → **Connect** (auto-discovers the PC — no IP)

Same release tag is reused; assets are replaced on every publish. Version is not bumped automatically.
"""


def publish_release(tag: str, artifacts: list[Path], version: str) -> None:
    title = f"DeckLink v{version}"
    notes = default_notes(version)
    if release_exists(tag):
        for art in artifacts:
            r = run(
                ["gh", "release", "upload", tag, str(art), "--clobber"],
                check=False,
            )
            if r.returncode != 0:
                die((r.stderr or r.stdout or f"gh upload {art.name} failed").strip())
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
        cmd = [
            "gh",
            "release",
            "create",
            tag,
            *[str(a) for a in artifacts],
            "--title",
            title,
            "--notes",
            notes,
            "--latest",
        ]
        r = run(cmd, check=False)
        if r.returncode != 0:
            die((r.stderr or r.stdout or "gh release create failed").strip())
    emit(STATUS="uploaded", TAG=tag, ASSETS=",".join(a.name for a in artifacts))


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

    artifacts: list[Path] = []
    if args.skip_build:
        if not TARBALL.is_file():
            die("Linux tarball missing; run without --skip-build")
        if not HOST_ZIP.is_file():
            die("Windows host zip missing; run without --skip-build")
        artifacts = [TARBALL, HOST_ZIP]
    else:
        artifacts.append(build_linux_tarball())
        if os.name == "nt":
            artifacts.append(build_windows_host_zip())
        else:
            emit(STATUS="note", MESSAGE="skip Windows host zip (not on Windows)")

    publish_release(tag, artifacts, version)

    sha = run(["git", "rev-parse", "HEAD"], check=False)
    head = (sha.stdout or "").strip() if sha.returncode == 0 else ""
    if head:
        run(["git", "tag", "-f", tag, head], check=False)
        run(["git", "push", "-f", "origin", f"refs/tags/{tag}"], check=False)

    url_r = run(
        ["gh", "release", "view", tag, "--json", "url", "-q", ".url"],
        check=False,
    )
    base = "https://github.com/bastianjosekottekudy-cmyk/DeckLink-BT/releases/latest/download"
    emit(
        STATUS="ok",
        RELEASE_URL=(url_r.stdout or "").strip(),
        TAG=tag,
        VERSION=version,
        TARBALL=f"{base}/{TARBALL_NAME}",
        HOST_ZIP=f"{base}/{HOST_ZIP_NAME}",
    )


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as e:
        die(f"command failed exit={e.returncode}: {(e.stderr or e.stdout or '')[:500]}")
    except KeyboardInterrupt:
        die("interrupted")
