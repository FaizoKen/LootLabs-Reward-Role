use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::json;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::{
    ConfigBody, ConfigField, ConfigResponse, ConfigSection, DeleteConfigBody, RegisterBody,
    Registration,
};
use crate::services::token_gen;
use crate::AppState;

fn extract_auth_token(headers: &HeaderMap) -> Result<String, AppError> {
    let raw = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            tracing::warn!("Authorization header missing");
            AppError::Unauthorized
        })?;
    // Tolerate either "Token rl_..." (RoleLogic spec) or accidental "Bearer rl_..."
    raw.strip_prefix("Token ")
        .or_else(|| raw.strip_prefix("token "))
        .or_else(|| raw.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            tracing::warn!(
                authorization_prefix = raw.split_whitespace().next().unwrap_or(""),
                "Authorization header has unexpected scheme"
            );
            AppError::Unauthorized
        })
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    // Read body as Bytes first so we can log the raw payload on parse failure.
    // This is the entry point RoleLogic calls during role-link creation; if it
    // fails, the entire link fails to create, so a precise diagnosis matters.
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("(missing)");
    let body_preview = String::from_utf8_lossy(&body[..body.len().min(500)]).to_string();
    tracing::info!(
        content_type,
        body_len = body.len(),
        body_preview = body_preview.as_str(),
        "/register received"
    );

    let token = extract_auth_token(&headers)?;

    let parsed: RegisterBody = serde_json::from_slice(&body).map_err(|e| {
        tracing::warn!(
            "/register body did not match expected shape ({{guild_id, role_id}}): {e} \
             — got: {body_preview}"
        );
        AppError::BadRequest(format!("Expected JSON {{\"guild_id\": \"...\", \"role_id\": \"...\"}}; parse error: {e}"))
    })?;

    if parsed.guild_id.trim().is_empty() || parsed.role_id.trim().is_empty() {
        return Err(AppError::BadRequest(
            "guild_id and role_id are required and must be non-empty strings".into(),
        ));
    }

    // Ensure a secret exists for this guild. The first role link in a guild
    // generates it; later role links reuse the same one so the admin only
    // pastes the postback URL into Loot Labs once per server.
    let new_secret = token_gen::random_token();
    sqlx::query(
        "INSERT INTO guild_secrets (guild_id, secret) VALUES ($1, $2) \
         ON CONFLICT (guild_id) DO NOTHING",
    )
    .bind(&parsed.guild_id)
    .bind(&new_secret)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(
            guild_id = parsed.guild_id,
            "DB upsert failed in /register guild_secrets: {e}"
        );
        AppError::Database(e)
    })?;

    let row: (Uuid,) = sqlx::query_as(
        r#"INSERT INTO registrations (guild_id, role_id, rolelogic_token)
           VALUES ($1, $2, $3)
           ON CONFLICT (guild_id, role_id) DO UPDATE SET
             rolelogic_token = EXCLUDED.rolelogic_token,
             updated_at = now()
           RETURNING id"#,
    )
    .bind(&parsed.guild_id)
    .bind(&parsed.role_id)
    .bind(&token)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(
            guild_id = parsed.guild_id,
            role_id = parsed.role_id,
            "DB upsert failed in /register: {e}"
        );
        AppError::Database(e)
    })?;

    tracing::info!(
        guild_id = parsed.guild_id,
        role_id = parsed.role_id,
        registration_id = ?row.0,
        "✓ Registered role link"
    );

    Ok(Json(json!({
        "success": true,
        "registration_id": row.0,
    })))
}

