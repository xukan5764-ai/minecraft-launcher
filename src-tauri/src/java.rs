//! Downloads the Java runtime Mojang publishes for each game version.

use crate::config::{GameDir, safe_name};
use crate::net::{self, DownloadTask};
use crate::version::DownloadInfo;
use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const RUNTIME_INDEX_URL: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

#[derive(Deserialize)]
struct RuntimeEntry {
    manifest: DownloadInfo,
    version: RuntimeVersion,
}

#[derive(Deserialize)]
struct RuntimeVersion {
    name: String,
}

#[derive(Deserialize)]
struct RuntimeManifest {
    files: HashMap<String, RuntimeFile>,
}

#[derive(Deserialize)]
struct RuntimeFile {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    executable: bool,
    downloads: Option<RuntimeDownloads>,
    target: Option<String>,
}

#[derive(Deserialize)]
struct RuntimeDownloads {
    raw: DownloadInfo,
}

/// Mojang platform names to try, best match first (ARM Macs/Windows fall back to x64 emulation).
fn platform_keys() -> &'static [&'static str] {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => &["windows-x64"],
        ("windows", "x86") => &["windows-x86"],
        ("windows", "aarch64") => &["windows-arm64", "windows-x64"],
        ("macos", "aarch64") => &["mac-os-arm64", "mac-os"],
        ("macos", _) => &["mac-os"],
        ("linux", "x86_64") => &["linux"],
        ("linux", "x86") => &["linux-i386"],
        _ => &[],
    }
}

fn java_executable(runtime_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        runtime_dir.join("bin").join("java.exe")
    } else if cfg!(target_os = "macos") {
        runtime_dir.join("jre.bundle/Contents/Home/bin/java")
    } else {
        runtime_dir.join("bin").join("java")
    }
}

/// Returns the java executable for `component` (e.g. `java-runtime-delta`), downloading it if needed.
pub async fn ensure_runtime(
    client: &reqwest::Client,
    game: &GameDir,
    component: &str,
    concurrency: usize,
    progress: impl Fn(usize, usize),
) -> Result<PathBuf> {
    let dir = game.runtime().join(safe_name(component)?);
    let marker = dir.join(".installed");
    let exe = java_executable(&dir);
    if marker.exists() && exe.exists() {
        return Ok(exe);
    }

    let index: HashMap<String, HashMap<String, Vec<RuntimeEntry>>> =
        net::get_json(client, RUNTIME_INDEX_URL).await?;
    let entry = platform_keys()
        .iter()
        .find_map(|platform| index.get(*platform)?.get(component)?.first())
        .ok_or_else(|| {
            anyhow!(
                "Mojang 没有为当前平台提供 Java 运行时 {component}，请在设置中手动指定 Java 路径"
            )
        })?;
    let manifest: RuntimeManifest = net::get_json(client, &entry.manifest.url).await?;

    let mut tasks = Vec::new();
    let mut executables = Vec::new();
    let mut links = Vec::new();
    for (rel, file) in &manifest.files {
        if rel.split('/').any(|part| part == "..") {
            continue;
        }
        let path = dir.join(rel);
        match (file.kind.as_str(), &file.downloads, &file.target) {
            ("directory", _, _) => std::fs::create_dir_all(&path)?,
            ("file", Some(downloads), _) => {
                tasks.push(DownloadTask {
                    url: downloads.raw.url.clone(),
                    path: path.clone(),
                    sha1: downloads.raw.sha1.clone(),
                    size: downloads.raw.size,
                });
                if file.executable {
                    executables.push(path);
                }
            }
            ("link", _, Some(target)) => links.push((path, target.clone())),
            _ => {}
        }
    }
    net::download_all(client, tasks, concurrency, progress).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in &executables {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        }
        for (link, target) in &links {
            if link.symlink_metadata().is_ok() {
                continue;
            }
            if let Some(parent) = link.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::os::unix::fs::symlink(target, link)?;
        }
    }
    #[cfg(not(unix))]
    let _ = (executables, links);

    anyhow::ensure!(
        exe.exists(),
        "Java 运行时安装不完整，缺少 {}",
        exe.display()
    );
    std::fs::write(&marker, &entry.version.name)?;
    Ok(exe)
}
