use anyhow::Result;
use reqwest::Client;
use tracing::debug;

pub struct AgentClient {
    client: Client,
    base_url: String,
}

impl AgentClient {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        Ok(Self {
            client,
            base_url: "http://127.0.0.1:4567".to_string(),
        })
    }
    
    pub async fn check(&self) -> Result<String> {
        debug!("Getting compliance from sakura agent");
        let resp = self.client.get(format!("{}/v2/check", self.base_url)).send().await?;
        let text = resp.text().await?;
        Ok(text)
    }
    
    pub async fn uid(&self) -> Result<String> {
        debug!("Getting info from sakura agent");
        let resp = self.client.get(format!("{}/uid", self.base_url)).send().await?;
        let text = resp.text().await?;
        Ok(text)
    }
    
    pub async fn network_interfaces(&self) -> Result<String> {
        let resp = self.client.get(format!("{}/network-interfaces", self.base_url)).send().await?;
        let text = resp.text().await?;
        Ok(text)
    }
}
