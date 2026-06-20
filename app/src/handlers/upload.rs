use axum::{extract::State, http::StatusCode, Json};
use axum_extra::extract::Multipart;
use std::sync::Arc;

use crate::{
    models::{ApiResponse, UploadResponse},
    state::AppState,
};

type AppResult<T> = Result<Json<ApiResponse<T>>, (StatusCode, Json<ApiResponse<()>>)>;

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::BAD_REQUEST, Json(ApiResponse::err(msg)))
}

fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiResponse::err(msg)),
    )
}

/// POST /api/uploads
/// Accepts multipart/form-data with `file` and `type` fields.
pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> AppResult<UploadResponse> {
    let mut file_data: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;
    let mut file_type: String = "general".to_string();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                file_name = field.file_name().map(|s| s.to_string());
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| bad_request(format!("Failed to read file: {}", e)))?;
                file_data = Some(bytes.to_vec());
            }
            "type" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| bad_request(format!("Failed to read type: {}", e)))?;
                file_type = text;
            }
            _ => {} // ignore unknown fields
        }
    }

    let data = file_data.ok_or_else(|| bad_request("Missing 'file' field"))?;
    let original_name = file_name.unwrap_or_else(|| "upload.bin".to_string());

    // Durable categories (tournament rules, brackets, team logos, evidence) are
    // stored on Walrus so they survive container restarts and are verifiable.
    // Everything else uses the local upload service.
    if is_durable_category(&file_type) && state.walrus.config().configured() {
        let content_type = guess_content_type(&original_name);
        let epochs = state.walrus.config().epochs;
        match state
            .walrus
            .store_bytes(data.clone(), content_type, epochs)
            .await
        {
            Ok(stored) => {
                if let Some(url) = stored.aggregator_url {
                    return Ok(Json(ApiResponse::ok(UploadResponse { url })));
                }
                tracing::warn!("Walrus upload returned no aggregator url; falling back to local");
            }
            Err(e) => {
                tracing::warn!(
                    "Walrus upload failed ({}); falling back to local: {}",
                    file_type,
                    e
                );
            }
        }
    }

    let upload_svc = state
        .upload_service
        .as_ref()
        .ok_or_else(|| internal_error("Upload service not configured"))?;

    let url = upload_svc
        .save_file(data, &original_name, &file_type)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(UploadResponse { url })))
}

/// File categories that warrant durable, verifiable Walrus storage.
fn is_durable_category(file_type: &str) -> bool {
    matches!(
        file_type.trim().to_ascii_lowercase().as_str(),
        "team_logo"
            | "tournament_rules"
            | "rules"
            | "bracket"
            | "tournament_bracket"
            | "evidence"
            | "dispute_evidence"
    )
}

/// Minimal content-type guess from a filename extension for Walrus uploads.
fn guess_content_type(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}