pub async fn get_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ConfigResponse>, AppError> {
    let token = extract_auth_token(&headers)?;

    let reg = sqlx::query_as::<_, Registration>(
        "SELECT id, guild_id, role_id, rolelogic_token, lootlabs_short_link, \
                role_duration_hours, created_at, updated_at \
         FROM registrations WHERE rolelogic_token = $1",
    )
    .bind(&token)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::Unauthorized)?;

    let guild_secret: String =
        sqlx::query_scalar("SELECT secret FROM guild_secrets WHERE guild_id = $1")
            .bind(&reg.guild_id)
            .fetch_one(&state.pool)
            .await?;

    // Has the admin already configured a sibling role link in this guild?
    // If so, they've already pasted the postback URL into Loot Labs and we
    // can soften Step 1 to a "you're done with this part" reminder.
    let other_links: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM registrations WHERE guild_id = $1 AND id <> $2",
    )
    .bind(&reg.guild_id)
    .bind(reg.id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);
    let postback_already_set_up = other_links > 0;

    let postback_url = format!("{}/postback?k={}", state.config.base_url, guild_secret);
    let claim_link = format!("{}/claim/{}", state.config.base_url, reg.id);

    let postback_section = if postback_already_set_up {
        ConfigSection {
            title: "Step 1 — Postback URL (already configured ✓)".to_string(),
            description: Some(
                "This Discord server already has another role link wired to Loot Labs, \
                 so the postback URL is shared and doesn't need to be pasted again. \
                 If you ever need it (e.g. switching Loot Labs accounts), it's shown below."
                    .to_string(),
            ),
            fields: vec![ConfigField {
                field_type: "display".to_string(),
                key: "postback_url".to_string(),
                label: "Loot Labs Postback URL (per-guild, already set)".to_string(),
                description: String::new(),
                placeholder: None,
                validation: None,
                value: Some(serde_json::Value::String(postback_url)),
            }],
        }
    } else {
        ConfigSection {
            title: "Step 1 — Postback URL (one-time per server)".to_string(),
            description: Some(
                "Paste this URL into Loot Labs → https://creators.lootlabs.gg/advanced → \
                 Postback URL. Tick the {CLICK_ID} parameter (Loot Labs requires it). \
                 You only need to do this once for the whole Discord server — future role \
                 links in this server will reuse the same postback URL automatically."
                    .to_string(),
            ),
            fields: vec![ConfigField {
                field_type: "display".to_string(),
                key: "postback_url".to_string(),
                label: "Loot Labs Postback URL".to_string(),
                description: String::new(),
                placeholder: None,
                validation: None,
                value: Some(serde_json::Value::String(postback_url)),
            }],
        }
    };

    let sections = vec![
        postback_section,
        ConfigSection {
            title: "Step 2 — Loot Labs short link".to_string(),
            description: Some(
                "Create a link in your Loot Labs dashboard → https://creators.lootlabs.gg/dashboard \
                 Set the destination to a Discord message link (right-click → Copy Message Link) so users return after completing the task. \
                 Paste the generated short link here as-is."
                    .to_string(),
            ),
            fields: vec![ConfigField {
                field_type: "url".to_string(),
                key: "lootlabs_short_link".to_string(),
                label: "Loot Labs short link".to_string(),
                description:
                    "Bare short link from your Loot Labs dashboard."
                        .to_string(),
                placeholder: Some("https://links.lootlabs.gg/s?ABCDEF".to_string()),
                validation: Some(json!({"required": true})),
                value: None,
            }],
        },
        ConfigSection {
            title: "Step 3 — Role duration".to_string(),
            description: Some(
                "How long the role stays granted after a successful completion. \
                 Set to 0 (or leave blank) for a permanent role that never expires."
                    .to_string(),
            ),
            fields: vec![ConfigField {
                field_type: "number".to_string(),
                key: "role_duration_hours".to_string(),
                label: "Role duration (hours)".to_string(),
                description:
                    "0 = permanent. 24 = role lasts a day. Max 87600 (10 years)."
                        .to_string(),
                placeholder: Some("24".to_string()),
                validation: Some(json!({"min": 0, "max": 87600})),
                value: None,
            }],
        },
        ConfigSection {
            title: "Step 4 — Share the claim link".to_string(),
            description: Some(
                "Send this link to your Discord members (e.g. in a sticky message or button). \
                 Members log in with Discord, complete one Loot Labs task, and get the role."
                    .to_string(),
            ),
            fields: vec![ConfigField {
                field_type: "display".to_string(),
                key: "claim_link".to_string(),
                label: "Member claim link".to_string(),
                description: String::new(),
                placeholder: None,
                validation: None,
                value: Some(serde_json::Value::String(claim_link)),
            }],
        },
    ];

    Ok(Json(ConfigResponse {
        version: 1,
        name: "Loot Labs Reward Role".to_string(),
        description:
            "Members complete one Loot Labs task and receive a Discord role. Optional duration."
                .to_string(),
        sections,
        values: json!({
            "lootlabs_short_link": reg.lootlabs_short_link.unwrap_or_default(),
            "role_duration_hours": reg.role_duration_hours.unwrap_or(0),
        }),
    }))
}

pub async fn post_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ConfigBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let token = extract_auth_token(&headers)?;

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM registrations \
         WHERE guild_id = $1 AND role_id = $2 AND rolelogic_token = $3)",
    )
    .bind(&body.guild_id)
    .bind(&body.role_id)
    .bind(&token)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(false);
    if !exists {
        return Err(AppError::Unauthorized);
    }

    // Normalize the short link: trim, basic shape check.
    let short_link = body
        .config
        .lootlabs_short_link
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    if let Some(ref link) = short_link {
        if !(link.starts_with("https://") || link.starts_with("http://")) {
            return Err(AppError::BadRequest(
                "Loot Labs short link must start with https://".into(),
            ));
        }
        if link.len() > 1000 {
            return Err(AppError::BadRequest("Short link too long".into()));
        }
    }

    // 0 / blank / negative → permanent (NULL)
    let duration: Option<i32> = body
        .config
        .role_duration_hours
        .map(|h| h.round() as i32)
        .filter(|h| *h > 0);
    if let Some(h) = duration {
        if h > 87600 {
            return Err(AppError::BadRequest(
                "Role duration too long (max 10 years)".into(),
            ));
        }
    }

    sqlx::query(
        "UPDATE registrations SET \
            lootlabs_short_link = $3, \
            role_duration_hours = $4, \
            updated_at = now() \
         WHERE guild_id = $1 AND role_id = $2",
    )
    .bind(&body.guild_id)
    .bind(&body.role_id)
    .bind(&short_link)
    .bind(duration)
    .execute(&state.pool)
    .await?;

    tracing::info!(
        guild_id = body.guild_id,
        role_id = body.role_id,
        has_link = short_link.is_some(),
        duration_hours = ?duration,
        "Config updated"
    );

    Ok(Json(json!({"success": true})))
}

pub async fn delete_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<DeleteConfigBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let token = extract_auth_token(&headers)?;
    let res = sqlx::query(
        "DELETE FROM registrations \
         WHERE guild_id = $1 AND role_id = $2 AND rolelogic_token = $3",
    )
    .bind(&body.guild_id)
    .bind(&body.role_id)
    .bind(&token)
    .execute(&state.pool)
    .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::Unauthorized);
    }
    tracing::info!(
        guild_id = body.guild_id,
        role_id = body.role_id,
        "Registration deleted"
    );
    Ok(Json(json!({"success": true})))
}
