use anyhow::Result;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct SakuraConfig {
    pub server_address: String,
    pub server_uuid: String,
    pub device_token: String,
    #[serde(default)]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub firebase_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RequestBody {
    #[serde(rename = "serverUUID")]
    server_uuid: String,
    #[serde(rename = "deviceToken")]
    device_token: String,
}

#[derive(Debug, Clone, Serialize)]
struct ResponseBody {
    #[serde(rename = "serverUUID")]
    server_uuid: String,
    #[serde(rename = "deviceToken")]
    device_token: String,
    #[serde(rename = "requestUUID")]
    request_uuid: String,
    answer: bool,
    data: ResponseData,
}

#[derive(Debug, Clone, Serialize)]
struct ResponseData {
    lat: String,
    lon: String,
    #[serde(rename = "deviceModel")]
    device_model: String,
    #[serde(rename = "deviceName")]
    device_name: String,
    #[serde(rename = "modelName")]
    model_name: String,
    #[serde(rename = "externalIP")]
    external_ip: String,
    #[serde(rename = "serialNumber")]
    serial_number: String,
}

pub struct SakuraClient {
    client: Client,
    config: SakuraConfig,
}

impl SakuraClient {
    pub fn new(config: SakuraConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        Ok(Self { client, config })
    }
    
    pub fn from_config() -> Result<Self> {
        let config_path = crate::config::sakura_config_path();
        if !config_path.exists() {
            anyhow::bail!("Sakura config not found: {:?}", config_path);
        }
        let content = std::fs::read_to_string(&config_path)?;
        let config: SakuraConfig = serde_json::from_str(&content)?;
        Self::new(config)
    }
    
    fn build_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Content-Type", "application/json".parse().unwrap());
        if let Some(ref token) = self.config.auth_token {
            if token.len() > 10 {
                if let Ok(h) = format!("Bearer {}", token).parse() {
                    headers.insert("Authorization", h);
                }
            }
        }
        if let Some(ref fb) = self.config.firebase_token {
            if fb.len() > 10 {
                if let Ok(h) = fb.parse() {
                    headers.insert("x-firebase-token", h);
                }
            }
        }
        headers
    }
    
    fn base_url(&self) -> String {
        let mut base = self.config.server_address.clone();
        if !base.ends_with('/') {
            base.push('/');
        }
        base
    }
    
    pub async fn check_requests(&self) -> Result<String> {
        let url = format!("{}2fa/mobile/request", self.base_url());
        let body = RequestBody {
            server_uuid: self.config.server_uuid.clone(),
            device_token: self.config.device_token.clone(),
        };
        let resp = self.client.post(&url)
            .headers(self.build_headers())
            .json(&body)
            .send()
            .await?;
        let text = resp.text().await?;
        Ok(text)
    }
    
    pub async fn send_response(&self, request_uuid: &str, answer: bool) -> Result<Response> {
        let url = format!("{}2fa/mobile/response", self.base_url());
        let hostname = hostname::get()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let entry = ResponseBody {
            server_uuid: self.config.server_uuid.clone(),
            device_token: self.config.device_token.clone(),
            request_uuid: request_uuid.to_string(),
            answer,
            data: ResponseData {
                lat: "0.0".to_string(),
                lon: "0.0".to_string(),
                device_model: "linux".to_string(),
                device_name: hostname.clone(),
                model_name: hostname.clone(),
                external_ip: "".to_string(),
                serial_number: self.config.device_token.clone(),
            },
        };
        let resp = self.client.post(&url)
            .headers(self.build_headers())
            .json(&vec![entry])
            .send()
            .await?;
        Ok(resp)
    }
    
    pub async fn ping(&self, registry_token: Option<&str>) -> Result<String> {
        let url = format!("{}2fa/mobile/ping", self.base_url());
        let resp = self.client.post(&url)
            .headers(self.build_headers())
            .json(&serde_json::json!({"registryToken": registry_token}))
            .send()
            .await?;
        Ok(resp.text().await?)
    }

    pub async fn history(&self) -> Result<String> {
        let url = format!("{}2fa/mobile/history", self.base_url());
        let body = RequestBody {
            server_uuid: self.config.server_uuid.clone(),
            device_token: self.config.device_token.clone(),
        };
        let resp = self.client.post(&url)
            .headers(self.build_headers())
            .json(&body)
            .send()
            .await?;
        Ok(resp.text().await?)
    }

    pub fn registry_token(&self) -> Option<String> {
        self.config.auth_token.clone()
    }
}
