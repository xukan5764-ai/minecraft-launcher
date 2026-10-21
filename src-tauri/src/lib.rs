mod auth;
mod config;
mod curseforge;
mod error;
mod fabric;
mod java;
mod launch;
mod net;
mod version;

use anyhow::{Context, anyhow};
use auth::{AccountStore, AccountView, DeviceCode};
use config::{AppPaths, Config, GameDir, SettingsView, safe_name};
use error::Result;
use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::oneshot;

struct AppState {
    paths: AppPaths,
    http: reqwest::Client,
    config: Mutex<Config>,
    accounts: Mutex<AccountStore>,
    login_cancelled: AtomicBool,
    game_running: AtomicBool,
    game_kill: Mutex<Option<oneshot::Sender<()>>>,
}

impl AppState {
    fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    fn game_dir(&self) -> GameDir {
        self.paths.game_dir(&self.config.lock().unwrap())
    }

    fn update_accounts<T>(
        &self,
        f: impl FnOnce(&mut AccountStore) -> anyhow::Result<T>,
    ) -> Result<T> {
        let mut store = self.accounts.lock().unwrap();
        let out = f(&mut store)?;
        store.save(&self.paths.accounts_file())?;
        Ok(out)
    }
}

#[derive(Serialize, Clone)]
struct ProgressEvent {
    stage: String,
    done: usize,
    total: usize,
}

/// Emits `install-progress` events, throttled to roughly 200 per stage.
fn progress_reporter(app: &AppHandle) -> impl Fn(&str, usize, usize) + Send + Sync + 'static {
    let app = app.clone();
    move |stage, done, total| {
        let step = (total / 200).max(1);
        if done == 0 || done == total || done % step == 0 {
            let _ = app.emit(
                "install-progress",
                ProgressEvent {
                    stage: stage.to_string(),
                    done,
                    total,
                },
            );
        }
    }
}

fn log_line(app: &AppHandle, line: impl Into<String>) {
    let _ = app.emit("game-log", line.into());
}

// ---------------------------------------------------------------------------
// Settings

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> SettingsView {
    let config = state.config();
    SettingsView {
        has_builtin_curseforge_key: Config::default().curseforge_api_key().is_some(),
        has_builtin_ms_client_id: Config::default().ms_client_id().is_some(),
        default_game_dir: state.paths.default_game_dir().display().to_string(),
        config,
    }
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, config: Config) -> Result<()> {
    config.save(&state.paths.config_file())?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

// ---------------------------------------------------------------------------
// Versions

#[tauri::command]
async fn get_version_manifest(state: State<'_, AppState>) -> Result<version::VersionManifest> {
    Ok(version::fetch_manifest(&state.http).await?)
}

#[tauri::command]
fn list_installed_versions(state: State<'_, AppState>) -> Vec<version::InstalledVersion> {
    version::list_installed(&state.game_dir())
}

/// Downloads everything a version needs, including its Java runtime when Java is automatic.
async fn install_files(app: &AppHandle, state: &AppState, id: &str) -> anyhow::Result<PathBuf> {
    let config = state.config();
    let game = state.game_dir();
    let progress = progress_reporter(app);
    let version = version::load_version(&game, id)?;
    version::ensure_game_files(
        &state.http,
        &game,
        &version,
        config.download_concurrency,
        &progress,
    )
    .await?;
    match config.java_path.trim() {
        "" => {
            let component = version.java_component();
            java::ensure_runtime(
                &state.http,
                &game,
                &component,
                config.download_concurrency,
                |d, t| progress(&format!("下载 Java 运行时 ({component})"), d, t),
            )
            .await
        }
        path => Ok(PathBuf::from(path)),
    }
}

#[tauri::command]
async fn install_version(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    version::install_vanilla_json(&state.http, &state.game_dir(), &id).await?;
    install_files(&app, &state, &id).await?;
    Ok(())
}

#[tauri::command]
fn delete_version(state: State<'_, AppState>, id: String) -> Result<()> {
    let dir = state.game_dir().version_dir(safe_name(&id)?);
    std::fs::remove_dir_all(&dir).with_context(|| format!("删除 {} 失败", dir.display()))?;
    Ok(())
}

#[tauri::command]
async fn get_fabric_loaders(
    state: State<'_, AppState>,
    game_version: String,
) -> Result<Vec<fabric::LoaderInfo>> {
    Ok(fabric::list_loaders(&state.http, &game_version).await?)
}

#[tauri::command]
async fn install_fabric(
    app: AppHandle,
    state: State<'_, AppState>,
    game_version: String,
    loader_version: String,
) -> Result<String> {
    let id = fabric::install(
        &state.http,
        &state.game_dir(),
        &game_version,
        &loader_version,
    )
    .await?;
    install_files(&app, &state, &id).await?;
    Ok(id)
}

// ---------------------------------------------------------------------------
// Accounts

#[tauri::command]
fn list_accounts(state: State<'_, AppState>) -> Vec<AccountView> {
    state.accounts.lock().unwrap().views()
}

#[tauri::command]
fn add_offline_account(state: State<'_, AppState>, name: String) -> Result<Vec<AccountView>> {
    let account = auth::offline_account(&name)?;
    state.update_accounts(|store| {
        store.selected = Some(account.id.clone());
        store.upsert(account);
        Ok(store.views())
    })
}

#[tauri::command]
fn remove_account(state: State<'_, AppState>, id: String) -> Result<Vec<AccountView>> {
    state.update_accounts(|store| {
        store.remove(&id);
        Ok(store.views())
    })
}

#[tauri::command]
fn select_account(state: State<'_, AppState>, id: String) -> Result<Vec<AccountView>> {
    state.update_accounts(|store| {
        store.select(&id)?;
        Ok(store.views())
    })
}

fn ms_client_id(state: &AppState) -> anyhow::Result<String> {
    state
        .config()
        .ms_client_id()
        .context("还没有配置微软 Client ID：请在 .env 中设置 MS_CLIENT_ID，或在设置页填写")
}

#[tauri::command]
async fn ms_login_start(state: State<'_, AppState>) -> Result<DeviceCode> {
    let client_id = ms_client_id(&state)?;
    state.login_cancelled.store(false, Ordering::SeqCst);
    Ok(auth::start_device_code(&state.http, &client_id).await?)
}

#[tauri::command]
async fn ms_login_finish(state: State<'_, AppState>, code: DeviceCode) -> Result<Vec<AccountView>> {
    let client_id = ms_client_id(&state)?;
    let account =
        auth::finish_device_code(&state.http, &client_id, &code, &state.login_cancelled).await?;
    state.update_accounts(|store| {
        store.selected = Some(account.id.clone());
        store.upsert(account);
        Ok(store.views())
    })
}

#[tauri::command]
fn ms_login_cancel(state: State<'_, AppState>) {
    state.login_cancelled.store(true, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Launching

/// Clears `game_running` unless the game actually started.
struct RunningGuard<'a> {
    flag: &'a AtomicBool,
    armed: bool,
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.flag.store(false, Ordering::SeqCst);
        }
    }
}

