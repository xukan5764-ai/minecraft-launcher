//! CurseForge mod search and install (https://docs.curseforge.com/rest-api/).

use crate::config::safe_name;
use crate::net::{self, DownloadTask};
use anyhow::{Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

const API_BASE: &str = "https://api.curseforge.com/v1";
const MINECRAFT_GAME_ID: &str = "432";
const MODS_CLASS_ID: &str = "6";
const RELEASE_TYPE_RELEASE: u32 = 1;
const RELATION_REQUIRED: u32 = 3;
const HASH_ALGO_SHA1: u32 = 1;

fn loader_type(loader: &str) -> Option<&'static str> {
    match loader {
        "forge" => Some("1"),
        "fabric" => Some("4"),
        "quilt" => Some("5"),
        "neoforge" => Some("6"),
        _ => None,
    }
}

#[derive(Deserialize)]
struct DataList<T> {
    data: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfMod {
    id: u64,
    name: String,
    summary: String,
    download_count: f64,
    logo: Option<CfLogo>,
    #[serde(default)]
    authors: Vec<CfAuthor>,
    links: Option<CfLinks>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfLogo {
    thumbnail_url: Option<String>,
}

#[derive(Deserialize)]
struct CfAuthor {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfLinks {
    website_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfFile {
    display_name: String,
    file_name: String,
    download_url: Option<String>,
    #[serde(default)]
    hashes: Vec<CfHash>,
    file_length: u64,
    file_date: String,
    release_type: u32,
    #[serde(default)]
    dependencies: Vec<CfDependency>,
}

#[derive(Deserialize)]
struct CfHash {
    value: String,
    algo: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfDependency {
    mod_id: u64,
    relation_type: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModSummary {
    pub id: u64,
    pub name: String,
    pub summary: String,
    pub download_count: u64,
    pub icon_url: Option<String>,
    pub authors: Vec<String>,
    pub website_url: Option<String>,
}

pub struct CurseForge<'a> {
    client: &'a reqwest::Client,
    api_key: String,
}

impl<'a> CurseForge<'a> {
    pub fn new(client: &'a reqwest::Client, api_key: String) -> Self {
        Self { client, api_key }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let resp = self
            .client
            .get(format!("{API_BASE}{path}"))
            .header("x-api-key", &self.api_key)
            .header("Accept", "application/json")
            .query(query)
            .send()
            .await?;
        match resp.status() {
            reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED => {
                bail!("CurseForge API Key 无效，请检查 .env 或设置页里的配置")
            }
            status if !status.is_success() => bail!("CurseForge 请求失败: HTTP {status}"),
            _ => Ok(resp.json().await?),
        }
    }

    pub async fn search(
        &self,
        query: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
        index: u32,
    ) -> Result<Vec<ModSummary>> {
        let index = index.to_string();
        let mut params = vec![
            ("gameId", MINECRAFT_GAME_ID),
            ("classId", MODS_CLASS_ID),
            ("searchFilter", query),
            ("sortField", "2"), // popularity
            ("sortOrder", "desc"),
            ("pageSize", "20"),
            ("index", index.as_str()),
        ];
        if let Some(gv) = game_version {
            params.push(("gameVersion", gv));
        }
        if let Some(lt) = loader.and_then(loader_type) {
            params.push(("modLoaderType", lt));
        }
        let list: DataList<CfMod> = self.get("/mods/search", &params).await?;
        Ok(list
            .data
            .into_iter()
            .map(|m| ModSummary {
                id: m.id,
                name: m.name,
                summary: m.summary,
                download_count: m.download_count as u64,
                icon_url: m.logo.and_then(|l| l.thumbnail_url),
                authors: m.authors.into_iter().map(|a| a.name).collect(),
                website_url: m.links.and_then(|l| l.website_url),
            })
            .collect())
    }

    /// Newest file for this game version + loader, preferring stable releases over betas.
    async fn best_file(
        &self,
        mod_id: u64,
        game_version: &str,
        loader: &str,
    ) -> Result<Option<CfFile>> {
        let mut params = vec![("gameVersion", game_version), ("pageSize", "50")];
        if let Some(lt) = loader_type(loader) {
            params.push(("modLoaderType", lt));
        }
        let list: DataList<CfFile> = self.get(&format!("/mods/{mod_id}/files"), &params).await?;
        let mut files = list.data;
        // ISO-8601 timestamps sort chronologically as strings.
        files.sort_by(|a, b| b.file_date.cmp(&a.file_date));
        let pick = files
            .iter()
            .position(|f| f.release_type == RELEASE_TYPE_RELEASE)
            .or(if files.is_empty() { None } else { Some(0) });
        Ok(pick.map(|i| files.swap_remove(i)))
    }

    /// Installs a mod and its required dependencies into `mods_dir`; returns the file names.
    pub async fn install(
        &self,
        mod_id: u64,
        game_version: &str,
        loader: &str,
        mods_dir: &Path,
    ) -> Result<Vec<String>> {
        let mut queue = vec![mod_id];
        let mut visited = HashSet::new();
        let mut installed = Vec::new();
        while let Some(id) = queue.pop() {
            if !visited.insert(id) {
                continue;
            }
            let Some(file) = self.best_file(id, game_version, loader).await? else {
                if id == mod_id {
                    bail!("这个模组没有适用于 {game_version} ({loader}) 的版本");
                }
                continue;
            };
            let Some(url) = file.download_url.clone() else {
                bail!(
                    "“{}”的作者关闭了第三方下载，请到 CurseForge 网站手动下载",
                    file.display_name
                );
            };
            let file_name = safe_name(&file.file_name)?.to_string();
            net::download(
                self.client,
                &DownloadTask {
                    url,
                    path: mods_dir.join(&file_name),
                    sha1: file
                        .hashes
                        .iter()
                        .find(|h| h.algo == HASH_ALGO_SHA1)
                        .map(|h| h.value.clone()),
                    size: Some(file.file_length),
                },
            )
            .await?;
            installed.push(file_name);
            queue.extend(
                file.dependencies
                    .iter()
                    .filter(|d| d.relation_type == RELATION_REQUIRED)
                    .map(|d| d.mod_id),
            );
        }
        Ok(installed)
    }
}
