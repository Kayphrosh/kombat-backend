// app/src/handlers/upload.rs
//! File upload handler — accepts multipart/form-data and stores files.

use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use axum_extra::extract::Multipart;
use std::sync::Arc;

use crate::{
    handlers::wager::AppState,
    models::{ApiResponse, UploadResponse},
};

type AppResult<T> = Result<Json<ApiResponse<T>>, (StatusCode, Json<ApiResponse<()>>)>;

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::BAD_REQUEST, Json(ApiResponse::err(msg)))
}

fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiResponse::err(msg)))
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
                let bytes = field.bytes().await.map_err(|e| bad_request(format!("Failed to read file: {}", e)))?;
                file_data = Some(bytes.to_vec());
            }
            "type" => {
                let text = field.text().await.map_err(|e| bad_request(format!("Failed to read type: {}", e)))?;
                file_type = text;
            }
            _ => {} // ignore unknown fields
        }
    }

    let data = file_data.ok_or_else(|| bad_request("Missing 'file' field"))?;
    let original_name = file_name.unwrap_or_else(|| "upload.bin".to_string());

    let upload_svc = state.upload_service.as_ref()
        .ok_or_else(|| internal_error("Upload service not configured"))?;

    let url = upload_svc.save_file(data, &original_name, &file_type)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(UploadResponse { url })))
}