fn forward_output(app: AppHandle, reader: impl AsyncRead + Unpin + Send + 'static) {
    tauri::async_runtime::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut buf = Vec::new();
        // Read raw bytes: game output isn't guaranteed to be UTF-8 (e.g. GBK consoles on Windows).
        while let Ok(n) = reader.read_until(b'\n', &mut buf).await {
            if n == 0 {
                break;
            }
            log_line(&app, String::from_utf8_lossy(&buf).trim_end());
            buf.clear();
        }
    });
}

#[tauri::command]
async fn launch_game(app: AppHandle, state: State<'_, AppState>, version_id: String) -> Result<()> {
    safe_name(&version_id)?;
    if state.game_running.swap(true, Ordering::SeqCst) {
        Err(anyhow!("游戏已经在运行了"))?;
    }
    let mut guard = RunningGuard {
        flag: &state.game_running,
        armed: true,
    };

    let config = state.config();
    let game = state.game_dir();
    let mut account = state
        .accounts
        .lock()
        .unwrap()
        .selected()
        .cloned()
        .context("请先在“账号”页添加一个账号")?;
    if auth::needs_refresh(&account) {
        log_line(&app, "[启动器] 正在刷新微软登录...");
        account = auth::refresh(&state.http, &ms_client_id(&state)?, &account).await?;
        let refreshed = account.clone();
        state.update_accounts(|store| {
            store.upsert(refreshed);
            Ok(())
        })?;
    }

    log_line(&app, format!("[启动器] 检查 {version_id} 的游戏文件..."));
    let java = install_files(&app, &state, &version_id).await?;
    let version = version::load_version(&game, &version_id)?;
    let prepared = launch::prepare(
        game,
        version,
        account,
        launch::LaunchOptions {
            java,
            max_memory_mb: config.max_memory_mb,
            extra_jvm_args: config.jvm_args.clone(),
            ms_client_id: config.ms_client_id().unwrap_or_default(),
        },
    )
    .await?;
    log_line(&app, format!("[启动器] {}", prepared.redacted_command()));

    let mut command = tokio::process::Command::new(&prepared.java);
    command
        .args(&prepared.args)
        .current_dir(&prepared.working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .with_context(|| format!("无法启动 Java: {}", prepared.java.display()))?;

    if let Some(stdout) = child.stdout.take() {
        forward_output(app.clone(), stdout);
    }
    if let Some(stderr) = child.stderr.take() {
        forward_output(app.clone(), stderr);
    }

    let (kill_tx, kill_rx) = oneshot::channel();
    *state.game_kill.lock().unwrap() = Some(kill_tx);
    guard.armed = false;

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let status = tokio::select! {
            status = child.wait() => status,
            _ = kill_rx => {
                let _ = child.kill().await;
                child.wait().await
            }
        };
        let state = app.state::<AppState>();
        state.game_kill.lock().unwrap().take();
        state.game_running.store(false, Ordering::SeqCst);
        let code = status.ok().and_then(|s| s.code());
        let shown = code.map_or_else(|| "未知".to_string(), |c| c.to_string());
        log_line(&app, format!("[启动器] 游戏已退出，退出码 {shown}"));
        let _ = app.emit("game-exit", code);
    });
    Ok(())
}

