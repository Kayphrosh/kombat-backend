use axum::{http::StatusCode, Json};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::{models::ApiResponse, state::AppState};

pub type HandlerError = (StatusCode, Json<ApiResponse<()>>);

fn internal_error(msg: impl Into<String>) -> HandlerError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiResponse::err(msg)),
    )
}

pub fn notification_action(
    label: &str,
    action_type: &str,
    method: &str,
    endpoint: String,
    params: Value,
) -> Value {
    json!({
        "label": label,
        "type": action_type,
        "method": method,
        "endpoint": endpoint,
        "params": params,
    })
}

pub fn notification_payload(title: &str, body: &str, action: Value, entities: Value) -> Value {
    json!({
        "title": title,
        "body": body,
        "action": action,
        "entities": entities,
    })
}

pub async fn notify_user(
    state: &Arc<AppState>,
    wallet: &str,
    kind: &str,
    payload: Value,
) -> Result<(), HandlerError> {
    let notification = state
        .db
        .create_notification(wallet, kind, Some(payload))
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let realtime_payload =
        serde_json::to_value(notification).map_err(|e| internal_error(e.to_string()))?;
    let _ = state.notif_tx.send((wallet.to_string(), realtime_payload));

    Ok(())
}

pub async fn notify_user_best_effort(
    state: &Arc<AppState>,
    wallet: &str,
    kind: &str,
    payload: Value,
) {
    if let Err((_, Json(err))) = notify_user(state, wallet, kind, payload).await {
        tracing::error!(
            "failed to create notification kind={} wallet={}: {}",
            kind,
            wallet,
            err.error.unwrap_or_else(|| "unknown error".to_string())
        );
    }
}
