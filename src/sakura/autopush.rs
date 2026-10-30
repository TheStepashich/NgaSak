use anyhow::Result;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub struct Autopush {
    client: crate::sakura::client::SakuraClient,
    seen: HashSet<String>,
    interval: Duration,
    expected_login: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RequestPayload {
    #[serde(default)]
    payload: Vec<PushRequest>,
}

#[derive(Debug, Deserialize)]
struct PushRequest {
    #[serde(default)]
    guid: String,
    #[serde(default)]
    message: String,
}

impl Autopush {
    pub fn new(expected_login: Option<String>, interval: Duration) -> Result<Self> {
        let client = crate::sakura::client::SakuraClient::from_config()?;
        Ok(Self {
            client,
            seen: HashSet::new(),
            interval,
            expected_login,
        })
    }

    pub async fn run_forever(mut self) -> Result<()> {
        match &self.expected_login {
            Some(login) => info!("Sakura autopush started (login filter: {})", login),
            None => warn!("Sakura autopush started without login filter - approving every request"),
        }
        let token = CancellationToken::new();
        let token2 = token.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                token2.cancel();
            }
        });
        self.run_until(token).await
    }

    pub async fn run_until(&mut self, shutdown: CancellationToken) -> Result<()> {
        loop {
            if shutdown.is_cancelled() {
                debug!("Sakura autopush stopped");
                return Ok(());
            }
            if let Err(e) = self.check_and_approve().await {
                debug!("Sakura request check failed: {e:#}");
            }
            tokio::select! {
                _ = tokio::time::sleep(self.interval) => {}
                _ = shutdown.cancelled() => {
                    debug!("Sakura autopush stopped");
                    return Ok(());
                }
            }
        }
    }

    pub async fn check_and_approve(&mut self) -> Result<usize> {
        let text = self.client.check_requests().await?;
        if text.trim().is_empty() {
            return Ok(0);
        }
        let data: RequestPayload = serde_json::from_str(&text)?;
        let mut approved = 0;

        for request in data.payload {
            if request.guid.is_empty() || self.seen.contains(&request.guid) {
                continue;
            }
            self.seen.insert(request.guid.clone());

            if !self.matches(&request.message) {
                warn!(
                    "Sakura push {} ignored (login filter `{}`): {}",
                    request.guid,
                    self.expected_login.as_deref().unwrap_or("*"),
                    request.message
                );
                continue;
            }

            info!("Sakura push: {} msg={}", request.guid, request.message);
            match self.client.send_response(&request.guid, true).await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    info!("Push approved: HTTP {} {}", status, body.trim());
                    approved += 1;
                }
                Err(e) => warn!("Failed to approve push {}: {e:#}", request.guid),
            }
        }
        Ok(approved)
    }

    fn matches(&self, message: &str) -> bool {
        match &self.expected_login {
            Some(login) => message.to_lowercase().contains(&login.to_lowercase()),
            None => true,
        }
    }
}
