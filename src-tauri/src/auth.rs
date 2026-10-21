//! Accounts: offline players and Microsoft login (device code flow → Xbox Live → XSTS → Minecraft).

use crate::config::write_json;
use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.signin offline_access";
const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    Microsoft,
    Offline,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub kind: AccountKind,
    pub name: String,
    /// Profile UUID without dashes.
    pub uuid: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds when `access_token` expires (Microsoft only).
    #[serde(default)]
    pub expires_at: i64,
}

/// What the frontend sees; tokens never leave the backend.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub kind: AccountKind,
    pub name: String,
    pub uuid: String,
    pub selected: bool,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountStore {
    pub accounts: Vec<Account>,
    pub selected: Option<String>,
}

impl AccountStore {
    pub fn load(path: &Path) -> AccountStore {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_json(path, self)?;
        // The file holds refresh tokens; keep it private to the user.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Adds or replaces an account (matched by id).
    pub fn upsert(&mut self, account: Account) {
        match self.accounts.iter_mut().find(|a| a.id == account.id) {
            Some(existing) => *existing = account,
            None => self.accounts.push(account),
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.accounts.retain(|a| a.id != id);
        if self.selected.as_deref() == Some(id) {
            self.selected = self.accounts.first().map(|a| a.id.clone());
        }
    }

    pub fn select(&mut self, id: &str) -> Result<()> {
        anyhow::ensure!(self.accounts.iter().any(|a| a.id == id), "账号不存在");
        self.selected = Some(id.to_string());
        Ok(())
    }

    pub fn selected(&self) -> Option<&Account> {
        let selected = self.selected.as_deref();
        self.accounts
            .iter()
            .find(|a| Some(a.id.as_str()) == selected)
            .or_else(|| self.accounts.first())
    }

    pub fn views(&self) -> Vec<AccountView> {
        let selected_id = self.selected().map(|a| a.id.clone());
        self.accounts
            .iter()
            .map(|a| AccountView {
                id: a.id.clone(),
                kind: a.kind,
                name: a.name.clone(),
                uuid: a.uuid.clone(),
                selected: Some(&a.id) == selected_id.as_ref(),
            })
            .collect()
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Offline

/// Same as Java's `UUID.nameUUIDFromBytes("OfflinePlayer:" + name)`, which servers in offline mode use.
pub fn offline_uuid(name: &str) -> String {
    let digest = Md5::digest(format!("OfflinePlayer:{name}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest);
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    hex::encode(bytes)
}

pub fn offline_account(name: &str) -> Result<Account> {
    let name = name.trim();
    let valid = (3..=16).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    anyhow::ensure!(valid, "离线用户名需为 3-16 位字母、数字或下划线");
    let uuid = offline_uuid(name);
    Ok(Account {
        id: format!("offline:{uuid}"),
        kind: AccountKind::Offline,
        name: name.to_string(),
        uuid,
        access_token: uuid::Uuid::new_v4().simple().to_string(),
        refresh_token: None,
        expires_at: 0,
    })
}

// ---------------------------------------------------------------------------
// Microsoft

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct MsTokens {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct OAuthError {
    error: String,
    error_description: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XboxToken {
    token: String,
    display_claims: DisplayClaims,
}

#[derive(Deserialize)]
struct DisplayClaims {
    xui: Vec<Xui>,
}

#[derive(Deserialize)]
struct Xui {
    uhs: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XstsError {
    x_err: u64,
}

#[derive(Deserialize)]
struct McLogin {
    access_token: String,
    expires_in: i64,
}

#[derive(Deserialize)]
struct McProfile {
    id: String,
    name: String,
}

fn oauth_error_message(body: &str) -> String {
    match serde_json::from_str::<OAuthError>(body) {
        Ok(err) => err.error_description.unwrap_or(err.error),
        Err(_) => body.to_string(),
    }
}

pub async fn start_device_code(client: &reqwest::Client, client_id: &str) -> Result<DeviceCode> {
    let resp = client
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", client_id), ("scope", SCOPE)])
        .send()
        .await?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("获取微软登录设备码失败: {}", oauth_error_message(&body));
    }
    let r: DeviceCodeResponse = resp.json().await?;
    Ok(DeviceCode {
        device_code: r.device_code,
        user_code: r.user_code,
        verification_uri: r.verification_uri,
        expires_in: r.expires_in,
        interval: r.interval,
    })
}

/// Polls until the user finishes signing in on the verification page, then logs into Minecraft.
pub async fn finish_device_code(
    client: &reqwest::Client,
    client_id: &str,
    code: &DeviceCode,
    cancelled: &AtomicBool,
) -> Result<Account> {
    let deadline = Instant::now() + Duration::from_secs(code.expires_in);
    let mut interval = code.interval.max(1);
    let tokens = loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if cancelled.load(Ordering::SeqCst) {
            bail!("登录已取消");
        }
        if Instant::now() > deadline {
            bail!("设备码已过期，请重新登录");
        }
        let resp = client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id),
                ("device_code", &code.device_code),
            ])
            .send()
            .await?;
        if resp.status().is_success() {
            break resp.json::<MsTokens>().await?;
        }
        let err: OAuthError = resp.json().await.context("无法解析微软登录响应")?;
        match err.error.as_str() {
            "authorization_pending" => {}
            "slow_down" => interval += 5,
            "authorization_declined" => bail!("你拒绝了授权"),
            "expired_token" => bail!("设备码已过期，请重新登录"),
            _ => bail!(
                "微软登录失败: {}",
                err.error_description.unwrap_or(err.error)
            ),
        }
    };
    login_minecraft(client, tokens).await
}

pub fn needs_refresh(account: &Account) -> bool {
    account.kind == AccountKind::Microsoft && now_secs() + 300 >= account.expires_at
}

/// Gets a fresh Minecraft token for a Microsoft account using its stored refresh token.
pub async fn refresh(
    client: &reqwest::Client,
    client_id: &str,
    account: &Account,
) -> Result<Account> {
    let refresh_token = account
        .refresh_token
        .as_deref()
        .context("账号没有保存刷新令牌，请重新登录")?;
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("scope", SCOPE),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!(
            "刷新微软令牌失败，请重新登录: {}",
            oauth_error_message(&body)
        );
    }
    login_minecraft(client, resp.json().await?).await
}

async fn login_minecraft(client: &reqwest::Client, tokens: MsTokens) -> Result<Account> {
    let xbl: XboxToken = client
        .post(XBL_URL)
        .json(&json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={}", tokens.access_token),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        }))
        .send()
        .await?
        .error_for_status()
        .context("Xbox Live 认证失败")?
        .json()
        .await?;
    let uhs = xbl
        .display_claims
        .xui
        .first()
        .map(|x| x.uhs.clone())
        .context("Xbox Live 响应缺少 uhs")?;

    let resp = client
        .post(XSTS_URL)
        .json(&json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl.token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        }))
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let code = resp
            .json::<XstsError>()
            .await
            .map(|e| e.x_err)
            .unwrap_or_default();
        bail!(xsts_error_message(code));
    }
    let xsts: XboxToken = resp
        .error_for_status()
        .context("XSTS 认证失败")?
        .json()
        .await?;

    let resp = client
        .post(MC_LOGIN_URL)
        .json(&json!({ "identityToken": format!("XBL3.0 x={uhs};{}", xsts.token) }))
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        bail!(
            "Minecraft 服务拒绝了这个 Azure 应用 (Invalid app registration)。\
             新注册的 Client ID 需要先向 Mojang 申请 Minecraft API 权限，详见 README"
        );
    }
    let mc: McLogin = resp
        .error_for_status()
        .context("登录 Minecraft 服务失败")?
        .json()
        .await?;

