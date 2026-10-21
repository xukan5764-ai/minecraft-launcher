//! Mojang version manifest, version JSON parsing (including `inheritsFrom`), and game file installation.

use crate::config::{GameDir, safe_name};
use crate::net::{self, DownloadTask};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const LIBRARIES_BASE: &str = "https://libraries.minecraft.net/";
const RESOURCES_BASE: &str = "https://resources.download.minecraft.net";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<ManifestEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub release_time: String,
    #[serde(default)]
    pub sha1: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    pub id: String,
    pub inherits_from: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub main_class: Option<String>,
    /// Pre-1.13 game arguments as one string.
    pub minecraft_arguments: Option<String>,
    pub arguments: Option<Arguments>,
    pub asset_index: Option<AssetIndexRef>,
    pub assets: Option<String>,
    pub downloads: Option<VersionDownloads>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    pub java_version: Option<JavaVersion>,
    pub logging: Option<Logging>,
    /// Which version's client jar to use; set when merging an inherited version.
    pub jar: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Conditional {
        #[serde(default)]
        rules: Vec<Rule>,
        value: ArgValue,
    },
}

#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum ArgValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize, Clone, Debug)]
pub struct Rule {
    pub action: String,
    pub os: Option<OsRule>,
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct OsRule {
    pub name: Option<String>,
    pub arch: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct AssetIndexRef {
    pub id: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct VersionDownloads {
    pub client: Option<DownloadInfo>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DownloadInfo {
    pub path: Option<String>,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Library {
    pub name: String,
    pub downloads: Option<LibraryDownloads>,
    /// Maven repository base, used by loaders such as Fabric that omit `downloads`.
    pub url: Option<String>,
    pub rules: Option<Vec<Rule>>,
    pub natives: Option<HashMap<String, String>>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LibraryDownloads {
    pub artifact: Option<DownloadInfo>,
    pub classifiers: Option<HashMap<String, DownloadInfo>>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct JavaVersion {
    pub component: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Logging {
    pub client: Option<LoggingClient>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LoggingClient {
    pub argument: String,
    pub file: LoggingFile,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LoggingFile {
    pub id: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

#[derive(Deserialize)]
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
    #[serde(default, rename = "virtual")]
    is_virtual: bool,
    #[serde(default)]
    map_to_resources: bool,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

// ---------------------------------------------------------------------------
// Rules

pub fn os_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

fn os_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        arch => arch,
    }
}

fn arch_bits() -> &'static str {
    if cfg!(target_pointer_width = "64") {
        "64"
    } else {
        "32"
    }
}

impl Rule {
    fn matches(&self) -> bool {
        if let Some(os) = &self.os {
            if os.name.as_deref().is_some_and(|name| name != os_name()) {
                return false;
            }
            if os.arch.as_deref().is_some_and(|arch| arch != os_arch()) {
                return false;
            }
        }
        // No launcher features (demo user, custom resolution, quick play) are enabled.
        !self
            .features
            .as_ref()
            .is_some_and(|features| features.values().any(|enabled| *enabled))
    }
}

/// Mojang semantics: no rules means allowed, otherwise the last matching rule decides.
pub fn rules_allow(rules: &[Rule]) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules.iter().filter(|r| r.matches()) {
        allowed = rule.action == "allow";
    }
    allowed
}

// ---------------------------------------------------------------------------
// Libraries

/// `group:artifact:version[:classifier][@ext]` → `group/path/artifact/version/artifact-version[-classifier].ext`
pub fn maven_path(name: &str) -> Option<String> {
    let (coords, ext) = name.split_once('@').unwrap_or((name, "jar"));
    let parts: Vec<&str> = coords.split(':').collect();
    let [group, artifact, version, rest @ ..] = parts.as_slice() else {
        return None;
    };
    let group = group.replace('.', "/");
    let file = match rest.first() {
        Some(classifier) => format!("{artifact}-{version}-{classifier}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!("{group}/{artifact}/{version}/{file}"))
}

/// Identity of a library ignoring its version, so a child version can override its parent's copy.
fn library_key(name: &str) -> String {
    let coords = name.split('@').next().unwrap_or(name);
    let parts: Vec<&str> = coords.split(':').collect();
    match parts.as_slice() {
        [group, artifact, _version, classifier, ..] => format!("{group}:{artifact}:{classifier}"),
        [group, artifact, ..] => format!("{group}:{artifact}"),
        _ => name.to_string(),
    }
}

fn with_classifier(name: &str, classifier: &str) -> String {
    match name.split_once('@') {
        Some((coords, ext)) => format!("{coords}:{classifier}@{ext}"),
        None => format!("{name}:{classifier}"),
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedLibrary {
    pub path: PathBuf,
    pub download: Option<DownloadTask>,
    pub classpath: bool,
    /// Old-style natives jar that has to be unpacked into the natives directory.
    pub extract_natives: bool,
}

fn resolve_artifact(
    lib: &Library,
    info: Option<&DownloadInfo>,
    classifier: Option<&str>,
    lib_dir: &Path,
) -> Option<ResolvedLibrary> {
    let maven_name = match classifier {
        Some(c) => with_classifier(&lib.name, c),
        None => lib.name.clone(),
    };
    let rel = info
        .and_then(|i| i.path.clone())
        .or_else(|| maven_path(&maven_name))?;
    let path = lib_dir.join(&rel);
    let download = match info {
        Some(i) if !i.url.is_empty() => Some(DownloadTask {
            url: i.url.clone(),
            path: path.clone(),
            sha1: i.sha1.clone(),
            size: i.size,
        }),
        // An empty URL means the file is produced locally (e.g. by a loader installer).
        Some(_) => None,
        None => {
            let base = lib.url.as_deref().unwrap_or(LIBRARIES_BASE);
            Some(DownloadTask {
                url: format!("{}/{rel}", base.trim_end_matches('/')),
                path: path.clone(),
                sha1: lib.sha1.clone(),
                size: lib.size,
            })
        }
    };
    Some(ResolvedLibrary {
        path,
        download,
        classpath: true,
        extract_natives: false,
    })
}

/// Libraries that apply to this OS, in classpath order, with duplicates removed.
pub fn resolve_libraries(version: &VersionJson, game: &GameDir) -> Vec<ResolvedLibrary> {
    let lib_dir = game.libraries();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for lib in &version.libraries {
        if !rules_allow(lib.rules.as_deref().unwrap_or_default()) {
            continue;
        }
        let downloads = lib.downloads.as_ref();
        let artifact = downloads.and_then(|d| d.artifact.as_ref());

        if let Some(natives) = &lib.natives {
            if let Some(classifier) = natives.get(os_name()) {
                let classifier = classifier.replace("${arch}", arch_bits());
                let info = downloads
                    .and_then(|d| d.classifiers.as_ref())
                    .and_then(|c| c.get(&classifier));
                if let Some(native) = resolve_artifact(lib, info, Some(&classifier), &lib_dir) {
                    out.push(ResolvedLibrary {
                        classpath: false,
                        extract_natives: true,
                        ..native
                    });
                }
            }
            if artifact.is_none() {
                continue;
            }
        }

        // Newer versions ship natives as `...:natives-<os>` classpath jars; LWJGL unpacks
        // those itself into `-Dorg.lwjgl.system.SharedLibraryExtractPath`.
        if !seen.insert(library_key(&lib.name)) {
            continue;
        }
        if let Some(resolved) = resolve_artifact(lib, artifact, None, &lib_dir) {
            out.push(resolved);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Loading versions

pub fn read_version_json(game: &GameDir, id: &str) -> Result<VersionJson> {
    let path = game.version_json(safe_name(id)?);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("找不到版本 {id}，请先安装"))?;
    serde_json::from_str(&text).with_context(|| format!("解析 {} 失败", path.display()))
}

/// Loads a version and merges every `inheritsFrom` parent into it.
pub fn load_version(game: &GameDir, id: &str) -> Result<VersionJson> {
    let mut version = read_version_json(game, id)?;
    let mut depth = 0;
    while let Some(parent_id) = version.inherits_from.clone() {
        depth += 1;
        anyhow::ensure!(depth <= 8, "版本 {id} 的继承链过深");
        let parent = read_version_json(game, &parent_id)
            .with_context(|| format!("{id} 依赖的原版 {parent_id} 未安装"))?;
        version = merge(parent, version);
    }
    Ok(version)
}

fn merge(parent: VersionJson, child: VersionJson) -> VersionJson {
    let mut libraries = child.libraries;
    libraries.extend(parent.libraries);

    let arguments = match (parent.arguments, child.arguments) {
        (Some(mut p), Some(c)) => {
            p.game.extend(c.game);
            p.jvm.extend(c.jvm);
            Some(p)
        }
        (p, c) => c.or(p),
    };

    VersionJson {
        jar: child.jar.or(parent.jar).or(Some(parent.id)),
        id: child.id,
        inherits_from: parent.inherits_from,
        kind: child.kind.or(parent.kind),
        main_class: child.main_class.or(parent.main_class),
        minecraft_arguments: child.minecraft_arguments.or(parent.minecraft_arguments),
        arguments,
        asset_index: child.asset_index.or(parent.asset_index),
        assets: child.assets.or(parent.assets),
        downloads: child.downloads.or(parent.downloads),
        libraries,
        java_version: child.java_version.or(parent.java_version),
        logging: child.logging.or(parent.logging),
    }
}

impl VersionJson {
    pub fn jar_id(&self) -> &str {
        self.jar.as_deref().unwrap_or(&self.id)
    }

    pub fn asset_index_name(&self) -> String {
        self.asset_index
            .as_ref()
            .map(|a| a.id.clone())
            .or_else(|| self.assets.clone())
            .unwrap_or_else(|| "legacy".to_string())
    }

    /// Mojang runtime component this version wants; versions before 1.17 don't say and use Java 8.
    pub fn java_component(&self) -> String {
        self.java_version
            .as_ref()
            .map(|j| j.component.clone())
            .unwrap_or_else(|| "jre-legacy".to_string())
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstalledVersion {
    pub id: String,
    pub game_version: String,
    /// "fabric" | "quilt" | "forge" | "neoforge", or None for vanilla.
    pub loader: Option<String>,
    pub loader_version: Option<String>,
}

fn detect_loader(version: &VersionJson) -> (Option<String>, Option<String>) {
    let main_class = version.main_class.as_deref().unwrap_or_default();
    let loader = [
        ("net.fabricmc", "fabric"),
        ("org.quiltmc", "quilt"),
        ("neoforged", "neoforge"),
        ("cpw.mods", "forge"),
        ("net.minecraftforge", "forge"),
    ]
    .into_iter()
    .find(|(needle, _)| main_class.contains(needle))
    .map(|(_, loader)| loader.to_string());

    let loader_version = (loader.as_deref() == Some("fabric"))
        .then(|| {
            version.libraries.iter().find_map(|lib| {
                lib.name
                    .strip_prefix("net.fabricmc:fabric-loader:")
                    .map(str::to_string)
            })
        })
        .flatten();
    (loader, loader_version)
}

pub fn installed_info(game: &GameDir, id: &str) -> Result<InstalledVersion> {
    let version = read_version_json(game, id)?;
    let (loader, loader_version) = detect_loader(&version);
    Ok(InstalledVersion {
        game_version: version
            .inherits_from
            .clone()
            .unwrap_or_else(|| id.to_string()),
        id: id.to_string(),
        loader,
        loader_version,
    })
}

/// Installed versions, most recently modified first.
pub fn list_installed(game: &GameDir) -> Vec<InstalledVersion> {
    let Ok(entries) = std::fs::read_dir(game.versions()) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().to_string();
            let info = installed_info(game, &id).ok()?;
            let modified = entry.metadata().and_then(|m| m.modified()).ok();
            Some((modified, info))
        })
        .collect();
    found.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    found.into_iter().map(|(_, info)| info).collect()
}

// ---------------------------------------------------------------------------
// Installing

pub async fn fetch_manifest(client: &reqwest::Client) -> Result<VersionManifest> {
    net::get_json(client, MANIFEST_URL).await
}

/// Downloads a vanilla version's JSON from Mojang if it isn't there yet.
pub async fn install_vanilla_json(
    client: &reqwest::Client,
    game: &GameDir,
    id: &str,
) -> Result<()> {
    let path = game.version_json(safe_name(id)?);
    if path.exists() {
        return Ok(());
    }
    let manifest = fetch_manifest(client).await?;
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == id)
        .ok_or_else(|| anyhow!("Mojang 版本列表中没有 {id}"))?;
    net::download(
        client,
        &DownloadTask {
            url: entry.url.clone(),
            path,
            sha1: entry.sha1.clone(),
            size: None,
        },
    )
    .await
}

pub type Progress<'a> = &'a (dyn Fn(&str, usize, usize) + Send + Sync);

/// Makes sure the client jar, libraries, logging config and assets of a (merged) version exist.
pub async fn ensure_game_files(
    client: &reqwest::Client,
    game: &GameDir,
    version: &VersionJson,
    concurrency: usize,
    progress: Progress<'_>,
) -> Result<()> {
    let mut tasks = Vec::new();

    let jar_path = game.version_jar(version.jar_id());
    match version.downloads.as_ref().and_then(|d| d.client.as_ref()) {
        Some(client_dl) => tasks.push(DownloadTask {
            url: client_dl.url.clone(),
            path: jar_path,
            sha1: client_dl.sha1.clone(),
            size: client_dl.size,
        }),
        None => anyhow::ensure!(jar_path.exists(), "缺少客户端文件 {}", jar_path.display()),
    }

    tasks.extend(
        resolve_libraries(version, game)
            .into_iter()
            .filter_map(|lib| lib.download),
    );

    if let Some(logging) = version.logging.as_ref().and_then(|l| l.client.as_ref()) {
        tasks.push(DownloadTask {
            url: logging.file.url.clone(),
            path: game
                .assets()
                .join("log_configs")
                .join(safe_name(&logging.file.id)?),
            sha1: logging.file.sha1.clone(),
            size: logging.file.size,
        });
    }

    net::download_all(client, tasks, concurrency, |done, total| {
        progress("下载游戏文件", done, total)
    })
    .await?;

    let Some(index_ref) = &version.asset_index else {
        return Ok(());
    };
    let index_path = asset_index_path(game, &index_ref.id)?;
    net::download(
        client,
        &DownloadTask {
            url: index_ref.url.clone(),
            path: index_path.clone(),
            sha1: index_ref.sha1.clone(),
            size: index_ref.size,
        },
    )
    .await?;
    let index = read_asset_index(&index_path)?;
    let objects_dir = game.assets().join("objects");
    let asset_tasks = index
        .objects
        .values()
        .filter(|obj| obj.hash.len() > 2)
        .map(|obj| DownloadTask {
            url: format!("{RESOURCES_BASE}/{}/{}", &obj.hash[..2], obj.hash),
            path: objects_dir.join(&obj.hash[..2]).join(&obj.hash),
            sha1: Some(obj.hash.clone()),
            size: Some(obj.size),
        })
        .collect();
    net::download_all(client, asset_tasks, concurrency, |done, total| {
        progress("下载资源文件", done, total)
    })
    .await
}

fn asset_index_path(game: &GameDir, id: &str) -> Result<PathBuf> {
    Ok(game
        .assets()
        .join("indexes")
        .join(format!("{}.json", safe_name(id)?)))
}

fn read_asset_index(path: &Path) -> Result<AssetIndex> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("读取资源索引 {} 失败", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

/// Very old versions read assets by name instead of hash; copy them where they look.
/// Returns the directory to use for `${game_assets}`.
pub fn prepare_legacy_assets(
    game: &GameDir,
    version: &VersionJson,
    instance_dir: &Path,
) -> Result<PathBuf> {
    let index_name = version.asset_index_name();
    let Ok(index) = asset_index_path(game, &index_name).and_then(|p| read_asset_index(&p)) else {
        return Ok(game.assets());
    };
    let target = if index.map_to_resources {
        instance_dir.join("resources")
    } else if index.is_virtual {
        game.assets().join("virtual").join(&index_name)
    } else {
        return Ok(game.assets());
    };
    let objects_dir = game.assets().join("objects");
    for (name, obj) in &index.objects {
        if name.split(['/', '\\']).any(|part| part == "..") || obj.hash.len() <= 2 {
            continue;
        }
        let dest = target.join(name);
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(objects_dir.join(&obj.hash[..2]).join(&obj.hash), &dest)
            .with_context(|| format!("复制旧版资源 {name} 失败"))?;
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_paths() {
        assert_eq!(
            maven_path("net.fabricmc:fabric-loader:0.16.9").as_deref(),
            Some("net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar")
        );
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.3:natives-linux").as_deref(),
            Some("org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar")
        );
        assert_eq!(
            maven_path("de.oceanlabs.mcp:mcp_config:1.20.1@zip").as_deref(),
            Some("de/oceanlabs/mcp/mcp_config/1.20.1/mcp_config-1.20.1.zip")
        );
        assert_eq!(maven_path("broken"), None);
    }

    #[test]
    fn rules() {
        let allow_other_os: Vec<Rule> =
            serde_json::from_str(r#"[{"action":"allow","os":{"name":"nonexistent-os"}}]"#).unwrap();
        assert!(!rules_allow(&allow_other_os));

        let disallow_other_os: Vec<Rule> = serde_json::from_str(
            r#"[{"action":"allow"},{"action":"disallow","os":{"name":"nonexistent-os"}}]"#,
        )
        .unwrap();
        assert!(rules_allow(&disallow_other_os));

        let demo_only: Vec<Rule> =
            serde_json::from_str(r#"[{"action":"allow","features":{"is_demo_user":true}}]"#)
                .unwrap();
        assert!(!rules_allow(&demo_only));
        assert!(rules_allow(&[]));
    }

    #[test]
    fn merge_inherited_version() {
        let parent: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.20.1", "type": "release", "mainClass": "net.minecraft.client.main.Main",
                "arguments": {"game": ["--username", "${auth_player_name}"], "jvm": ["-cp", "${classpath}"]},
                "assetIndex": {"id": "5", "url": "https://example.invalid/5.json"},
                "libraries": [{"name": "org.ow2.asm:asm:9.0"}, {"name": "com.google.guava:guava:31.1-jre"}]
            }"#,
        )
        .unwrap();
        let child: VersionJson = serde_json::from_str(
            r#"{
                "id": "fabric-loader-0.16.9-1.20.1", "inheritsFrom": "1.20.1",
                "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
                "arguments": {"game": [], "jvm": ["-DFabricMcEmu= net.minecraft.client.main.Main "]},
                "libraries": [{"name": "org.ow2.asm:asm:9.7", "url": "https://maven.fabricmc.net/"}]
            }"#,
        )
        .unwrap();

        let merged = merge(parent, child);
        assert_eq!(merged.id, "fabric-loader-0.16.9-1.20.1");
        assert_eq!(merged.jar_id(), "1.20.1");
        assert_eq!(
            merged.main_class.as_deref(),
            Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
        );
        assert_eq!(merged.asset_index_name(), "5");
        assert_eq!(merged.arguments.as_ref().unwrap().jvm.len(), 3);

        let game = GameDir {
            root: PathBuf::from("/mc"),
        };
        let libs = resolve_libraries(&merged, &game);
        // The loader's newer ASM wins over vanilla's.
        assert_eq!(libs.len(), 2);
        assert!(libs[0].path.ends_with("org/ow2/asm/asm/9.7/asm-9.7.jar"));
        assert_eq!(
            libs[0].download.as_ref().unwrap().url,
            "https://maven.fabricmc.net/org/ow2/asm/asm/9.7/asm-9.7.jar"
        );
    }

    #[test]
    fn old_style_natives() {
        let version: VersionJson = serde_json::from_str(&format!(
            r#"{{
                "id": "1.8.9",
                "libraries": [{{
                    "name": "org.lwjgl.lwjgl:lwjgl-platform:2.9.4",
                    "natives": {{"{os}": "natives-{os}-${{arch}}"}},
                    "downloads": {{"classifiers": {{"natives-{os}-{bits}": {{
                        "path": "natives.jar", "url": "https://example.invalid/natives.jar"
                    }}}}}}
                }}]
            }}"#,
            os = os_name(),
            bits = arch_bits(),
        ))
        .unwrap();
        let libs = resolve_libraries(
            &version,
            &GameDir {
                root: PathBuf::from("/mc"),
            },
        );
        assert_eq!(libs.len(), 1);
        assert!(libs[0].extract_natives && !libs[0].classpath);
    }
}
