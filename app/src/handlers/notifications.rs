use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::{
    extract::{Path, Query, State},
    response::sse::{self},
    Json,
};
use futures_util::stream;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::models::*;
use crate::services::auth;
use crate::services::sui::SuiService;

use crate::state::AppState;

type AppResult<T> = Result<
    Json<crate::models::ApiResponse<T>>,
    (axum::http::StatusCode, Json<crate::models::ApiResponse<()>>),
>;

fn internal_error(
    msg: impl Into<String>,
) -> (axum::http::StatusCode, Json<crate::models::ApiResponse<()>>) {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(crate::models::ApiResponse::err(msg)),
    )
}

fn gateway_timeout(
    msg: impl Into<String>,
) -> (axum::http::StatusCode, Json<crate::models::ApiResponse<()>>) {
    (
        axum::http::StatusCode::GATEWAY_TIMEOUT,
        Json(crate::models::ApiResponse::err(msg)),
    )
}

fn unauthorized(
    msg: impl Into<String>,
) -> (axum::http::StatusCode, Json<crate::models::ApiResponse<()>>) {
    (
        axum::http::StatusCode::UNAUTHORIZED,
        Json(crate::models::ApiResponse::err(msg)),
    )
}

fn not_found(
    msg: impl Into<String>,
) -> (axum::http::StatusCode, Json<crate::models::ApiResponse<()>>) {
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(crate::models::ApiResponse::err(msg)),
    )
}

fn extract_wallet_from_jwt(
    headers: &HeaderMap,
) -> Result<String, (axum::http::StatusCode, Json<crate::models::ApiResponse<()>>)> {
    let secret = std::env::var("AUTH_JWT_SECRET")
        .map_err(|_| internal_error("server missing AUTH_JWT_SECRET"))?;

    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized("missing Authorization header"))?;

    auth::verify_jwt_get_wallet(token, &secret)
        .map_err(|e| unauthorized(format!("invalid token: {}", e)))
}

fn normalize_wallet(wallet: &str) -> String {
    SuiService::normalize_address(wallet).unwrap_or_else(|| wallet.to_string())
}

pub async fn list_notifications(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
    Query(query): Query<NotificationListQuery>,
    headers: HeaderMap,
) -> AppResult<Vec<NotificationRecord>> {
    let auth_wallet = normalize_wallet(&extract_wallet_from_jwt(&headers)?);
    let requested_wallet = normalize_wallet(&wallet);
    if auth_wallet != requested_wallet {
        return Err(unauthorized("wallet in token does not match request"));
    }

    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);

    let rows = state
        .db
        .list_notifications_for_user(&requested_wallet, limit, offset)
        .await
        .map_err(|e| {
            let message = e.to_string();
            if message.contains("timed out listing notifications") {
                gateway_timeout("notifications query timed out")
            } else {
                internal_error(message)
            }
        })?;

    Ok(Json(crate::models::ApiResponse::ok(rows)))
}

pub async fn mark_read(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<()> {
    let auth_wallet = normalize_wallet(&extract_wallet_from_jwt(&headers)?);
    let updated = state
        .db
        .mark_notification_read_for_user(&id, &auth_wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    if !updated {
        return Err(not_found("Notification not found"));
    }

    Ok(Json(crate::models::ApiResponse::ok(())))
}

/// SSE stream for realtime notifications for a wallet
pub async fn stream_notifications(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Extract token from Authorization header. Avoid query-string tokens because
    // URLs are commonly persisted in browser history and proxy/access logs.
    let token_opt = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string()));

    let token = match token_opt {
        Some(t) => t,
        None => {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(crate::models::ApiResponse::err("missing token")),
            )
                .into_response()
        }
    };

    let secret = match std::env::var("AUTH_JWT_SECRET") {
        Ok(s) => s,
        Err(_) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(crate::models::ApiResponse::err(
                    "server missing AUTH_JWT_SECRET",
                )),
            )
                .into_response()
        }
    };

    match auth::verify_jwt_get_wallet(&token, &secret) {
        Ok(claim_wallet) if normalize_wallet(&claim_wallet) == normalize_wallet(&wallet) => {}
        _ => {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(crate::models::ApiResponse::err("invalid token")),
            )
                .into_response()
        }
    }

    let wallet = normalize_wallet(&wallet);

    // Create a BroadcastStream from the AppState sender
    let rx = state.notif_tx.subscribe();
    let bs = BroadcastStream::new(rx);

    // Stream events, filter by wallet
    let out_stream = bs.filter_map(move |res| {
        let wallet_cloned = wallet.clone();
        match res {
            Ok((to_wallet, payload)) if normalize_wallet(&to_wallet) == wallet_cloned => {
                let data = match serde_json::to_string(&payload) {
                    Ok(s) => s,
                    Err(_) => return None,
                };
                Some(Ok::<_, axum::Error>(sse::Event::default().data(data)))
            }
            _ => None,
        }
    });

    // Keep the stream alive with a ping every 30s to prevent proxies closing
    let ping_stream = stream::iter(std::iter::repeat_with(|| {
        Ok::<_, axum::Error>(sse::Event::default().comment("ping"))
    }))
    .throttle(std::time::Duration::from_secs(30));

    let merged = futures_util::stream::select(out_stream, ping_stream);

    sse::Sse::new(merged).into_response()
}

/// WebSocket endpoint for realtime notifications for a wallet
pub async fn ws_notifications(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Token extraction like SSE; do not accept query-string JWTs.
    let token_opt = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string()));

    let token = match token_opt {
        Some(t) => t,
        None => {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(crate::models::ApiResponse::err("missing token")),
            )
                .into_response()
        }
    };

    let secret = match std::env::var("AUTH_JWT_SECRET") {
        Ok(s) => s,
        Err(_) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(crate::models::ApiResponse::err(
                    "server missing AUTH_JWT_SECRET",
                )),
            )
                .into_response()
        }
    };

    match auth::verify_jwt_get_wallet(&token, &secret) {
        Ok(claim_wallet) if normalize_wallet(&claim_wallet) == normalize_wallet(&wallet) => {}
        _ => {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(crate::models::ApiResponse::err("invalid token")),
            )
                .into_response()
        }
    }

    let wallet = normalize_wallet(&wallet);
    ws.on_upgrade(move |socket| handle_ws(socket, state, wallet))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>, wallet: String) {
    let mut rx = state.notif_tx.subscribe();

    loop {
        tokio::select! {
            biased;
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        // allow client to ACK a notification by sending JSON {"ack": "<id>"}
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&txt) {
                            if let Some(id) = val.get("ack").and_then(|v| v.as_str()) {
                                if let Err(e) = state.db.mark_notification_read_for_user(id, &wallet).await {
                                    tracing::error!("failed to mark notification read: {}", e);
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        let _ = socket.send(Message::Pong(vec![])).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    _ => {}
                }
            }
            res = rx.recv() => {
                match res {
                    Ok((to_wallet, payload)) if normalize_wallet(&to_wallet) == wallet => {
                        if let Ok(text) = serde_json::to_string(&payload) {
                            let res: Result<(), axum::Error> = socket.send(Message::Text(text)).await;
                            if res.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("ws client lagged: {} messages", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}
