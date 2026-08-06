//! Detect / install Nefarius ViGEmBus (required for Xbox mode).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tracing::{info, warn};

const VIGEM_MSI_NAME: &str = "ViGEmBusSetup_x64.msi";
/// Official signed setup (GPL-3). Bundled in release zip when possible.
const VIGEM_MSI_URL: &str =
    "https://github.com/nefarius/ViGEmBus/releases/download/setup-v1.17.333/ViGEmBusSetup_x64.msi";

pub fn vigem_available() -> bool {
    vigem_client::Client::connect().is_ok()
}

/// If ViGEmBus is missing, install from bundled MSI (or download), with UAC prompt.
pub fn ensure_vigem() -> Result<()> {
    if vigem_available() {
        info!("ViGEmBus already installed");
        return Ok(());
    }

    warn!("ViGEmBus not found — installing driver (UAC prompt)…");
    let msi = locate_or_download_msi().context("locate/download ViGEmBus MSI")?;
    info!("installing {}", msi.display());
    run_msi_elevated(&msi).context("ViGEmBus MSI install")?;

    for attempt in 1..=40 {
        if vigem_available() {
            info!("ViGEmBus ready after install (attempt {attempt})");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!(
        "ViGEmBus installer ran but the driver is not ready yet. \
         Reboot Windows, then start DeckLink Host again."
    );
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn locate_or_download_msi() -> Result<PathBuf> {
    let beside = exe_dir().join(VIGEM_MSI_NAME);
    if beside.is_file() {
        return Ok(beside);
    }
    let bundled = exe_dir().join("drivers").join(VIGEM_MSI_NAME);
    if bundled.is_file() {
        return Ok(bundled);
    }

    // Cache under %LOCALAPPDATA%\DeckLink\drivers
    let cache_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("DeckLink")
        .join("drivers");
    std::fs::create_dir_all(&cache_dir)?;
    let cached = cache_dir.join(VIGEM_MSI_NAME);
    if cached.is_file() && cached.metadata().map(|m| m.len() > 100_000).unwrap_or(false) {
        return Ok(cached);
    }

    info!("downloading ViGEmBus setup…");
    download_file(VIGEM_MSI_URL, &cached)?;
    Ok(cached)
}

fn download_file(url: &str, dest: &Path) -> Result<()> {
    let tmp = dest.with_extension("msi.part");
    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing",
                url.replace('\'', "''"),
                tmp.display().to_string().replace('\'', "''")
            ),
        ])
        .status()
        .context("powershell download")?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("download failed (exit {status})");
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

fn run_msi_elevated(msi: &Path) -> Result<()> {
    let msi_s = msi.display().to_string().replace('\'', "''");
    // runas + Wait so we don't continue before the driver is copied.
    let ps = format!(
        "Start-Process -FilePath msiexec.exe -ArgumentList '/i','{msi_s}','/qn' -Verb RunAs -Wait"
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps])
        .status()
        .context("elevate msiexec")?;
    if !status.success() {
        bail!(
            "ViGEmBus install was cancelled or failed (exit {status}). \
             Approve the UAC prompt, or install manually from {VIGEM_MSI_URL}"
        );
    }
    Ok(())
}
