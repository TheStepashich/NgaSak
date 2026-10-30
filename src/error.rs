use thiserror::Error;

#[derive(Error, Debug)]
pub enum NgaSakError {
    #[error("Ngate error: {0}")]
    Ngate(String),
    #[error("D-Bus error: {0}")]
    #[cfg(target_os = "linux")]
    DBus(#[from] zbus::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Sakura error: {0}")]
    Sakura(String),
    #[error("SCP login error: {0}")]
    ScpLogin(String),
    #[error("Config error: {0}")]
    Config(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, NgaSakError>;
