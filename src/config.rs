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
            },
            scp: ScpConfig {
                login_url: default_scp_url(),
                post_delay: default_post_delay(),
                login: None,
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
