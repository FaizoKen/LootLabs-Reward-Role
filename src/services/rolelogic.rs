use std::time::Duration;

use crate::error::AppError;

/// Minimal RoleLogic User Management API client. We only need add/remove
/// for this plugin — no bulk replace, no chunked upload (our roster grows
/// one user at a time as completions roll in).
#[derive(Clone)]
pub struct RoleLogicClient {
    http: reqwest::Client,
    base_url: String,
}

impl RoleLogicClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(8)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http,
            base_url: "https://api-rolelogic.faizo.net".to_string(),
        }
    }

    fn user_url(&self, guild_id: &str, role_id: &str, user_id: &str) -> String {
        format!(
            "{}/api/role-link/{}/{}/users/{}",
            self.base_url, guild_id, role_id, user_id
        )
    }

    pub async fn add_user(
        &self,
        guild_id: &str,
        role_id: &str,
        user_id: &str,
        token: &str,
    ) -> Result<bool, AppError> {
        let resp = self
            .http
            .post(self.user_url(guild_id, role_id, user_id))
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RoleLogic(format!(
                "Add user failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;
        Ok(body["data"]["added"].as_bool().unwrap_or(false))
    }

    pub async fn remove_user(
        &self,
        guild_id: &str,
        role_id: &str,
        user_id: &str,
        token: &str,
    ) -> Result<bool, AppError> {
        let resp = self
            .http
            .delete(self.user_url(guild_id, role_id, user_id))
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RoleLogic(format!(
                "Remove user failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;
        Ok(body["data"]["removed"].as_bool().unwrap_or(false))
    }
}
