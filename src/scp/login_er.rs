use anyhow::{Context, Result};
use reqwest::Certificate;
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tracing::{debug, info, warn};

const SCP_INTERMEDIATE: &[u8] =
    include_bytes!("../certs/globalsign-gcc-r6-alphassl-ca-2025.pem");

pub async fn do_login_er(url: &str, login: &str, password: &str) -> Result<()> {
    info!("Starting login-er flow");

    let client = build_client().context("Failed to build HTTP client")?;

    let agent = crate::sakura::agent::AgentClient::new()?;

    const ATTEMPTS: u32 = 3;
    for attempt in 1..=ATTEMPTS {
        let compliance = agent
            .check()
            .await
            .context("Failed to get compliance from sakura agent")?;
        let info_data = agent
            .uid()
            .await
            .context("Failed to get info from sakura agent")?;

        info!(
            "Got compliance ({} chars) and info ({} chars), attempt {}/{}",
            compliance.len(),
            info_data.len(),
            attempt,
            ATTEMPTS
        );

        let body = json!({
            "login": login,
            "password": password,
            "compliance": compliance,
            "info": info_data
        });

        debug!("POST to {}", url);

        let resp = client
            .post(url)
            .header("Content-Type", "application/json;charset=UTF-8")
            .header("Accept", "application/json, text/plain, */*")
            .header("Origin", "https://scp.mos.ru")
            .header("Referer", "https://scp.mos.ru/login-er")
            .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        info!(
            "Response: HTTP {} {}",
            status,
            &text[..text.len().min(200)]
        );

        if status.is_success() && text.contains("success") {
            info!("Login-er successful!");
            return Ok(());
        }

        if status.is_server_error() && attempt < ATTEMPTS {
            warn!(
                "SCP returned HTTP {}, retrying with fresh compliance/info in 15s",
                status
            );
            tokio::time::sleep(Duration::from_secs(15)).await;
            continue;
        }

        anyhow::bail!("Login-er failed: HTTP {} {}", status, text)
    }

    unreachable!()
}

fn build_client() -> Result<Client> {
    let mut builder = Client::builder()
        .cookie_store(true)
        .timeout(Duration::from_secs(60));

    match Certificate::from_pem(SCP_INTERMEDIATE) {
        Ok(cert) => builder = builder.add_root_certificate(cert),
        Err(e) => debug!("Failed to load embedded SCP intermediate cert: {e}"),
    }

    Ok(builder.build()?)
}
