//! OAuth2 authentication for MCP remote servers

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// OAuth2 configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Config {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub authorization_url: String,
    pub token_url: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
}

impl OAuth2Config {
    pub fn new(
        client_id: String,
        authorization_url: String,
        token_url: String,
        redirect_uri: String,
    ) -> Self {
        Self {
            client_id,
            client_secret: None,
            authorization_url,
            token_url,
            redirect_uri,
            scopes: Vec::new(),
        }
    }

    pub fn with_client_secret(mut self, secret: String) -> Self {
        self.client_secret = Some(secret);
        self
    }

    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    /// Generate authorization URL
    pub fn authorization_url(&self, state: &str) -> String {
        let scopes = self.scopes.join(" ");
        format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
            self.authorization_url,
            self.client_id,
            self.redirect_uri,
            urlencoding::encode(&scopes),
            state
        )
    }
}

/// OAuth2 token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Token {
    pub access_token: String,
    pub token_type: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

impl OAuth2Token {
    pub fn is_expired(&self) -> bool {
        // Simple check - in production, track issue time
        false
    }
}

/// OAuth2 client
pub struct OAuth2Client {
    config: OAuth2Config,
    client: reqwest::Client,
    token: Option<OAuth2Token>,
}

impl OAuth2Client {
    pub fn new(config: OAuth2Config) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            token: None,
        }
    }

    pub fn config(&self) -> &OAuth2Config {
        &self.config
    }

    pub fn token(&self) -> Option<&OAuth2Token> {
        self.token.as_ref()
    }

    pub fn set_token(&mut self, token: OAuth2Token) {
        self.token = Some(token);
    }

    /// Exchange authorization code for token
    pub async fn exchange_code(&mut self, code: &str) -> Result<OAuth2Token> {
        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code");
        params.insert("code", code);
        params.insert("redirect_uri", &self.config.redirect_uri);
        params.insert("client_id", &self.config.client_id);

        if let Some(secret) = &self.config.client_secret {
            params.insert("client_secret", secret);
        }

        let response = self
            .client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow::anyhow!("Token exchange failed: {}", error));
        }

        let token: OAuth2Token = response.json().await?;
        self.token = Some(token.clone());
        Ok(token)
    }

    /// Refresh access token
    pub async fn refresh_token(&mut self) -> Result<OAuth2Token> {
        let refresh_token = self
            .token
            .as_ref()
            .and_then(|t| t.refresh_token.as_deref())
            .ok_or_else(|| anyhow::anyhow!("No refresh token available"))?;

        let mut params = HashMap::new();
        params.insert("grant_type", "refresh_token");
        params.insert("refresh_token", refresh_token);
        params.insert("client_id", &self.config.client_id);

        if let Some(secret) = &self.config.client_secret {
            params.insert("client_secret", secret);
        }

        let response = self
            .client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow::anyhow!("Token refresh failed: {}", error));
        }

        let token: OAuth2Token = response.json().await?;
        self.token = Some(token.clone());
        Ok(token)
    }

    /// Get authorization header
    pub fn auth_header(&self) -> Option<String> {
        self.token
            .as_ref()
            .map(|t| format!("{} {}", t.token_type, t.access_token))
    }

    /// Check if authenticated
    pub fn is_authenticated(&self) -> bool {
        self.token.is_some() && !self.token.as_ref().unwrap().is_expired()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oauth2_config_creation() {
        let config = OAuth2Config::new(
            "client_id".into(),
            "https://auth.example.com/authorize".into(),
            "https://auth.example.com/token".into(),
            "http://localhost:8080/callback".into(),
        );
        assert_eq!(config.client_id, "client_id");
        assert!(config.scopes.is_empty());
    }

    #[test]
    fn test_oauth2_config_with_scopes() {
        let config = OAuth2Config::new(
            "client_id".into(),
            "https://auth.example.com/authorize".into(),
            "https://auth.example.com/token".into(),
            "http://localhost:8080/callback".into(),
        )
        .with_scopes(vec!["read".into(), "write".into()])
        .with_client_secret("secret".into());

        assert_eq!(config.scopes.len(), 2);
        assert!(config.client_secret.is_some());
    }

    #[test]
    fn test_authorization_url() {
        let config = OAuth2Config::new(
            "client_id".into(),
            "https://auth.example.com/authorize".into(),
            "https://auth.example.com/token".into(),
            "http://localhost:8080/callback".into(),
        )
        .with_scopes(vec!["mcp".into()]);

        let url = config.authorization_url("state123");
        assert!(url.contains("client_id=client_id"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("state=state123"));
        assert!(url.contains("scope=mcp"));
    }

    #[test]
    fn test_oauth2_client_creation() {
        let config = OAuth2Config::new(
            "client_id".into(),
            "https://auth.example.com/authorize".into(),
            "https://auth.example.com/token".into(),
            "http://localhost:8080/callback".into(),
        );
        let client = OAuth2Client::new(config);
        assert!(!client.is_authenticated());
        assert!(client.token().is_none());
    }

    #[test]
    fn test_oauth2_token_expiry() {
        let token = OAuth2Token {
            access_token: "token".into(),
            token_type: "Bearer".into(),
            expires_in: None,
            refresh_token: None,
            scope: None,
        };
        assert!(!token.is_expired());
    }

    #[test]
    fn test_auth_header() {
        let config = OAuth2Config::new(
            "client_id".into(),
            "https://auth.example.com/authorize".into(),
            "https://auth.example.com/token".into(),
            "http://localhost:8080/callback".into(),
        );
        let mut client = OAuth2Client::new(config);
        assert!(client.auth_header().is_none());

        client.set_token(OAuth2Token {
            access_token: "abc123".into(),
            token_type: "Bearer".into(),
            expires_in: None,
            refresh_token: None,
            scope: None,
        });

        assert_eq!(client.auth_header(), Some("Bearer abc123".into()));
    }
}
