use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use chrono::Utc;
use uuid::Uuid;

use crate::error::AppError;
use crate::services::{session, token_gen};
use crate::AppState;

const SESSION_COOKIE: &str = "rl_session";
const CLAIM_TOKEN_TTL_HOURS: i64 = 24;

/// `GET /claim/<registration_id>` — landing page for members.
///
/// 1. If not logged in → bounce through `/auth/login` with this URL as `return_to`.
/// 2. If logged in → issue a `puid` claim token, then redirect the user to the
///    admin-configured Loot Labs short link with `&puid={token}` appended.
///
/// We don't render any HTML on the happy path; this is a thin redirector.
/// The only HTML rendered is the "not configured yet" landing page if the
/// admin hasn't pasted their Loot Labs short link.
pub async fn start(
    State(state): State<Arc<AppState>>,
    Path(registration_id): Path<Uuid>,
    jar: CookieJar,
) -> Result<Response, AppError> {
    // Auth gate: redirect to the centralized Auth Gateway if not signed in.
    let discord_id = match jar
        .get(SESSION_COOKIE)
        .and_then(|c| session::verify_session(c.value(), &state.config.session_secret))
        .map(|(id, _name)| id)
    {
        Some(id) => id,
        None => {
            let return_to = format!(
                "/lootlabs-reward-role/claim/{}",
                registration_id
            );
            let url = format!("/auth/login?return_to={}", urlencoding::encode(&return_to));
            return Ok(Redirect::to(&url).into_response());
        }
    };

    // Load registration. If missing or unconfigured, show a friendly page
    // rather than redirecting them into a broken Loot Labs URL.
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT lootlabs_short_link FROM registrations WHERE id = $1",
    )
    .bind(registration_id)
    .fetch_optional(&state.pool)
    .await?;

    let short_link = match row {
        None => {
            return Ok(landing_page(
                "Claim link not found",
                "This claim link doesn't exist anymore — the role link may have been deleted.",
            ));
        }
        Some((None,)) => {
            return Ok(landing_page(
                "Not set up yet",
                "The admin hasn't finished configuring this reward yet. Try again later.",
            ));
        }
        Some((Some(s),)) if s.trim().is_empty() => {
            return Ok(landing_page(
                "Not set up yet",
                "The admin hasn't finished configuring this reward yet. Try again later.",
            ));
        }
        Some((Some(s),)) => s,
    };

    // Issue a single-use puid token for this user + this registration.
    let token = token_gen::random_token();
    let expires_at = Utc::now() + chrono::Duration::hours(CLAIM_TOKEN_TTL_HOURS);
    sqlx::query(
        "INSERT INTO claim_tokens (token, registration_id, discord_id, expires_at) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(&token)
    .bind(registration_id)
    .bind(&discord_id)
    .bind(expires_at)
    .execute(&state.pool)
    .await?;

    let target = token_gen::append_puid(&short_link, &token);
    tracing::info!(
        ?registration_id,
        discord_id,
        "Claim started — redirecting to Loot Labs"
    );
    Ok(Redirect::to(&target).into_response())
}

fn landing_page(title: &str, body: &str) -> Response {
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8">
<title>{title}</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" type="image/x-icon" href="/lootlabs-reward-role/favicon.ico">
<style>
body{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;background:#0e1525;color:#c9d1d9;
display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0;padding:1rem}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:12px;padding:2rem;max-width:420px;text-align:center}}
h1{{color:#e6edf3;margin:0 0 .75rem;font-size:1.4rem}}p{{color:#8b949e;margin:0;line-height:1.5}}
</style></head><body><div class="card"><h1>{title}</h1><p>{body}</p></div></body></html>"#,
        title = html_escape(title),
        body = html_escape(body),
    );
    Html(html).into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
