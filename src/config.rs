use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ngate: NgateConfig,
    #[serde(default)]
    pub sakura: SakuraConfig,
    #[serde(default)]
    pub scp: ScpConfig,
    #[serde(default)]
    pub run: RunConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NgateConfig {
    #[serde(default = "default_ngate_bin")]
    pub binary: String,
    #[serde(default = "default_true")]
    pub use_dbus: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SakuraConfig {
    #[serde(default)]
    pub config_path: Option<PathBuf>,
    #[serde(default = "default_sakura_poll_interval")]
    pub poll_interval: f64,
    #[serde(default = "default_sakura_timeout")]
    pub check_timeout: u64,
    #[serde(default)]
    pub server_address: Option<String>,
    #[serde(default)]
    pub server_uuid: Option<String>,
    #[serde(default)]
    pub device_token: Option<String>,
    #[serde(default)]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub firebase_token: Option<String>,
    #[serde(default)]
    pub server_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpConfig {
    #[serde(default = "default_scp_url")]
    pub login_url: String,
    #[serde(default = "default_post_delay")]
    pub post_delay: u64,
    #[serde(default)]
    pub login: Option<String>,
}

fn default_ngate_bin() -> String { "/opt/cprongate/ngateclient".to_string() }
fn default_true() -> bool { true }
fn default_sakura_poll_interval() -> f64 { 2.0 }
fn default_sakura_timeout() -> u64 { 10 }
fn default_scp_url() -> String { "https://scp.mos.ru/api/login-er".to_string() }
fn default_post_delay() -> u64 { 30 }

impl Default for Config {
    fn default() -> Self {
        Self {
            ngate: NgateConfig {
                binary: default_ngate_bin(),
                use_dbus: default_true(),
            },
            sakura: SakuraConfig {
                config_path: None,
                poll_interval: default_sakura_poll_interval(),
                check_timeout: default_sakura_timeout(),
                server_address: None,
                server_uuid: None,
                device_token: None,
                auth_token: None,
                firebase_token: None,
                server_name: None,
            },
            scp: ScpConfig {
                login_url: default_scp_url(),
                post_delay: default_post_delay(),
                login: None,
            },
            run: RunConfig {
                post_command: None,
                post_run: None,
            },
        }
    }
}

impl Default for NgateConfig {
    fn default() -> Self { Config::default().ngate }
}
impl Default for SakuraConfig {
    fn default() -> Self { Config::default().sakura }
}
impl Default for ScpConfig {
    fn default() -> Self { Config::default().scp }
}

pub fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("Failed to get config dir")?
        .join("ngasak");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn load_config() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        let cfg = Config::default();
        save_config(&cfg)?;
        return Ok(cfg);
    }
    let content = std::fs::read_to_string(&path)?;
    let cfg: Config = toml::from_str(&content).context("Failed to parse config")?;
    Ok(cfg)
}

pub fn save_config(cfg: &Config) -> Result<()> {
    let path = config_path()?;
    let content = toml::to_string_pretty(cfg)?;
    std::fs::write(&path, content)?;
    Ok(())
}

pub fn sakura_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".config/sakura2fa/config.json")
}

pub fn load_sakura_merged() -> Result<crate::sakura::client::SakuraConfig> {
    let cfg = load_config()?;
    // Prefer ngasak config
    if cfg.sakura.server_address.is_some() && cfg.sakura.server_uuid.is_some() && cfg.sakura.device_token.is_some() {
        return Ok(crate::sakura::client::SakuraConfig {
            server_address: cfg.sakura.server_address.clone().unwrap_or_default(),
            server_uuid: cfg.sakura.server_uuid.clone().unwrap_or_default(),
            device_token: cfg.sakura.device_token.clone().unwrap_or_default(),
            auth_token: cfg.sakura.auth_token.clone(),
            firebase_token: cfg.sakura.firebase_token.clone(),
        });
    }
    // Fallback to legacy json
    let path = sakura_config_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        return Ok(serde_json::from_str(&content)?);
    }
    anyhow::bail!("Sakura config not found")
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    #[serde(default)]
    pub post_command: Option<String>,
    #[serde(default)]
    pub post_run: Option<String>,
}


impl Default for RunConfig {
    fn default() -> Self {
        Self {
            post_command: None,
            post_run: None,
        }
    }
}
