use std::collections::HashMap;
use std::path::Path;

/// Keys that can be baked into the binary at build time.
/// A real environment variable wins; otherwise the value comes from the repo-root `.env`.
const BAKED_KEYS: [&str; 2] = ["CURSEFORGE_API_KEY", "MS_CLIENT_ID"];

fn main() {
    bake_keys_from_dotenv();
    tauri_build::build()
}

fn bake_keys_from_dotenv() {
    let env_path = Path::new("../.env");
    println!("cargo:rerun-if-changed=../.env");
    for key in BAKED_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
    }

    let file_vars = std::fs::read_to_string(env_path)
        .map(|s| parse_dotenv(&s))
        .unwrap_or_default();

    for key in BAKED_KEYS {
        // option_env! already sees real environment variables, only forward .env values.
        if std::env::var_os(key).is_some() {
            continue;
        }
        if let Some(value) = file_vars.get(key) {
            println!("cargo:rustc-env={key}={value}");
        }
    }
}

/// Minimal KEY=VALUE parser. No `$` expansion on purpose: CurseForge keys contain `$`.
fn parse_dotenv(content: &str) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        if !value.is_empty() {
            vars.insert(key.trim().to_string(), value.to_string());
        }
    }
    vars
}
