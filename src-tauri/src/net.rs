use anyhow::{Context, Result, anyhow, bail};
use futures::stream::{self, StreamExt};
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct DownloadTask {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("MCLauncher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
        .expect("failed to build HTTP client")
}

pub async fn get_json<T: DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T> {
    async {
        let resp = client.get(url).send().await?.error_for_status()?;
        anyhow::Ok(resp.json::<T>().await?)
    }
    .await
    .with_context(|| format!("请求失败: {url}"))
}

pub async fn get_text(client: &reqwest::Client, url: &str) -> Result<String> {
    async {
        let resp = client.get(url).send().await?.error_for_status()?;
        anyhow::Ok(resp.text().await?)
    }
    .await
    .with_context(|| format!("请求失败: {url}"))
}

/// An existing file counts as complete when its size matches; the full SHA1 is checked
/// right after downloading, so a launch doesn't have to re-hash every asset.
async fn is_present(task: &DownloadTask) -> bool {
    match tokio::fs::metadata(&task.path).await {
        Ok(meta) if meta.is_file() => task.size.is_none_or(|size| size == meta.len()),
        _ => false,
    }
}

pub async fn download(client: &reqwest::Client, task: &DownloadTask) -> Result<()> {
    if is_present(task).await {
        return Ok(());
    }
    let mut last_err = anyhow!("未知错误");
    for attempt in 1..=3u64 {
        match try_download(client, task).await {
            Ok(()) => return Ok(()),
            Err(err) => last_err = err,
        }
        tokio::time::sleep(Duration::from_millis(500 * attempt)).await;
    }
    Err(last_err).with_context(|| format!("下载失败: {}", task.url))
}

async fn try_download(client: &reqwest::Client, task: &DownloadTask) -> Result<()> {
    let bytes = client
        .get(&task.url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if let Some(expected) = &task.sha1 {
        let actual = hex::encode(Sha1::digest(&bytes));
        if !actual.eq_ignore_ascii_case(expected) {
            bail!("SHA1 校验失败 (期望 {expected}, 实际 {actual})");
        }
    }
    if let Some(parent) = task.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut tmp = task.path.clone().into_os_string();
    tmp.push(".part");
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, &task.path).await?;
    Ok(())
}

/// Downloads everything concurrently, calling `on_progress(done, total)` as files finish.
pub async fn download_all(
    client: &reqwest::Client,
    tasks: Vec<DownloadTask>,
    concurrency: usize,
    on_progress: impl Fn(usize, usize),
) -> Result<()> {
    // Assets can share a hash, which would make two tasks write the same file.
    let mut seen = HashSet::new();
    let tasks: Vec<_> = tasks
        .into_iter()
        .filter(|t| seen.insert(t.path.clone()))
        .collect();

    let total = tasks.len();
    on_progress(0, total);
    let mut results = stream::iter(tasks)
        .map(|task| async move { download(client, &task).await })
        .buffer_unordered(concurrency.max(1));

    let mut done = 0;
    let mut errors = Vec::new();
    while let Some(result) = results.next().await {
        done += 1;
        if let Err(err) = result {
            errors.push(err);
        }
        on_progress(done, total);
    }
    match errors.len() {
        0 => Ok(()),
        n => Err(errors.swap_remove(0)).context(format!("{n} 个文件下载失败")),
    }
}
