use anyhow::{Context, Result};
use reqwest::Certificate;
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tracing::{debug, info, warn};

const SCP_INTERMEDIATE: &[u8] =
    include_bytes!("../certs/globalsign-gcc-r6-alphassl-ca-2025.pem");

pub async fn do_login_er(url: &str, login: &str, password: &str, attempts: u32) -> Result<()> {
    info!("Starting login-er flow");

    // Sakura sometimes considers the agent "not ready" for a while; restart
    // its services if the local agent does not answer at all.
    crate::sakura::restart::bring_up_agent()
        .await
        .context("Sakura agent readiness")?;

    let client = build_client().context("Failed to build HTTP client")?;

    let agent = crate::sakura::agent::AgentClient::new()?;

    for attempt in 1..=attempts {
        let resolution = gain_creds(&agent).await;

        let (compliance, info_data) = match resolution {
            Ok(v) => v,
            Err(e) => {
                if attempt < attempts {
                    warn!(
                        "sakura agent did not deliver compliance/info (attempt {}/{}): {e:#}; restarting its services",
                        attempt, attempts
                    );
                    if let Err(re) = crate::sakura::restart::bring_up_agent().await {
                        warn!("sakura agent still down: {re:#}");
                    }
                    continue;
                }
                return Err(e);
            }
        };

        info!(
            "Got compliance ({} chars) and info ({} chars), attempt {}/{}",
            compliance.len(),
            info_data.len(),
            attempt,
            attempts
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

        if attempt < attempts {
            warn!(
                "SCP returned HTTP {}, restarting sakura services and retrying with fresh compliance/info",
                status
            );
            // The agent might have been the reason SCP rejected the POST:
            // bring the services back up before the next take.
            if let Err(re) = crate::sakura::restart::bring_up_agent().await {
                warn!("sakura agent still down: {re:#}");
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        }

        anyhow::bail!("Login-er failed: HTTP {} {}", status, text)
    }

    anyhow::bail!("Login-er exhausted all {} attempts", attempts)
}

// Both agent reads in one shot: either both succeed or the attempt fails.
async fn gain_creds(
    agent: &crate::sakura::agent::AgentClient,
) -> Result<(String, String)> {
    let compliance = agent
        .check()
        .await
        .context("Failed to get compliance from sakura agent")?;
    let info_data = agent
        .uid()
        .await
        .context("Failed to get info from sakura agent")?;
    Ok((compliance, info_data))
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
