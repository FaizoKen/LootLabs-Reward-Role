use std::env;

#[derive(Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub session_secret: String,
    pub base_url: String,
    pub auth_gateway_url: String,
    pub listen_addr: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: required("DATABASE_URL"),
            session_secret: required("SESSION_SECRET"),
            base_url: required("BASE_URL").trim_end_matches('/').to_string(),
            auth_gateway_url: required("AUTH_GATEWAY_URL").trim_end_matches('/').to_string(),
            listen_addr: env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8088".into()),
        }
    }
}

fn required(key: &str) -> String {
    env::var(key).unwrap_or_else(|_| panic!("Required env var not set: {key}"))
}
