use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct Registration {
    pub id: Uuid,
    pub guild_id: String,
    pub role_id: String,
    pub rolelogic_token: String,
    pub lootlabs_short_link: Option<String>,
    pub role_duration_hours: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── RoleLogic plugin contract ────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegisterBody {
    pub guild_id: String,
    pub role_id: String,
}

#[derive(Deserialize)]
pub struct ConfigBody {
    pub guild_id: String,
    pub role_id: String,
    pub config: ConfigValues,
}

#[derive(Deserialize, Default)]
pub struct ConfigValues {
    pub lootlabs_short_link: Option<String>,
    pub role_duration_hours: Option<f64>,
}

#[derive(Deserialize)]
pub struct DeleteConfigBody {
    pub guild_id: String,
    pub role_id: String,
}

#[derive(Serialize)]
pub struct ConfigResponse {
    pub version: u32,
    pub name: String,
    pub description: String,
    pub sections: Vec<ConfigSection>,
    pub values: serde_json::Value,
}

#[derive(Serialize)]
pub struct ConfigSection {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub fields: Vec<ConfigField>,
}

#[derive(Serialize)]
pub struct ConfigField {
    #[serde(rename = "type")]
    pub field_type: String,
    pub key: String,
    pub label: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}
