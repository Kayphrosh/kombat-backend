// app/src/main.rs
use axum::{
    routing::{get, post},
    Router,
};
use dotenvy::dotenv;
use std::{net::SocketAddr, sync::Arc};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod handlers;
mod models;
mod services;

use handlers::wager::{
    AppState, accept_wager, cancel_wager, create_wager,
    dispute_wager, get_wager, list_wagers, resolve_wager,
};
use handlers::notifications::{list_notifications, mark_read as mark_notification_read, stream_notifications, ws_notifications};
use handlers::auth::mint_token;
use handlers::user::{get_user_profile, update_user_profile};
use services::{DbService, SolanaService, IndexerService};
use prometheus::{Encoder, TextEncoder, IntCounter};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    // ── Logging ───────────────────────────────────────────────────────────────
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wager_api=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // ── Config from env ───────────────────────────────────────────────────────
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    let solana_rpc_url = std::env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .expect("PORT must be a valid number");

    // ── Services ──────────────────────────────────────────────────────────────
    tracing::info!("Connecting to database...");
    let db = Arc::new(DbService::new(&database_url).await?);

    tracing::info!("Connecting to Solana RPC: {}", solana_rpc_url);
    let solana = Arc::new(SolanaService::new(&solana_rpc_url));

    // ── Indexer Start ─────────────────────────────────────────────────────────
    let indexer_db = db.clone();
    let indexer_rpc = solana_rpc_url.clone();
    let program_id = std::env::var("WAGER_PROGRAM_ID")
       .unwrap_or_else(|_| "Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK".to_string());

    tokio::spawn(async move {
        let indexer = IndexerService::new(indexer_db, indexer_rpc, program_id);
        indexer.run().await;
    });

    // Realtime notifications broadcast channel
    let (notif_tx, _notif_rx) = tokio::sync::broadcast::channel::<(String, serde_json::Value)>(100);
    let nonce_rate = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    // Optionally initialize Redis if REDIS_URL is set
    let redis_client = match std::env::var("REDIS_URL") {
        Ok(url) => {
            tracing::info!("Connecting to Redis: {}", url);
            let client = redis::Client::open(url.as_str())?;
            Some(Arc::new(client))
        }
        Err(_) => None,
    };

    // Prometheus counters
    let rl_exceeded = IntCounter::new("nonce_rate_limit_exceeded_total", "Number of nonce rate limit exceedances").unwrap();
    let rl_requests = IntCounter::new("nonce_rate_limit_requests_total", "Number of nonce requests").unwrap();
    // register to default registry
    let _ = prometheus::default_registry().register(Box::new(rl_exceeded.clone()));
    let _ = prometheus::default_registry().register(Box::new(rl_requests.clone()));

    let state = Arc::new(AppState { db, solana, notif_tx: Arc::new(notif_tx), nonce_rate, redis_client, rate_limit_exceeded: Some(std::sync::Arc::new(rl_exceeded)), rate_limit_requests: Some(std::sync::Arc::new(rl_requests)) });

    // ── CORS ──────────────────────────────────────────────────────────────────
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // ── Router ────────────────────────────────────────────────────────────────
    let app = Router::new()
        // Health check
        .route("/health", get(health_handler))
        // Wager routes
        .route("/wagers",             get(list_wagers).post(create_wager))
        .route("/wagers/:address",    get(get_wager))
        .route("/wagers/:address/accept",  post(accept_wager))
        .route("/wagers/:address/cancel",  post(cancel_wager))
        .route("/wagers/:address/resolve", post(resolve_wager))
        .route("/wagers/:address/dispute", post(dispute_wager))
        // User profile routes
        .route("/users/:wallet", get(get_user_profile).post(update_user_profile))
        // Notifications
        .route("/notifications/:wallet", get(list_notifications))
        .route("/notifications/:id/read", post(mark_notification_read))
        .route("/notifications/stream/:wallet", get(stream_notifications))
        // Auth (admin mint token) + Solana nonce flow
        .route("/auth/token", post(mint_token))
        .route("/auth/nonce/:wallet", get(handlers::auth::get_nonce))
        .route("/auth/verify", post(handlers::auth::verify_signature))
        .route("/ws/notifications/:wallet", get(ws_notifications))
        // Prometheus metrics
        .route("/metrics", get(get_metrics))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // ── Start server ──────────────────────────────────────────────────────────
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Wager API listening on {}", addr);

    // Note: Redis subscriber task for cross-instance notification delivery would go here.
    // The current implementation publishes to Redis when REDIS_URL is configured, but
    // subscribing requires advanced async pubsub handling. For production, consider:
    // - Using Redis Streams instead of Pub/Sub for durability
    // - Using a dedicated listener service
    // - Using Cloud Pub/Sub (GCP) for managed pub/sub

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn get_metrics() -> impl axum::response::IntoResponse {
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    let mf = prometheus::default_registry().gather();
    let _ = encoder.encode(&mf, &mut buffer);
    (
        axum::http::StatusCode::OK,
        [("Content-Type", "text/plain; version=0.0.4")],
        String::from_utf8(buffer).unwrap_or_default(),
    )
}

async fn health_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": "wager-api",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}