#[tauri::command]
fn kill_game(state: State<'_, AppState>) {
    if let Some(tx) = state.game_kill.lock().unwrap().take() {
        let _ = tx.send(());
    }
}

// ---------------------------------------------------------------------------
// Mods

fn curseforge_key(state: &AppState) -> anyhow::Result<String> {
    state.config().curseforge_api_key().context(
        "还没有配置 CurseForge API Key：请在 .env 中设置 CURSEFORGE_API_KEY，或在设置页填写",
    )
}

#[tauri::command]
async fn cf_search(
    state: State<'_, AppState>,
    query: String,
    game_version: Option<String>,
    loader: Option<String>,
    index: u32,
) -> Result<Vec<curseforge::ModSummary>> {
    let cf = curseforge::CurseForge::new(&state.http, curseforge_key(&state)?);
    Ok(cf
        .search(&query, game_version.as_deref(), loader.as_deref(), index)
        .await?)
}

#[tauri::command]
async fn cf_install(
    state: State<'_, AppState>,
    mod_id: u64,
    version_id: String,
) -> Result<Vec<String>> {
    let game = state.game_dir();
    let info = version::installed_info(&game, &version_id)?;
    let loader = info
        .loader
        .context("这是原版游戏，请先在“版本”页为它安装 Fabric 等模组加载器")?;
    let cf = curseforge::CurseForge::new(&state.http, curseforge_key(&state)?);
    Ok(cf
        .install(
            mod_id,
            &info.game_version,
            &loader,
            &game.mods_dir(&version_id),
        )
        .await?)
}

#[tauri::command]
fn list_mods(state: State<'_, AppState>, version_id: String) -> Result<Vec<String>> {
    let dir = state.game_dir().mods_dir(safe_name(&version_id)?);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut mods: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".jar") || name.ends_with(".jar.disabled"))
        .collect();
    mods.sort_by_key(|name| name.to_lowercase());
    Ok(mods)
}

#[tauri::command]
fn remove_mod(state: State<'_, AppState>, version_id: String, file_name: String) -> Result<()> {
    let path = state
        .game_dir()
        .mods_dir(safe_name(&version_id)?)
        .join(safe_name(&file_name)?);
    std::fs::remove_file(path)?;
    Ok(())
}

/// Opens the game root, or a version's instance folder, in the system file manager.
#[tauri::command]
fn open_game_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    version_id: Option<String>,
) -> Result<()> {
    let game = state.game_dir();
    let dir = match version_id {
        Some(id) => game.instance_dir(safe_name(&id)?),
        None => game.root,
    };
    std::fs::create_dir_all(&dir)?;
    app.opener()
        .open_path(dir.display().to_string(), None::<&str>)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let paths = AppPaths { data_dir };
            app.manage(AppState {
                http: net::build_client(),
                config: Mutex::new(Config::load(&paths.config_file())),
                accounts: Mutex::new(AccountStore::load(&paths.accounts_file())),
                login_cancelled: AtomicBool::new(false),
                game_running: AtomicBool::new(false),
                game_kill: Mutex::new(None),
                paths,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_version_manifest,
            list_installed_versions,
            install_version,
            delete_version,
            get_fabric_loaders,
            install_fabric,
            list_accounts,
            add_offline_account,
            remove_account,
            select_account,
            ms_login_start,
            ms_login_finish,
            ms_login_cancel,
            launch_game,
            kill_game,
            cf_search,
            cf_install,
            list_mods,
            remove_mod,
            open_game_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
