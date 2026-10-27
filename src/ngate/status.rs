use anyhow::Result;

#[derive(Debug, serde::Serialize)]
pub struct NgateStatus {
    pub online: bool,
    pub raw: Option<String>,
}

#[cfg(target_os = "linux")]
pub async fn is_online(bin: &str) -> Result<bool> {
    use tokio::process::Command;
    let out = Command::new(bin).arg("-I").output().await?;
    if !out.status.success() {
        return Ok(false);
    }
    let s = String::from_utf8_lossy(&out.stdout).to_lowercase();
    if s.contains("online") {
        return Ok(true);
    }
    if s.contains("offline") {
        return Ok(false);
    }
    Ok(s.contains("online"))
}

#[cfg(target_os = "windows")]
pub async fn is_online(bin: &str) -> Result<bool> {
    let cfg = crate::config::load_config()?;
    let backend = crate::ngate::create_backend(&cfg);
    backend.status().await
}

pub async fn get_status(bin: &str) -> Result<NgateStatus> {
    let online = is_online(bin).await?;
    Ok(NgateStatus { online, raw: None })
}
