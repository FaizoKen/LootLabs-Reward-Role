mod config;
mod db;
mod error;
mod models;
mod routes;
mod services;
mod tasks;

use std::sync::Arc;

use axum::routing::{delete, get, post};
use axum::Router;
use sqlx::PgPool;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::services::rolelogic::RoleLogicClient;

pub struct AppState {
    pub pool: PgPool,
    pub config: config::AppConfig,
    pub rl_client: RoleLogicClient,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lootlabs_reward_role=info,tower_http=info".into()),
        )
        .init();

    let app_config = config::AppConfig::from_env();
    let listen_addr = app_config.listen_addr.clone();

    let pool = db::create_pool(&app_config.database_url).await;
    db::run_migrations(&pool).await;
    tracing::info!("Database connected and migrations applied");

    let rl_client = RoleLogicClient::new();

    let state = Arc::new(AppState {
        pool,
        config: app_config,
        rl_client,
    });

    tokio::spawn(tasks::role_expiry_worker::run(Arc::clone(&state)));
    tokio::spawn(tasks::claim_token_cleanup::run(Arc::clone(&state)));

    let app = Router::new()
        .nest(
            "/lootlabs-reward-role",
            Router::new()
                // RoleLogic plugin contract
                .route("/register", post(routes::plugin::register))
                .route("/config", get(routes::plugin::get_config))
                .route("/config", post(routes::plugin::post_config))
                .route("/config", delete(routes::plugin::delete_config))
                // Loot Labs callback (URL-secret-authed). One URL per guild;
                // dispatch is by the puid in CLICK_ID, not by path.
                .route("/postback", get(routes::postback::callback))
                // Member-facing claim flow
                .route("/claim/{registration_id}", get(routes::claim::start))
                // Static
                .route("/health", get(routes::health::health))
                .route("/favicon.ico", get(routes::health::favicon)),
        )
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .expect("Failed to bind listener");
    tracing::info!("Server starting on {listen_addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Shutdown signal received, draining...");
        })
        .await
        .expect("Server error");
}
