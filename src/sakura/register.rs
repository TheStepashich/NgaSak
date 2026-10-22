use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use tracing::{info, warn};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SakuraConfigFile {
    #[serde(default)]
    pub server_address: String,
    #[serde(default)]
    pub server_uuid: String,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub device_token: String,
    #[serde(default)]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub firebase_token: Option<String>,
    #[serde(default)]
    pub registry_code_pending: Option<String>,
    #[serde(default)]
    pub registry_token: Option<String>,
    #[serde(default)]
    pub registry_code: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    status: i32,
    #[serde(default)]
    msg: Option<String>,
    #[serde(rename = "authToken")]
    auth_token: Option<String>,
}

pub fn config_path() -> PathBuf {
    crate::config::config_path().unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".config/ngasak/config.toml"))
}

pub fn load_config() -> Result<SakuraConfigFile> {
    let ngasak_cfg = crate::config::load_config()?;
    if ngasak_cfg.sakura.server_address.is_some() && ngasak_cfg.sakura.device_token.is_some() {
        return Ok(SakuraConfigFile {
            server_address: ngasak_cfg.sakura.server_address.clone().unwrap_or_default(),
            server_uuid: ngasak_cfg.sakura.server_uuid.clone().unwrap_or_default(),
            server_name: ngasak_cfg.sakura.server_name.clone(),
            device_token: ngasak_cfg.sakura.device_token.clone().unwrap_or_default(),
            auth_token: ngasak_cfg.sakura.auth_token.clone(),
            firebase_token: ngasak_cfg.sakura.firebase_token.clone(),
            registry_code_pending: None,
            registry_token: None,
            registry_code: None,
            extra: serde_json::Map::new(),
        });
    }
    // fallback
    let path = dirs::home_dir().unwrap_or_default().join(".config/sakura2fa/config.json");
    if !path.exists() {
        anyhow::bail!("Config not found: {}", path.display());
    }
    let content = fs::read_to_string(&path).context("Failed to read sakura config")?;
    serde_json::from_str(&content).context("Failed to parse sakura config")
}

pub fn save_config(cfg: &SakuraConfigFile) -> Result<()> {
    let mut ngasak_cfg = crate::config::load_config()?;
    ngasak_cfg.sakura.server_address = Some(cfg.server_address.clone());
    ngasak_cfg.sakura.server_uuid = Some(cfg.server_uuid.clone());
    ngasak_cfg.sakura.device_token = Some(cfg.device_token.clone());
    ngasak_cfg.sakura.auth_token = cfg.auth_token.clone();
    ngasak_cfg.sakura.firebase_token = cfg.firebase_token.clone();
    ngasak_cfg.sakura.server_name = cfg.server_name.clone();
    crate::config::save_config(&ngasak_cfg)?;
    // also keep legacy for compatibility
    let path = dirs::home_dir().unwrap_or_default().join(".config/sakura2fa/config.json");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let content = serde_json::to_string_pretty(cfg)?;
    let _ = fs::write(&path, content);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    info!("Saved to {}", crate::config::config_path()?.display());
    Ok(())
}

pub fn parse_qr_uri(uri: &str) -> Result<(String, String, String, Option<String>)> {
    let parsed = Url::parse(uri).context("Invalid QR URI")?;
    if parsed.scheme() != "sakura" || parsed.host_str() != Some("register") {
        anyhow::bail!("Expected sakura://register?... URI");
    }

    let mut server_name = String::new();
    let mut server_address = String::new();
    let mut server_uuid = String::new();
    let mut registry_code: Option<String> = None;

    for (key, value) in parsed.query_pairs() {
        let raw = value.replace(' ', "");
        let pad = "=".repeat((4 - raw.len() % 4) % 4);
        let decoded = BASE64
            .decode(format!("{raw}{pad}"))
            .context("Invalid base64")?;
        let s = String::from_utf8(decoded).context("Invalid utf-8")?;

        match key.as_ref() {
            "servername" => server_name = s,
            "serveraddr" => server_address = s,
            "servertoken" => server_uuid = s,
            "registrationtoken" => registry_code = Some(s),
            _ => {}
        }
    }

    Ok((server_name, server_address, server_uuid, registry_code))
}

