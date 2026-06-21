// app/src/main.rs
use axum::{
    routing::{get, post},
    Router,
};
use dotenvy::dotenv;
use std::{net::SocketAddr, sync::Arc};
use tower_http::{
    cors::{Any, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod handlers;
mod models;
mod services;
mod state;

use handlers::admin::{
    get_admin_organizer, get_admin_outcome_proposal, list_admin_organizers,
    list_admin_outcome_proposals,
};
use handlers::agent::{get_admin_agent_run, list_admin_agent_runs, submit_agent_outcome_proposal};
use handlers::market::{
    activate_receipt_listing, create_receipt_listing, get_receipt_listing,
    get_receipt_listing_buy_ptb, get_receipt_listing_list_ptb, list_receipt_listings,
    mark_receipt_listing_sold,
};
use handlers::notifications::{
    list_notifications, mark_read as mark_notification_read, stream_notifications, ws_notifications,
};
use handlers::organizer::{
    apply_organizer, create_organizer_kyc_session, get_organizer, review_organizer,
};
use handlers::payment::{
    create_payment_intent, create_payment_intent_onramp_session, get_payment_intent,
    get_payment_intent_ptb,
};
use handlers::ramp::{create_dynamic_ramp_session, list_ramp_providers};
use handlers::sui::{
    get_network_wallet_balances_handler, get_network_wallet_dashboard_handler,
    get_network_wallet_usdc_balance_handler, get_sui_config, get_sui_health,
    get_sui_network_config, get_sui_network_health, get_wallet_balances, get_wallet_dashboard,
    get_wallet_usdc_balance,
};
use handlers::tournament::{
    backfill_tournament_pools, calculate_payout, cancel_tournament, configure_tournament_pool,
    create_organizer_match, create_organizer_tournament, create_outcome_proposal,
    create_tournament, get_tournament, get_tournament_source_grid,
    get_tournament_source_pandascore, get_user_stake_stats, get_user_stakes,
    list_organizer_tournaments, list_outcome_proposals, list_tournament_stakes, list_tournaments,
    place_stake, probe_grid_tournaments, probe_pandascore_tournaments, resolve_tournament,
    review_outcome_proposal, sync_grid_tournaments, sync_pandascore_tournaments, sync_tournament,
    sync_tournament_stakes,
};
use handlers::transak::{create_transak_widget_url, get_transak_config, get_transak_quote};
use handlers::upload::upload_file;
use handlers::user::{
    check_email, delete_user, get_home_summary, get_notification_settings, get_user_profile,
    get_user_stats, register_push_token, search_users, update_notification_settings,
    update_user_profile,
};
use handlers::wager::{
    accept_wager, accept_wager_ptb, cancel_wager, cancel_wager_ptb, create_wager, create_wager_ptb,
    declare_winner, decline_wager, get_wager, list_disputes, list_my_wagers, list_wager_artifacts,
    list_wagers, resolve_wager_ptb, submit_dispute, update_wager_status,
};
use handlers::walrus::{
    create_walrus_artifact, get_walrus_artifact, get_walrus_blob_url, get_walrus_config,
};
use handlers::webhook::{handle_match_result_webhook, handle_pandascore_webhook};
use prometheus::{Encoder, IntCounter, TextEncoder};
use services::{
    DbService, GridConfig, GridService, PandascoreConfig, PandascoreService, RampConfig,
    RampService, SuiConfig, SuiService, TransakConfig, TransakService, WalrusConfig, WalrusService,
};
use state::AppState;

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
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .expect("PORT must be a valid number");

    // ── Services ──────────────────────────────────────────────────────────────
    tracing::info!("Connecting to database...");
    let db = Arc::new(DbService::new(&database_url).await?);

    let sui_config = SuiConfig::from_env();
    tracing::info!(
        "Configuring Sui RPC: active_network={}, networks={}",
        sui_config.active_network,
        sui_config
            .networks
            .iter()
            .map(|config| format!("{}:{}", config.network, config.rpc_url))
            .collect::<Vec<_>>()
            .join(",")
    );
    let sui = Arc::new(SuiService::new(sui_config));

    let ramp_config = RampConfig::from_env();
    tracing::info!(
        "Configuring ramps: primary_provider={}, dynamic_onramp_enabled={}",
        ramp_config.primary_provider,
        ramp_config.dynamic_onramp_enabled
    );
    let ramp = Arc::new(RampService::new(ramp_config));

    let transak_config = TransakConfig::from_env();
    tracing::info!(
        "Configuring Transak: enabled={}, environment={}, referrer_domain={}",
        transak_config.enabled,
        transak_config.environment,
        transak_config.referrer_domain
    );
    let transak = Arc::new(TransakService::new(transak_config));

    let grid_config = GridConfig::from_env();
    tracing::info!(
        "Configuring GRID: enabled={}, configured={}, base_url={}, matches_path={}",
        grid_config.enabled,
        grid_config.configured(),
        grid_config.base_url,
        grid_config.matches_path
    );
    let grid = Arc::new(GridService::new(grid_config));

    let pandascore_config = PandascoreConfig::from_env();
    tracing::info!(
        "Configuring PandaScore: enabled={}, configured={}, base_url={}",
        pandascore_config.enabled,
        pandascore_config.configured(),
        pandascore_config.base_url,
    );
    let pandascore = Arc::new(PandascoreService::new(pandascore_config));

    let walrus_config = WalrusConfig::from_env();
    tracing::info!(
        "Configuring Walrus: enabled={}, configured={}, network={}",
        walrus_config.enabled,
        walrus_config.configured(),
        walrus_config.network
    );
    let walrus = Arc::new(WalrusService::new(walrus_config));

    // Realtime notifications broadcast channel
    let (notif_tx, _notif_rx) = tokio::sync::broadcast::channel::<(String, serde_json::Value)>(100);

    // ── Dynamic SDK service (optional) ────────────────────────────────────────
    let dynamic_service = match std::env::var("DYNAMIC_ENVIRONMENT_ID") {
        Ok(env_id) => {
            tracing::info!("Dynamic SDK verification enabled (env: {})", env_id);
            Some(Arc::new(services::DynamicService::new(env_id)))
        }
        Err(_) => {
            tracing::info!("DYNAMIC_ENVIRONMENT_ID not set — Dynamic auth disabled");
            None
        }
    };

    // ── Upload service (optional) ─────────────────────────────────────────────
    let upload_base_url =
        std::env::var("UPLOAD_BASE_URL").unwrap_or_else(|_| format!("http://localhost:{}", port));
    let upload_service = match std::env::var("UPLOAD_DIR") {
        Ok(dir) => {
            tracing::info!("File uploads enabled (dir: {})", dir);
            Some(Arc::new(
                services::UploadService::new(&dir, &upload_base_url)
                    .expect("Failed to initialize upload service"),
            ))
        }
        Err(_) => {
            tracing::info!("UPLOAD_DIR not set — file uploads disabled");
            None
        }
    };

    // Prometheus counters
    let dynamic_auth_requests = IntCounter::new(
        "dynamic_auth_requests_total",
        "Number of Dynamic auth verification requests",
    )
    .unwrap();
    let _ = prometheus::default_registry().register(Box::new(dynamic_auth_requests));

    let state = Arc::new(AppState {
        db,
        sui,
        ramp,
        transak,
        grid,
        pandascore,
        walrus,
        notif_tx: Arc::new(notif_tx),
        dynamic_service,
        upload_service,
    });

    // ── Background PandaScore sync scheduler ──────────────────────────────────
    spawn_pandascore_scheduler(state.clone());
    // Fast loop for near-real-time live/just-finished match updates.
    spawn_pandascore_live_poller(state.clone());
    // Index on-chain stakes into the DB so pool/staker aggregates stay correct.
    spawn_stake_reconciler(state.clone());

    // ── CORS ──────────────────────────────────────────────────────────────────
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // ── Router ────────────────────────────────────────────────────────────────
    let mut app = Router::new()
        // Health check
        .route("/health", get(health_handler))
        // ── User profile routes ──────────────────────────────────────────────
        .route("/check-email", get(check_email))
        .route("/users/search", get(search_users))
        .route(
            "/users/:wallet",
            get(get_user_profile)
                .post(update_user_profile)
                .delete(delete_user),
        )
        .route("/home/:wallet", get(get_home_summary))
        .route("/users/:wallet/stats", get(get_user_stats))
        .route(
            "/users/:wallet/notification-settings",
            get(get_notification_settings).put(update_notification_settings),
        )
        // ── /api/users/* aliases ──────────────────────────────────────────────
        .route("/api/check-email", get(check_email))
        .route("/api/users/search", get(search_users))
        .route("/api/home/:wallet", get(get_home_summary))
        .route(
            "/api/users/:wallet",
            get(get_user_profile)
                .post(update_user_profile)
                .delete(delete_user),
        )
        .route("/api/users/:wallet/stats", get(get_user_stats))
        .route(
            "/api/users/:wallet/notification-settings",
            get(get_notification_settings).put(update_notification_settings),
        )
        .route("/api/users/:wallet/push-token", post(register_push_token))
        .route("/users/:wallet/push-token", post(register_push_token))
        // Plural aliases (client uses POST .../push-tokens with { token })
        .route("/api/users/:wallet/push-tokens", post(register_push_token))
        .route("/users/:wallet/push-tokens", post(register_push_token))
        // ── Notifications ────────────────────────────────────────────────────
        .route("/notifications/:wallet", get(list_notifications))
        // Accept both POST and PATCH for marking a notification read.
        .route(
            "/notifications/:id/read",
            post(mark_notification_read).patch(mark_notification_read),
        )
        .route("/notifications/stream/:wallet", get(stream_notifications))
        .route("/api/notifications/:wallet", get(list_notifications))
        .route(
            "/api/notifications/:id/read",
            post(mark_notification_read).patch(mark_notification_read),
        )
        // ── Auth ─────────────────────────────────────────────────────────────
        .route("/api/auth/verify", post(handlers::auth::verify_dynamic))
        // ── Sui ──────────────────────────────────────────────────────────────
        .route("/api/sui/config", get(get_sui_config))
        .route("/api/sui/health", get(get_sui_health))
        .route(
            "/api/sui/networks/:network/config",
            get(get_sui_network_config),
        )
        .route(
            "/api/sui/networks/:network/health",
            get(get_sui_network_health),
        )
        .route(
            "/api/sui/wallets/:wallet/balances",
            get(get_wallet_balances),
        )
        .route(
            "/api/sui/wallets/:wallet/usdc-balance",
            get(get_wallet_usdc_balance),
        )
        .route(
            "/api/sui/wallets/:wallet/dashboard",
            get(get_wallet_dashboard),
        )
        .route(
            "/api/sui/networks/:network/wallets/:wallet/balances",
            get(get_network_wallet_balances_handler),
        )
        .route(
            "/api/sui/networks/:network/wallets/:wallet/usdc-balance",
            get(get_network_wallet_usdc_balance_handler),
        )
        .route(
            "/api/sui/networks/:network/wallets/:wallet/dashboard",
            get(get_network_wallet_dashboard_handler),
        )
        // ── Generic funding provider layer ───────────────────────────────────
        .route("/api/ramps/providers", get(list_ramp_providers))
        .route("/api/ramps/session", post(create_dynamic_ramp_session))
        // ── Programmable payment intents ────────────────────────────────────
        .route("/api/payments/intents", post(create_payment_intent))
        .route("/api/payments/intents/:id", get(get_payment_intent))
        .route(
            "/api/payments/intents/:id/onramp-session",
            post(create_payment_intent_onramp_session),
        )
        .route("/api/payments/intents/:id/ptb", get(get_payment_intent_ptb))
        // ── Stake receipt secondary market ──────────────────────────────────
        .route(
            "/api/receipt-market/listings",
            get(list_receipt_listings).post(create_receipt_listing),
        )
        .route("/api/receipt-market/listings/:id", get(get_receipt_listing))
        .route(
            "/api/receipt-market/listings/:id/activate",
            post(activate_receipt_listing),
        )
        .route(
            "/api/receipt-market/listings/:id/list-ptb",
            get(get_receipt_listing_list_ptb),
        )
        .route(
            "/api/receipt-market/listings/:id/buy-ptb",
            get(get_receipt_listing_buy_ptb),
        )
        .route(
            "/api/receipt-market/listings/:id/mark-sold",
            post(mark_receipt_listing_sold),
        )
        // ── Transak on-ramp fallback ─────────────────────────────────────────
        .route("/api/transak/config", get(get_transak_config))
        .route("/api/transak/widget-url", post(create_transak_widget_url))
        .route("/api/transak/quote", post(get_transak_quote))
        // ── File upload ──────────────────────────────────────────────────────
        .route("/api/uploads", post(upload_file))
        // Alias: client uploads dispute/evidence images to /api/files
        .route("/api/files", post(upload_file))
        // ── Walrus artifacts / agent evidence ───────────────────────────────
        .route("/api/walrus/config", get(get_walrus_config))
        .route("/api/walrus/artifacts", post(create_walrus_artifact))
        .route("/api/walrus/artifacts/:id", get(get_walrus_artifact))
        .route("/api/walrus/blobs/:blob_id/url", get(get_walrus_blob_url))
        .route(
            "/api/agents/outcome-proposals",
            post(submit_agent_outcome_proposal),
        )
        .route("/api/admin/agent-runs", get(list_admin_agent_runs))
        .route("/api/admin/agent-runs/:id", get(get_admin_agent_run))
        // ── Webhooks (organizer systems) ─────────────────────────────────────
        .route(
            "/api/webhooks/match-result",
            post(handle_match_result_webhook),
        )
        .route(
            "/api/webhooks/pandascore",
            post(handle_pandascore_webhook),
        )
        // ── P2P wagers (1-v-1) ───────────────────────────────────────────────
        .route("/api/wagers", get(list_wagers).post(create_wager))
        .route("/api/wagers/mine", get(list_my_wagers))
        .route("/api/wagers/create-ptb", post(create_wager_ptb))
        .route("/api/wagers/:address", get(get_wager))
        .route("/api/wagers/:address/accept", post(accept_wager))
        .route("/api/wagers/:address/accept-ptb", get(accept_wager_ptb))
        .route("/api/wagers/:address/cancel", post(cancel_wager))
        .route("/api/wagers/:address/cancel-ptb", get(cancel_wager_ptb))
        .route("/api/wagers/:address/decline", post(decline_wager))
        .route("/api/wagers/:address/resolve-ptb", get(resolve_wager_ptb))
        .route("/api/wagers/:address/status", post(update_wager_status))
        .route("/api/wagers/:address/declare-winner", post(declare_winner))
        .route("/api/wagers/:address/artifacts", get(list_wager_artifacts))
        .route(
            "/api/wagers/:address/disputes",
            get(list_disputes).post(submit_dispute),
        )
        // ── Tournament / Match Betting (Pool Staking) ────────────────────────
        .route(
            "/api/tournaments",
            get(list_tournaments).post(create_tournament),
        )
        .route(
            "/api/tournaments/source/grid",
            get(get_tournament_source_grid),
        )
        .route(
            "/api/tournaments/source/grid/probe",
            post(probe_grid_tournaments),
        )
        .route(
            "/api/tournaments/source/grid/sync",
            post(sync_grid_tournaments),
        )
        .route(
            "/api/tournaments/source/pandascore",
            get(get_tournament_source_pandascore),
        )
        .route(
            "/api/tournaments/source/pandascore/probe",
            post(probe_pandascore_tournaments),
        )
        .route(
            "/api/tournaments/source/pandascore/sync",
            post(sync_pandascore_tournaments),
        )
        .route("/api/tournaments/:id", get(get_tournament))
        .route(
            "/api/admin/tournaments/:id/pool",
            post(configure_tournament_pool),
        )
        .route(
            "/api/admin/tournaments/pools/backfill",
            post(backfill_tournament_pools),
        )
        .route(
            "/api/admin/tournaments/:id/stakes/sync",
            post(sync_tournament_stakes),
        )
        .route(
            "/api/tournaments/:id/outcome-proposals",
            get(list_outcome_proposals).post(create_outcome_proposal),
        )
        .route(
            "/api/outcome-proposals/:id/review",
            post(review_outcome_proposal),
        )
        .route("/api/admin/organizers", get(list_admin_organizers))
        .route("/api/admin/organizers/:wallet", get(get_admin_organizer))
        .route(
            "/api/admin/outcome-proposals",
            get(list_admin_outcome_proposals),
        )
        .route(
            "/api/admin/outcome-proposals/:id",
            get(get_admin_outcome_proposal),
        )
        .route(
            "/api/organizer/tournaments",
            get(list_organizer_tournaments).post(create_organizer_tournament),
        )
        .route(
            "/api/organizer/tournaments/:id/matches",
            post(create_organizer_match),
        )
        .route("/api/organizers/apply", post(apply_organizer))
        .route(
            "/api/organizers/kyc-session",
            post(create_organizer_kyc_session),
        )
        .route("/api/organizers/:wallet", get(get_organizer))
        .route("/api/organizers/:wallet/review", post(review_organizer))
        .route("/api/tournaments/:id/stake", post(place_stake))
        .route("/api/tournaments/:id/calculate", post(calculate_payout))
        .route("/api/tournaments/:id/stakes", get(list_tournament_stakes))
        .route("/api/tournaments/:id/resolve", post(resolve_tournament))
        .route("/api/tournaments/:id/cancel", post(cancel_tournament))
        .route("/api/tournaments/:id/sync", post(sync_tournament))
        .route("/api/users/:wallet/stakes", get(get_user_stakes))
        .route("/api/users/:wallet/stake-stats", get(get_user_stake_stats))
        // ── WebSocket ────────────────────────────────────────────────────────
        .route("/ws/notifications/:wallet", get(ws_notifications))
        // ── Prometheus metrics ───────────────────────────────────────────────
        .route("/metrics", get(get_metrics))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(Arc::clone(&state));

    // ── Serve uploaded files (if UPLOAD_DIR is configured) ────────────────────
    if let Ok(upload_dir) = std::env::var("UPLOAD_DIR") {
        app = app.nest_service("/uploads", ServeDir::new(upload_dir));
    }

    // ── Start server ──────────────────────────────────────────────────────────
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Wager API listening on {}", addr);

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

/// Spawns a background task that periodically syncs PandaScore matches so the
/// lobby stays fresh and stale `upcoming` statuses get advanced to
/// running/finished without any manual admin trigger.
///
/// Controlled by env:
/// - `PANDASCORE_AUTO_SYNC` (default: enabled when PandaScore is configured)
/// - `PANDASCORE_SYNC_INTERVAL_MINUTES` (default: 30)
fn spawn_pandascore_scheduler(state: Arc<AppState>) {
    if !state.pandascore.config().configured() {
        tracing::info!("PandaScore auto-sync disabled: provider not configured");
        return;
    }

    let enabled = std::env::var("PANDASCORE_AUTO_SYNC")
        .ok()
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(true);
    if !enabled {
        tracing::info!("PandaScore auto-sync disabled via PANDASCORE_AUTO_SYNC");
        return;
    }

    let interval_minutes = std::env::var("PANDASCORE_SYNC_INTERVAL_MINUTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(10);

    // When enabled, the scheduler also creates on-chain pools for newly-complete
    // matches after each sync, so the import->stakeable pipeline is hands-off.
    // Defaults to ON so a missing env var doesn't silently leave upcoming
    // matches un-stakeable; set POOL_AUTO_BACKFILL=false to disable.
    let auto_backfill = std::env::var("POOL_AUTO_BACKFILL")
        .ok()
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(true);
    // Cap pools created per cycle so a cold start doesn't fire hundreds of
    // on-chain transactions at once.
    let backfill_limit = std::env::var("POOL_AUTO_BACKFILL_LIMIT")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(25);

    tracing::info!(
        "PandaScore auto-sync enabled: every {} minute(s) (auto_backfill={})",
        interval_minutes,
        auto_backfill
    );

    tokio::spawn(async move {
        // Give the server a moment to finish binding before the first sync.
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let mut ticker =
            tokio::time::interval(std::time::Duration::from_secs(interval_minutes * 60));

        loop {
            ticker.tick().await;
            // Default request uses configured statuses (upcoming, running, past)
            // and game slugs — pulling `past` is what advances stale rows.
            let req = models::PandascoreSyncRequest {
                statuses: None,
                videogame_slugs: None,
                tournament_id: None,
                league_id: None,
                serie_id: None,
                sort: None,
                per_page: None,
                max_pages: None,
            };
            match handlers::tournament::run_pandascore_sync(&state, &req).await {
                Ok(resp) => tracing::info!(
                    "PandaScore auto-sync: fetched={} synced={} resolved={} errors={}",
                    resp.fetched,
                    resp.synced,
                    resp.resolved,
                    resp.errors.len()
                ),
                Err(e) => tracing::warn!("PandaScore auto-sync failed: {}", e),
            }

            if auto_backfill {
                match handlers::tournament::run_pool_backfill(
                    &state,
                    None,
                    None,
                    Some(backfill_limit),
                    None,
                )
                .await
                {
                    Ok(resp) => tracing::info!(
                        "Pool auto-backfill: attempted={} created={} skipped={} failed={}",
                        resp.attempted,
                        resp.created,
                        resp.skipped,
                        resp.failed
                    ),
                    Err(e) => tracing::warn!("Pool auto-backfill failed: {}", e),
                }
            }
        }
    });
}

/// Fast near-real-time poller for live and just-finished matches. PandaScore
/// webhooks are plan-gated, so this is the no-webhook path to keep live status,
/// scores, and finish->resolution fresh. It only pulls `running` + recently
/// modified `past` matches across all games (≈2 API calls per cycle), so it is
/// cheap enough to run on a tight interval.
///
/// Controlled by env:
/// - `PANDASCORE_LIVE_POLL_SECONDS` (default: 60, set 0 to disable)
fn spawn_pandascore_live_poller(state: Arc<AppState>) {
    if !state.pandascore.config().configured() {
        return;
    }
    let enabled = std::env::var("PANDASCORE_AUTO_SYNC")
        .ok()
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(true);
    if !enabled {
        return;
    }

    let poll_seconds = std::env::var("PANDASCORE_LIVE_POLL_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60);
    if poll_seconds == 0 {
        tracing::info!("PandaScore live poller disabled (PANDASCORE_LIVE_POLL_SECONDS=0)");
        return;
    }

    tracing::info!(
        "PandaScore live poller enabled: every {} second(s)",
        poll_seconds
    );

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(poll_seconds));
        loop {
            ticker.tick().await;
            // running + freshest past, all games (empty slugs => one call per
            // status), newest-changed first so a just-finished match resolves
            // within one cycle.
            let req = models::PandascoreSyncRequest {
                statuses: Some(vec!["running".to_string(), "past".to_string()]),
                videogame_slugs: Some(vec![]),
                tournament_id: None,
                league_id: None,
                serie_id: None,
                sort: Some("-modified_at".to_string()),
                per_page: Some(50),
                max_pages: Some(1),
            };
            match handlers::tournament::run_pandascore_sync(&state, &req).await {
                Ok(resp) if resp.resolved > 0 || resp.synced > 0 => tracing::debug!(
                    "PandaScore live poll: synced={} resolved={}",
                    resp.synced,
                    resp.resolved
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!("PandaScore live poll failed: {}", e),
            }
        }
    });
}

/// Background reconciler: indexes on-chain stake events into the `pool_stakes`
/// table for matches that have an active pool. The PTB staking flow settles
/// on-chain without writing to the DB, so without this, DB aggregates
/// (total_stakers, per-side pools) lag behind on-chain reality.
///
/// Controlled by env:
/// - `STAKE_RECONCILE_ENABLED` (default: on when a Sui package is configured)
/// - `STAKE_RECONCILE_SECONDS` (default: 120)
/// - `STAKE_RECONCILE_LIMIT` (matches scanned per cycle, default: 40)
fn spawn_stake_reconciler(state: Arc<AppState>) {
    let enabled = std::env::var("STAKE_RECONCILE_ENABLED")
        .ok()
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(true);
    if !enabled {
        tracing::info!("Stake reconciler disabled via STAKE_RECONCILE_ENABLED");
        return;
    }
    let interval_secs = std::env::var("STAKE_RECONCILE_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(120);
    let scan_limit = std::env::var("STAKE_RECONCILE_LIMIT")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(40);

    tracing::info!(
        "Stake reconciler enabled: every {}s, up to {} pooled matches/cycle",
        interval_secs,
        scan_limit
    );

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            let matches = match state.db.list_matches_with_active_pools(scan_limit).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("stake reconciler: failed to list pooled matches: {}", e);
                    continue;
                }
            };
            let mut indexed_total = 0usize;
            for m in matches {
                match handlers::tournament::run_stake_sync(&state, &m.id.to_string(), None, Some(50usize))
                    .await
                {
                    Ok(resp) => indexed_total += resp.indexed,
                    Err(e) => tracing::debug!("stake reconcile {}: {}", m.id, e),
                }
            }
            if indexed_total > 0 {
                tracing::info!("Stake reconciler: indexed {} new stake(s)", indexed_total);
            }
        }
    });
}

