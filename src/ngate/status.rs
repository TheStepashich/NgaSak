use anyhow::Result;

#[derive(Debug, serde::Serialize)]
pub struct NgateStatus {
    pub online: bool,
    pub raw: Option<String>,
}

// Offline markers win over online ones, so "Not Connected" / "disconnected"
// never match the online markers they contain. Table shared with
// tools/ngate-win-probe.ps1.
pub fn parse_status_output(text: String) -> (bool, Option<String>) {
    let lower = text.to_lowercase();
    if lower.contains("offline") || lower.contains("not online") || lower.contains("not connected")
        || lower.contains("disconnected") || lower.contains("not configured") || lower.contains("unconfigured") {
        return (false, Some(lower));
    }
    if lower.contains("online") || lower.contains("connected") || lower.contains("registered")
        || lower.contains("configuring") {
        return (true, Some(lower));
    }
    (false, Some(lower))
}

pub async fn status_raw(bin: &str) -> Result<(bool, Option<String>)> {
    use tokio::process::Command;
    let out = match Command::new(bin).arg("-I").output().await {
        Err(_) => return Ok((false, None)),
        Ok(out) => out,
    };
    // Exit code is not the verdict: the client may exit non-zero while printing
    // a parseable status line. Probe stdout+stderr and trust the markers.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    Ok(parse_status_output(format!("{stdout}\n{stderr}")))
}

#[cfg(target_os = "linux")]
pub async fn is_online(bin: &str) -> Result<bool> {
    let (online, _) = status_raw(bin).await?;
    Ok(online)
}

// On Windows the probe may be overridden with NGASAK_NGATE_STATUS_CMD and the
// client lives at a discovered path, so is_online must go through the backend.
#[cfg(target_os = "windows")]
pub async fn is_online(_bin: &str) -> Result<bool> {
    let (online, _) = crate::ngate::windows::WindowsNgateBackend::new().status_detail().await;
    Ok(online)
}

// Same for get_status on Windows: honor the user's probe setup (NGASAK_NGATE_STATUS_CMD,
// discovered client path) and report raw output instead of a silent probe.
#[cfg(target_os = "windows")]
pub async fn get_status(_bin: &str) -> Result<NgateStatus> {
    let (online, raw) = crate::ngate::windows::WindowsNgateBackend::new()
        .status_detail()
        .await;
    Ok(NgateStatus { online, raw })
}

#[cfg(not(target_os = "windows"))]
pub async fn get_status(bin: &str) -> Result<NgateStatus> {
    let (online, raw) = status_raw(bin).await?;
    Ok(NgateStatus { online, raw })
}
