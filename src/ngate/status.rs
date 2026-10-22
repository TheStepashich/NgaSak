use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::debug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NgateStatus {
    #[serde(rename = "assignedIP")]
    pub assigned_ip: Option<String>,
    #[serde(rename = "endTimeConstraint")]
    pub end_time_constraint: Option<i64>,
    #[serde(rename = "lastRefreshTime")]
    pub last_refresh_time: Option<serde_json::Value>,
    #[serde(rename = "persistentSessionID")]
    pub persistent_session_id: Option<serde_json::Value>,
    #[serde(rename = "sessionID")]
    pub session_id: Option<String>,
    pub status: String,
    #[serde(rename = "vpnGateIP")]
    pub vpn_gate_ip: Option<String>,
}

pub async fn get_status(binary: &str) -> Result<NgateStatus> {
    debug!("Checking ngate status with {}", binary);
    let output = Command::new(binary)
        .arg("-I")
        .output()
        .await
        .context(format!("Failed to run {}", binary))?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let status: NgateStatus = serde_json::from_str(&stdout).context("Failed to parse ngate status")?;
    Ok(status)
}

pub async fn is_online(binary: &str) -> Result<bool> {
    match get_status(binary).await {
        Ok(s) => Ok(s.status == "Online"),
        Err(_) => Ok(false),
    }
}
