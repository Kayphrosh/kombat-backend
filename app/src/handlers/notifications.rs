use axum::{extract::{Path, Query, State}, Json, response::sse::{self}};
use axum::http::HeaderMap;
use std::collections::HashMap;
use axum::extract::ws::{WebSocketUpgrade, WebSocket, Message};
use axum::response::IntoResponse;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use futures_util::stream;
use tokio_stream::StreamExt;

use crate::models::*;
use crate::services::auth;

use crate::handlers::wager::AppState;

type AppResult<T> = Result<Json<crate::models::ApiResponse<T>>, (axum::http::StatusCode, Json<crate::models::ApiResponse<()>>)>;

fn bad_request(msg: impl Into<String>) -> (axum::http::StatusCode, Json<crate::models::ApiResponse<()>>) {
    (axum::http::StatusCode::BAD_REQUEST, Json(crate::models::ApiResponse::err(msg)))
}

fn internal_error(msg: impl Into<String>) -> (axum::http::StatusCode, Json<crate::models::ApiResponse<()>>) {
    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err(msg)))
}

pub async fn list_notifications(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
    Query(query): Query<NotificationListQuery>,
) -> AppResult<Vec<NotificationRecord>> {
    let limit = query.limit.unwrap_or(50).min(200);
    let offset = query.offset.unwrap_or(0);

    let rows = state.db.list_notifications_for_user(&wallet, limit, offset)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(crate::models::ApiResponse::ok(rows)))
}

pub async fn mark_read(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<()> {
    state.db.mark_notification_read(&id)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(crate::models::ApiResponse::ok(())))
}

/// SSE stream for realtime notifications for a wallet
pub async fn stream_notifications(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    // Extract token from Authorization header (Bearer) or ?token= fallback
    let token_opt = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string()))
        .or_else(|| query.get("token").cloned());

    let token = match token_opt {
        Some(t) => t,
        None => return (axum::http::StatusCode::UNAUTHORIZED, Json(crate::models::ApiResponse::err("missing token"))).into_response(),
    };

    let secret = match std::env::var("AUTH_JWT_SECRET") {
        Ok(s) => s,
        Err(_) => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err("server missing AUTH_JWT_SECRET"))).into_response(),
    };

    match auth::verify_jwt_get_wallet(&token, &secret) {
        Ok(claim_wallet) if claim_wallet == wallet.clone() => {}
        _ => return (axum::http::StatusCode::UNAUTHORIZED, Json(crate::models::ApiResponse::err("invalid token"))).into_response(),
    }

    // Create a BroadcastStream from the AppState sender
    let rx = state.notif_tx.subscribe();
    let bs = BroadcastStream::new(rx);

    // Stream events, filter by wallet
    let out_stream = bs.filter_map(move |res| {
        let wallet_cloned = wallet.clone();
        match res {
            Ok((to_wallet, payload)) if to_wallet == wallet_cloned => {
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
    let ping_stream = stream::iter(std::iter::repeat_with(|| Ok::<_, axum::Error>(sse::Event::default().comment("ping")))).throttle(std::time::Duration::from_secs(30));

    let merged = futures_util::stream::select(out_stream, ping_stream);

    sse::Sse::new(merged).into_response()
}

    /// WebSocket endpoint for realtime notifications for a wallet
    pub async fn ws_notifications(
        State(state): State<Arc<AppState>>,
        Path(wallet): Path<String>,
        headers: HeaderMap,
        Query(query): Query<HashMap<String, String>>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        // token extraction like SSE
        let token_opt = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string()))
            .or_else(|| query.get("token").cloned());

        let token = match token_opt {
            Some(t) => t,
            None => return (axum::http::StatusCode::UNAUTHORIZED, Json(crate::models::ApiResponse::err("missing token"))).into_response(),
        };

        let secret = match std::env::var("AUTH_JWT_SECRET") {
            Ok(s) => s,
            Err(_) => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err("server missing AUTH_JWT_SECRET"))).into_response(),
        };

        match auth::verify_jwt_get_wallet(&token, &secret) {
            Ok(claim_wallet) if claim_wallet == wallet.clone() => {}
            _ => return (axum::http::StatusCode::UNAUTHORIZED, Json(crate::models::ApiResponse::err("invalid token"))).into_response(),
        }

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
                                    if let Err(e) = state.db.mark_notification_read(id).await {
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
                        Ok((to_wallet, payload)) if to_wallet == wallet => {
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