pub async fn register(
    server_url: Option<String>,
    server_uuid: Option<String>,
    server_name: Option<String>,
    registry_code: Option<String>,
    qr: Option<String>,
) -> Result<()> {
    let mut cfg = if config_path().exists() {
        load_config().unwrap_or_else(|_| SakuraConfigFile {
            server_address: String::new(),
            server_uuid: String::new(),
            server_name: None,
            device_token: String::new(),
            auth_token: None,
            firebase_token: None,
            registry_code_pending: None,
            registry_token: None,
            registry_code: None,
            extra: serde_json::Map::new(),
        })
    } else {
        SakuraConfigFile {
            server_address: String::new(),
            server_uuid: String::new(),
            server_name: None,
            device_token: String::new(),
            auth_token: None,
            firebase_token: None,
            registry_code_pending: None,
            registry_token: None,
            registry_code: None,
            extra: serde_json::Map::new(),
        }
    };

    if let Some(qr_uri) = qr {
        let (sn, sa, su, rc) = parse_qr_uri(&qr_uri)?;
        if cfg.server_address.is_empty() {
            cfg.server_address = sa;
        }
        if cfg.server_uuid.is_empty() {
            cfg.server_uuid = su;
        }
        if cfg.server_name.is_none() {
            cfg.server_name = Some(sn);
        }
        if rc.is_some() {
            cfg.registry_code_pending = rc;
        }
    }

    if let Some(v) = server_url {
        cfg.server_address = v;
    }
    if let Some(v) = server_uuid {
        cfg.server_uuid = v;
    }
    if let Some(v) = server_name {
        cfg.server_name = Some(v);
    }

    if cfg.server_address.is_empty() || cfg.server_uuid.is_empty() {
        anyhow::bail!("Need --server-url and --server-uuid (or --qr sakura://register?...)");
    }

    if cfg.device_token.is_empty() {
        cfg.device_token = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    }

    let code = registry_code
        .or_else(|| cfg.registry_code_pending.clone())
        .or_else(|| cfg.registry_code.clone());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let mut base = cfg.server_address.clone();
    if !base.ends_with('/') {
        base.push('/');
    }
    let url = format!("{}2fa/mobile/register", base);

    info!("Registering device on {}", cfg.server_address);

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(ref token) = cfg.auth_token {
        if token.len() > 10 {
            if let Ok(h) = format!("Bearer {token}").parse() {
                headers.insert(AUTHORIZATION, h);
            }
        }
    }
    if let Some(ref fb) = cfg.firebase_token {
        if fb.len() > 10 {
            if let Ok(h) = fb.parse() {
                headers.insert("x-firebase-token", h);
            }
        }
    }

    let mut body = json!({
        "serverUUID": cfg.server_uuid,
        "deviceToken": cfg.device_token,
        "mandatoryFields": null,
    });
    if let Some(ref c) = code {
        body["registryCode"] = json!(c);
    }

    let resp = client.post(&url).headers(headers.clone()).json(&body).send().await?;
    if resp.status().as_u16() >= 400 {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Server error HTTP {}: {}", status, text);
    }

    let mut reg: RegisterResponse = resp.json().await?;
    info!("Server response: status={} msg={:?}", reg.status, reg.msg);

    if reg.status == 1 {
        info!("Server requires registration code from email");
        let input = inquire::Text::new("Enter registration code:").prompt()?;
        body["registryCode"] = json!(input.trim());
        let resp = client.post(&url).headers(headers).json(&body).send().await?;
        if resp.status().as_u16() >= 400 {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Server error HTTP {}: {}", status, text);
        }
        reg = resp.json().await?;
        info!("Server response: status={} msg={:?}", reg.status, reg.msg);
    }

    if reg.status != 0 {
        anyhow::bail!("Registration not completed: {}", reg.msg.as_deref().unwrap_or("unknown"));
    }

    let auth = reg
        .auth_token
        .or_else(|| {
            warn!("No authToken in response");
            None
        })
        .ok_or_else(|| anyhow::anyhow!("Server did not return authToken"))?;

    cfg.auth_token = Some(auth);
    cfg.registry_code_pending = None;
    save_config(&cfg)?;

    info!("Registration complete. authToken saved to {}", config_path().display());
    info!("device_token = {}", cfg.device_token);
    Ok(())
}
