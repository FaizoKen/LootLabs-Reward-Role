use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::AppState;

const POLL_INTERVAL_SECS: u64 = 60;

/// Periodically scan for claims whose role_duration has elapsed, revoke
/// them via the RoleLogic API, and mark `revoked_at`. Permanent claims
/// (NULL expires_at) are skipped.
pub async fn run(state: Arc<AppState>) {
    tracing::info!(interval_secs = POLL_INTERVAL_SECS, "Role-expiry worker started");
    let mut interval = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Err(e) = sweep_once(&state).await {
            tracing::error!("Role-expiry sweep error: {e}");
        }
    }
}

async fn sweep_once(state: &AppState) -> Result<(), sqlx::Error> {
    // Pull a bounded batch so a backlog can't starve the rest of the
    // worker pool. The remaining rows are picked up next tick.
    let rows: Vec<(Uuid, Uuid, String, String, String, String)> = sqlx::query_as(
        "SELECT c.id, c.registration_id, c.discord_id, \
                r.guild_id, r.role_id, r.rolelogic_token \
         FROM claims c \
         JOIN registrations r ON r.id = c.registration_id \
         WHERE c.expires_at IS NOT NULL \
           AND c.expires_at < now() \
           AND c.revoked_at IS NULL \
         LIMIT 500",
    )
    .fetch_all(&state.pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }
    tracing::info!(count = rows.len(), "Expiring claims");

    for (claim_id, _reg_id, discord_id, guild_id, role_id, rl_token) in rows {
        match state
            .rl_client
            .remove_user(&guild_id, &role_id, &discord_id, &rl_token)
            .await
        {
            Ok(_) => {
                let _ = sqlx::query("UPDATE claims SET revoked_at = now() WHERE id = $1")
                    .bind(claim_id)
                    .execute(&state.pool)
                    .await;
                tracing::info!(claim_id = ?claim_id, discord_id, "Role expired & revoked");
            }
            Err(e) => {
                // Leave revoked_at NULL so we retry next tick.
                tracing::warn!(claim_id = ?claim_id, discord_id, "Revoke failed (will retry): {e}");
            }
        }
    }
    Ok(())
}