    let resp = client
        .get(MC_PROFILE_URL)
        .bearer_auth(&mc.access_token)
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("这个微软账号还没有购买 Minecraft: Java 版");
    }
    let profile: McProfile = resp
        .error_for_status()
        .context("获取 Minecraft 档案失败")?
        .json()
        .await?;

    Ok(Account {
        id: format!("msa:{}", profile.id),
        kind: AccountKind::Microsoft,
        name: profile.name,
        uuid: profile.id,
        access_token: mc.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: now_secs() + mc.expires_in,
    })
}

fn xsts_error_message(code: u64) -> String {
    match code {
        2148916233 => "这个微软账号还没有 Xbox 档案，请先登录 xbox.com 创建".into(),
        2148916235 => "Xbox Live 在你所在的国家/地区不可用".into(),
        2148916236 | 2148916237 => "该账号需要在 Xbox 网站完成成人验证".into(),
        2148916238 => "未成年账号需要由家长加入 Microsoft 家庭组后才能登录".into(),
        _ => format!("XSTS 认证失败 (XErr {code})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_java() {
        // Java: UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(offline_uuid("Notch"), "b50ad385829d3141a2167e7d7539ba7f");
    }

    #[test]
    fn offline_name_validation() {
        assert!(offline_account("Steve_01").is_ok());
        assert!(offline_account("ab").is_err());
        assert!(offline_account("名字").is_err());
    }

    #[test]
    fn store_selection() {
        let mut store = AccountStore::default();
        store.upsert(offline_account("Alex").unwrap());
        store.upsert(offline_account("Steve").unwrap());
        assert_eq!(store.selected().unwrap().name, "Alex");
        let steve = store.accounts[1].id.clone();
        store.select(&steve).unwrap();
        assert_eq!(store.selected().unwrap().name, "Steve");
        store.remove(&steve);
        assert_eq!(store.selected().unwrap().name, "Alex");
    }
}
