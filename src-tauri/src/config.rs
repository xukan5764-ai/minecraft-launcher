use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Baked in at build time from env vars or the repo-root `.env` (see build.rs).
const BUILTIN_CURSEFORGE_API_KEY: &str = match option_env!("CURSEFORGE_API_KEY") {
    Some(v) => v,
    None => "",
};
const BUILTIN_MS_CLIENT_ID: &str = match option_env!("MS_CLIENT_ID") {
    Some(v) => v,
    None => "",
};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// `.minecraft` root. Empty means `<app data>/minecraft`.
    pub game_dir: String,
    /// Java executable. Empty means download Mojang's runtime automatically.
    pub java_path: String,
    pub max_memory_mb: u32,
    /// Extra JVM arguments, whitespace separated.
    pub jvm_args: String,
    pub download_concurrency: usize,
    /// Overrides the built-in key when non-empty.
    pub curseforge_api_key: String,
    /// Overrides the built-in client id when non-empty.
    pub ms_client_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            game_dir: String::new(),
            java_path: String::new(),
            max_memory_mb: 4096,
            jvm_args: String::new(),
            download_concurrency: 16,
            curseforge_api_key: String::new(),
            ms_client_id: String::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Config {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_json(path, self)
    }

    pub fn curseforge_api_key(&self) -> Option<String> {
        pick(&self.curseforge_api_key, BUILTIN_CURSEFORGE_API_KEY)
    }

    pub fn ms_client_id(&self) -> Option<String> {
        pick(&self.ms_client_id, BUILTIN_MS_CLIENT_ID)
    }
}

fn pick(user: &str, builtin: &str) -> Option<String> {
    [user.trim(), builtin.trim()]
        .into_iter()
        .find(|v| !v.is_empty())
        .map(str::to_string)
}

/// What the settings page sees: the config plus whether built-in keys exist.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub config: Config,
    pub default_game_dir: String,
    pub has_builtin_curseforge_key: bool,
    pub has_builtin_ms_client_id: bool,
}

#[derive(Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
}

impl AppPaths {
    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn accounts_file(&self) -> PathBuf {
        self.data_dir.join("accounts.json")
    }

    pub fn default_game_dir(&self) -> PathBuf {
        self.data_dir.join("minecraft")
    }

    pub fn game_dir(&self, config: &Config) -> GameDir {
        let root = match config.game_dir.trim() {
            "" => self.default_game_dir(),
            dir => PathBuf::from(dir),
        };
        GameDir { root }
    }
}

/// Layout of a `.minecraft` directory.
#[derive(Clone, Debug)]
pub struct GameDir {
    pub root: PathBuf,
}

impl GameDir {
    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn version_dir(&self, id: &str) -> PathBuf {
        self.versions().join(id)
    }

    pub fn version_json(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.json"))
    }

    pub fn version_jar(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.jar"))
    }

    pub fn libraries(&self) -> PathBuf {
        self.root.join("libraries")
    }

    pub fn assets(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn runtime(&self) -> PathBuf {
        self.root.join("runtime")
    }

    /// Each version runs in its own directory (version isolation), so mods and saves don't mix.
    pub fn instance_dir(&self, id: &str) -> PathBuf {
        self.version_dir(id)
    }

    pub fn mods_dir(&self, id: &str) -> PathBuf {
        self.instance_dir(id).join("mods")
    }
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, path).with_context(|| format!("写入 {} 失败", path.display()))
}

/// Rejects names that could escape their directory (`..`, separators, ...).
pub fn safe_name(name: &str) -> Result<&str> {
    let ok = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && name != "..";
    anyhow::ensure!(ok, "非法名称: {name}");
    Ok(name)
}