async fn health_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> axum::Json<serde_json::Value> {
    let sui = state.sui.config();
    let ramp = state.ramp.config();
    let transak = state.transak.config();
    let grid = state.grid.config();
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": "wager-api",
        "version": env!("CARGO_PKG_VERSION"),
        "grid": {
            "enabled": grid.enabled,
            "configured": grid.configured(),
            "base_url": grid.base_url.clone(),
            "matches_path": grid.matches_path.clone(),
            "api_style": grid.api_style.clone(),
        },
        "sui": {
            "active_network": sui.active_network.clone(),
            "networks": sui.networks.clone(),
            "platform_signer_address": services::SuiService::platform_signer_address(),
        },
        "transak": {
            "enabled": transak.enabled,
            "environment": transak.environment.clone(),
            "default_network": transak.default_network.clone(),
            "default_crypto_currency": transak.default_crypto_currency.clone(),
            "default_fiat_currency": transak.default_fiat_currency.clone(),
            "partner_fee_bps": transak.partner_fee_bps,
        },
        "ramps": {
            "primary_provider": ramp.primary_provider.clone(),
            "dynamic_onramp_enabled": ramp.dynamic_onramp_enabled,
            "manual_deposit_enabled": ramp.manual_deposit_enabled,
            "default_network": ramp.default_network.clone(),
            "default_crypto_currency": ramp.default_crypto_currency.clone(),
            "default_fiat_currency": ramp.default_fiat_currency.clone(),
            "partner_fee_bps": ramp.partner_fee_bps,
        },
        "walrus": {
            "enabled": state.walrus.config().enabled,
            "configured": state.walrus.config().configured(),
            "network": state.walrus.config().network.clone(),
            "aggregator_url": state.walrus.config().aggregator_url.clone(),
        }
    }))
}
