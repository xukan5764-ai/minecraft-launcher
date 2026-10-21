//! Builds the java command line for a version and prepares natives/legacy assets.

use crate::auth::{Account, AccountKind};
use crate::config::GameDir;
use crate::version::{self, ArgValue, Argument, ResolvedLibrary, VersionJson};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

pub const LAUNCHER_NAME: &str = "MCLauncher";

pub struct LaunchOptions {
    pub java: PathBuf,
    pub max_memory_mb: u32,
    pub extra_jvm_args: String,
    pub ms_client_id: String,
}

pub struct PreparedLaunch {
    pub java: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    /// Secrets to hide when the command line is logged.
    pub secrets: Vec<String>,
}

impl PreparedLaunch {
    pub fn redacted_command(&self) -> String {
        let mut line = std::iter::once(self.java.display().to_string())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        for secret in self.secrets.iter().filter(|s| !s.is_empty()) {
            line = line.replace(secret.as_str(), "******");
        }
        line
    }
}

/// Replaces `${name}` placeholders; unknown ones are left untouched.
fn substitute(arg: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(arg.len());
    let mut rest = arg;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        match vars.get(&after[..end]) {
            Some(value) => out.push_str(value),
            None => out.push_str(&rest[start..start + 2 + end + 1]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn expand_arguments(args: &[Argument], vars: &HashMap<&str, String>) -> Vec<String> {
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Argument::Plain(value) => out.push(substitute(value, vars)),
            Argument::Conditional { rules, value } if version::rules_allow(rules) => match value {
                ArgValue::One(v) => out.push(substitute(v, vars)),
                ArgValue::Many(vs) => out.extend(vs.iter().map(|v| substitute(v, vars))),
            },
            Argument::Conditional { .. } => {}
        }
    }
    out
}

/// Unpacks old-style natives jars. File names are flattened, which also rules out zip-slip.
fn extract_natives(libs: &[ResolvedLibrary], natives_dir: &Path) -> Result<()> {
    let _ = std::fs::remove_dir_all(natives_dir);
    std::fs::create_dir_all(natives_dir)?;
    for lib in libs.iter().filter(|l| l.extract_natives) {
        let file =
            File::open(&lib.path).with_context(|| format!("缺少本地库 {}", lib.path.display()))?;
        let mut archive = zip::ZipArchive::new(file)?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name()?.into_owned();
            if entry.is_dir() || name.starts_with("META-INF/") {
                continue;
            }
            let Some(file_name) = Path::new(&name).file_name() else {
                continue;
            };
            let mut out = File::create(natives_dir.join(file_name))?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

pub async fn prepare(
    game: GameDir,
    version: VersionJson,
    account: Account,
    options: LaunchOptions,
) -> Result<PreparedLaunch> {
    // Extracting natives and copying legacy assets is blocking file IO.
    tokio::task::spawn_blocking(move || prepare_blocking(&game, &version, &account, options))
        .await?
}

fn prepare_blocking(
    game: &GameDir,
    version: &VersionJson,
    account: &Account,
    options: LaunchOptions,
) -> Result<PreparedLaunch> {
    let instance_dir = game.instance_dir(&version.id);
    std::fs::create_dir_all(&instance_dir)?;
    let natives_dir = instance_dir.join("natives");

    let libs = version::resolve_libraries(version, game);
    extract_natives(&libs, &natives_dir)?;
    let game_assets = version::prepare_legacy_assets(game, version, &instance_dir)?;

    let client_jar = game.version_jar(version.jar_id());
    anyhow::ensure!(
        client_jar.exists(),
        "缺少客户端文件 {}",
        client_jar.display()
    );
    let separator = if cfg!(windows) { ";" } else { ":" };
    let classpath = libs
        .iter()
        .filter(|l| l.classpath)
        .map(|l| l.path.display().to_string())
        .chain(std::iter::once(client_jar.display().to_string()))
        .collect::<Vec<_>>()
        .join(separator);

    let path_str = |p: &Path| p.display().to_string();
    let token = account.access_token.clone();
    let vars: HashMap<&str, String> = HashMap::from([
        ("auth_player_name", account.name.clone()),
        ("auth_uuid", account.uuid.clone()),
        ("auth_access_token", token.clone()),
        ("auth_session", format!("token:{token}:{}", account.uuid)),
        ("auth_xuid", String::new()),
        ("clientid", options.ms_client_id.clone()),
        (
            "user_type",
            match account.kind {
                AccountKind::Microsoft => "msa",
                AccountKind::Offline => "legacy",
            }
            .to_string(),
        ),
        ("user_properties", "{}".to_string()),
        ("version_name", version.id.clone()),
        (
            "version_type",
            version.kind.clone().unwrap_or_else(|| "release".into()),
        ),
        ("game_directory", path_str(&instance_dir)),
        ("assets_root", path_str(&game.assets())),
        ("game_assets", path_str(&game_assets)),
        ("assets_index_name", version.asset_index_name()),
        ("natives_directory", path_str(&natives_dir)),
        ("library_directory", path_str(&game.libraries())),
        ("classpath", classpath.clone()),
        ("classpath_separator", separator.to_string()),
        ("launcher_name", LAUNCHER_NAME.to_string()),
        ("launcher_version", env!("CARGO_PKG_VERSION").to_string()),
    ]);

    let mut args = vec![format!("-Xmx{}M", options.max_memory_mb)];
    args.extend(
        options
            .extra_jvm_args
            .split_whitespace()
            .map(str::to_string),
    );

    if let Some(logging) = version.logging.as_ref().and_then(|l| l.client.as_ref()) {
        let config = game.assets().join("log_configs").join(&logging.file.id);
        if config.exists() {
            args.push(logging.argument.replace("${path}", &path_str(&config)));
        }
    }

    let declared_jvm = version
        .arguments
        .as_ref()
        .map(|a| expand_arguments(&a.jvm, &vars))
        .unwrap_or_default();
    // Pre-1.13 versions (and loaders on top of them) don't declare the classpath themselves.
    if !declared_jvm.iter().any(|a| a == "-cp" || a == "-classpath") {
        if cfg!(target_os = "macos") {
            args.push("-XstartOnFirstThread".into());
        }
        args.push(format!("-Djava.library.path={}", path_str(&natives_dir)));
        args.push("-cp".into());
        args.push(classpath);
    }
    args.extend(declared_jvm);

    args.push(
        version
            .main_class
            .clone()
            .context("版本文件缺少 mainClass")?,
    );

    if let Some(legacy) = &version.minecraft_arguments {
        args.extend(legacy.split_whitespace().map(|a| substitute(a, &vars)));
    }
    if let Some(arguments) = &version.arguments {
        args.extend(expand_arguments(&arguments.game, &vars));
    }

    Ok(PreparedLaunch {
        java: options.java,
        args,
        working_dir: instance_dir,
        secrets: vec![token],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_placeholders() {
        let vars = HashMap::from([("a", "1".to_string()), ("path", "/x y".to_string())]);
        assert_eq!(substitute("${a}", &vars), "1");
        assert_eq!(substitute("-Dp=${path}/lib", &vars), "-Dp=/x y/lib");
        assert_eq!(substitute("${unknown}-${a}", &vars), "${unknown}-1");
        assert_eq!(substitute("tail ${a", &vars), "tail ${a");
    }

    #[test]
    fn builds_modern_command_line() {
        let dir = std::env::temp_dir().join(format!("mcl-test-{}", uuid::Uuid::new_v4().simple()));
        let game = GameDir { root: dir.clone() };
        let version: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.21", "type": "release", "mainClass": "net.minecraft.client.main.Main",
                "assetIndex": {"id": "17", "url": "https://example.invalid/17.json"},
                "arguments": {
                    "game": ["--username", "${auth_player_name}", "--accessToken", "${auth_access_token}",
                             {"rules": [{"action": "allow", "features": {"is_demo_user": true}}], "value": "--demo"}],
                    "jvm": ["-Djava.library.path=${natives_directory}", "-cp", "${classpath}"]
                },
                "libraries": [{"name": "com.mojang:brigadier:1.2.9"}]
            }"#,
        )
        .unwrap();
        std::fs::create_dir_all(game.version_dir("1.21")).unwrap();
        std::fs::write(game.version_jar("1.21"), b"").unwrap();

        let account = crate::auth::offline_account("Steve").unwrap();
        let prepared = prepare_blocking(
            &game,
            &version,
            &account,
            LaunchOptions {
                java: PathBuf::from("java"),
                max_memory_mb: 2048,
                extra_jvm_args: "-XX:+UseG1GC".into(),
                ms_client_id: String::new(),
            },
        )
        .unwrap();

        let args = &prepared.args;
        assert_eq!(&args[..2], ["-Xmx2048M", "-XX:+UseG1GC"]);
        let cp = &args[args.iter().position(|a| a == "-cp").unwrap() + 1];
        assert!(cp.contains("brigadier-1.2.9.jar") && cp.ends_with("1.21.jar"));
        assert_eq!(args.iter().filter(|a| *a == "-cp").count(), 1);
        assert!(args.contains(&"net.minecraft.client.main.Main".to_string()));
        assert!(args.contains(&"Steve".to_string()));
        assert!(!args.contains(&"--demo".to_string()));
        assert!(!prepared.redacted_command().contains(&account.access_token));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
