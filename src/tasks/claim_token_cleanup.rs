use std::sync::Arc;
use std::time::Duration;

use crate::AppState;

const POLL_INTERVAL_SECS: u64 = 300;

/// Drop tokens that are past their TTL or were consumed > 24h ago.
/// The retention window keeps recently-consumed rows for diagnostic
/// log lookups.
pub async fn run(state: Arc<AppState>) {
    tracing::info!(interval_secs = POLL_INTERVAL_SECS, "Claim-token cleanup worker started");
    let mut interval = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match sqlx::query(
            "DELETE FROM claim_tokens \
             WHERE (consumed_at IS NULL AND expires_at < now() - interval '1 hour') \
                OR (consumed_at IS NOT NULL AND consumed_at < now() - interval '24 hours')",
        )
        .execute(&state.pool)
        .await
        {
            Ok(r) if r.rows_affected() > 0 => {
                tracing::info!(rows = r.rows_affected(), "Claim tokens pruned");
            }
            Ok(_) => {}
            Err(e) => tracing::error!("Cleanup error: {e}"),
        }
    }
}
