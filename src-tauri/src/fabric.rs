//! Fabric loader install via https://meta.fabricmc.net.

use crate::config::{GameDir, safe_name};
use crate::{net, version};
use anyhow::Result;
use serde::{Deserialize, Serialize};

const META_BASE: &str = "https://meta.fabricmc.net/v2";

#[derive(Deserialize)]
struct LoaderEntry {
    loader: LoaderInfo,
}

#[derive(Serialize, Deserialize)]
pub struct LoaderInfo {
    pub version: String,
    pub stable: bool,
}

#[derive(Deserialize)]
struct Profile {
    id: String,
}

pub async fn list_loaders(client: &reqwest::Client, game_version: &str) -> Result<Vec<LoaderInfo>> {
    let url = format!("{META_BASE}/versions/loader/{}", safe_name(game_version)?);
    let entries: Vec<LoaderEntry> = net::get_json(client, &url).await?;
    Ok(entries.into_iter().map(|e| e.loader).collect())
}

/// Writes the Fabric version JSON (which inherits from vanilla) and returns its id.
pub async fn install(
    client: &reqwest::Client,
    game: &GameDir,
    game_version: &str,
    loader_version: &str,
) -> Result<String> {
    let url = format!(
        "{META_BASE}/versions/loader/{}/{}/profile/json",
        safe_name(game_version)?,
        safe_name(loader_version)?
    );
    let text = net::get_text(client, &url).await?;
    let id = serde_json::from_str::<Profile>(&text)?.id;
    safe_name(&id)?;

    version::install_vanilla_json(client, game, game_version).await?;
    let path = game.version_json(&id);
    std::fs::create_dir_all(game.version_dir(&id))?;
    std::fs::write(path, text)?;
    Ok(id)
}
