use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::AppState;

/// Loot Labs callback. They send a GET with `?k=<guild_secret>` (we baked
/// in) plus auto-appended macros (CLICK_ID = our puid, IP, UNIQUE_ID).
/// Only `k` and `click_id` matter to us — the puid uniquely identifies
/// both the user and the role link, so the postback URL doesn't need
/// registration_id in the path. One URL per Discord server.
///
/// We always return 200 — Loot Labs retries non-200s and we don't want to
/// leak signal to attackers about which secret/token guesses are getting close.
pub async fn callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<Vec<(String, String)>>,
    headers: HeaderMap,
) -> Json<Value> {
    let cf_ip = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("?");
    tracing::info!(
        cf_ip,
        query_keys = ?query.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        "incoming postback"
    );

    let provided_k = match query
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("k"))
        .map(|(_, v)| v.as_str())
    {
        Some(v) => v,
        None => {
            tracing::warn!("postback rejected: missing ?k= URL secret");
            return Json(json!({"ok": true}));
        }
    };

    let puid = match query
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("click_id"))
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
    {
        Some(v) => v.to_string(),
        None => {
            tracing::warn!(
                "postback rejected: empty click_id (Loot Labs didn't echo our puid — \
                 user clicked the short link without going through the claim flow?)"
            );
            return Json(json!({"ok": true}));
        }
    };

    // Single shot: resolve puid → registration row + the guild's secret.
    // claim_tokens.token is the primary key; the join keeps everything in
    // one round trip and naturally rejects mismatched guilds.
    let row: Option<(
        Uuid,
        String,
        String,
        String,
        String,
        Option<i32>,
        String,
        Option<DateTime<Utc>>,
        DateTime<Utc>,
    )> = sqlx::query_as(
        "SELECT r.id, r.guild_id, r.role_id, r.rolelogic_token, ct.discord_id, \
                r.role_duration_hours, gs.secret, ct.consumed_at, ct.expires_at \
         FROM claim_tokens ct \
         JOIN registrations r ON r.id = ct.registration_id \
         JOIN guild_secrets gs ON gs.guild_id = r.guild_id \
         WHERE ct.token = $1",
    )
    .bind(&puid)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);

    let (
        registration_id,
        guild_id,
        role_id,
        rl_token,
        discord_id,
        duration_hours,
        secret,
        consumed_at,
        expires_at,
    ) = match row {
        Some(t) => t,
        None => {
            tracing::warn!(
                puid_prefix = puid.chars().take(8).collect::<String>(),
                "postback rejected: no matching claim_token row"
            );
            return Json(json!({"ok": true}));
        }
    };

    // URL secret check (constant time). Loot Labs has no HMAC — the only
    // auth is the unguessable `?k=` baked into the URL we gave the admin.
    if !bool::from(provided_k.as_bytes().ct_eq(secret.as_bytes())) {
        tracing::warn!(
            ?registration_id,
            received_k_prefix = &provided_k.chars().take(6).collect::<String>(),
            "postback rejected: URL secret mismatch (wrong guild secret)"
        );
        return Json(json!({"ok": true}));
    }

    if consumed_at.is_some() {
        tracing::info!(?registration_id, "postback ignored: token already consumed");
        return Json(json!({"ok": true}));
    }
    if expires_at < Utc::now() {
        tracing::warn!(?registration_id, "postback rejected: claim token expired");
        return Json(json!({"ok": true}));
    }

    // Persist the claim. role_duration_hours NULL = permanent.
    let claim_expires_at: Option<DateTime<Utc>> = duration_hours
        .filter(|h| *h > 0)
        .map(|h| Utc::now() + chrono::Duration::hours(h as i64));

    // Upsert: if a member completes again before old role expires, refresh
    // the expiry rather than inserting a duplicate active row. revoked_at
    // is preserved as NULL so the unique index on active rows still holds.
    let insert_res = sqlx::query(
        "INSERT INTO claims (registration_id, discord_id, expires_at) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (registration_id, discord_id) WHERE revoked_at IS NULL \
         DO UPDATE SET expires_at = EXCLUDED.expires_at, granted_at = now()",
    )
    .bind(registration_id)
    .bind(&discord_id)
    .bind(claim_expires_at)
    .execute(&state.pool)
    .await;
    if let Err(e) = insert_res {
        tracing::error!(?registration_id, "claim insert failed: {e}");
        return Json(json!({"ok": true}));
    }

    let _ = sqlx::query("UPDATE claim_tokens SET consumed_at = now() WHERE token = $1")
        .bind(&puid)
        .execute(&state.pool)
        .await;

    // Push the role to RoleLogic out-of-band so we can return to Loot Labs fast.
    let rl = state.rl_client.clone();
    let user_id = discord_id.clone();
    tokio::spawn(async move {
        match rl.add_user(&guild_id, &role_id, &user_id, &rl_token).await {
            Ok(added) => tracing::info!(
                ?registration_id,
                discord_id = user_id,
                added,
                ?claim_expires_at,
                "✓ claim granted via RoleLogic"
            ),
            Err(e) => tracing::error!(
                ?registration_id,
                discord_id = user_id,
                "RoleLogic add_user failed: {e}"
            ),
        }
    });

    Json(json!({"ok": true}))
}
