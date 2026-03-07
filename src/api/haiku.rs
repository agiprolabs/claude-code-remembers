use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("no API credentials found")]
    NoCredentials,
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("parse error: {0}")]
    Parse(String),
}

#[derive(Debug, Clone)]
enum AuthMethod {
    ApiKey(String),
    OAuthToken(String),
}

pub struct HaikuClient {
    http: reqwest::Client,
    auth: Option<AuthMethod>,
    base_url: String,
}

#[derive(Debug, Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

impl HaikuClient {
    pub fn from_env() -> Result<Self, ApiError> {
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com".to_string());

        // Try API key first
        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY")
            .or_else(|_| std::env::var("CLAUDE_CODE_API_KEY"))
        {
            info!("Using API key authentication");
            return Ok(Self {
                http: reqwest::Client::new(),
                auth: Some(AuthMethod::ApiKey(api_key)),
                base_url,
            });
        }

        // Try OAuth token from macOS keychain
        if let Some(token) = Self::read_oauth_from_keychain() {
            info!("Using OAuth token from keychain");
            return Ok(Self {
                http: reqwest::Client::new(),
                auth: Some(AuthMethod::OAuthToken(token)),
                base_url,
            });
        }

        Err(ApiError::NoCredentials)
    }

    #[cfg(target_os = "macos")]
    fn read_oauth_from_keychain() -> Option<String> {
        let output = std::process::Command::new("security")
            .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let json_str = String::from_utf8(output.stdout).ok()?.trim().to_string();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).ok()?;
        let token = parsed
            .get("claudeAiOauth")?
            .get("accessToken")?
            .as_str()?
            .to_string();

        if token.is_empty() {
            None
        } else {
            Some(token)
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn read_oauth_from_keychain() -> Option<String> {
        None
    }

    /// Create a client that always fails (for offline/no-credentials mode).
    pub fn unavailable() -> Self {
        Self {
            http: reqwest::Client::new(),
            auth: None,
            base_url: String::new(),
        }
    }

    pub fn is_available(&self) -> bool {
        self.auth.is_some()
    }

    pub async fn complete(&self, system: &str, user_msg: &str) -> Result<String, ApiError> {
        let auth = self.auth.as_ref().ok_or(ApiError::NoCredentials)?;

        let request = ApiRequest {
            model: "claude-haiku-4-5-20251001".to_string(),
            max_tokens: 1024,
            system: system.to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: user_msg.to_string(),
            }],
        };

        let mut retries = 0;
        let max_retries = 3;

        loop {
            let mut req_builder = self
                .http
                .post(format!("{}/v1/messages", self.base_url))
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json");

            req_builder = match auth {
                AuthMethod::ApiKey(key) => req_builder.header("x-api-key", key),
                AuthMethod::OAuthToken(token) => {
                    req_builder.header("Authorization", format!("Bearer {token}"))
                }
            };

            let resp = req_builder.json(&request).send().await?;

            let status = resp.status().as_u16();

            if status == 200 {
                let body: ApiResponse = resp.json().await?;
                let text = body
                    .content
                    .iter()
                    .filter_map(|b| {
                        if b.block_type == "text" {
                            b.text.as_deref()
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("");
                return Ok(text);
            }

            // Retry on 429 (rate limit) or 5xx (server error)
            if (status == 429 || status >= 500) && retries < max_retries {
                retries += 1;
                let delay = std::time::Duration::from_millis(1000 * 2u64.pow(retries));
                warn!("API returned {status}, retrying in {:?} ({retries}/{max_retries})", delay);
                tokio::time::sleep(delay).await;
                continue;
            }

            let body_text = resp.text().await.unwrap_or_default();
            error!("API error {status}: {body_text}");
            return Err(ApiError::Api {
                status,
                message: body_text,
            });
        }
    }
}
