use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use std::sync::Arc;

use crate::{
    models::{
        AdminOrganizerQuery, AdminOutcomeProposalQuery, ApiResponse, OrganizerProfileRecord,
        OutcomeProposalRecord,
    },
    services::sui::SuiService,
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

fn unauthorized(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::UNAUTHORIZED, Json(ApiResponse::err(msg)))
}

fn not_found(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::NOT_FOUND, Json(ApiResponse::err(msg)))
}

fn verify_admin(headers: &HeaderMap) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let admin_token = std::env::var("AUTH_ADMIN_TOKEN")
        .map_err(|_| internal_error("server missing AUTH_ADMIN_TOKEN"))?;
    let got = headers
        .get("x-admin-token")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer ").map(ToString::to_string))
        });

    if got.as_deref() != Some(admin_token.as_str()) {
        return Err(unauthorized("invalid admin token"));
    }
    Ok(())
}

pub async fn list_admin_organizers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AdminOrganizerQuery>,
) -> AppResult<Vec<OrganizerProfileRecord>> {
    verify_admin(&headers)?;
    let organizers = state
        .db
        .list_admin_organizer_profiles(&query)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(organizers)))
}

pub async fn get_admin_organizer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(wallet): Path<String>,
) -> AppResult<OrganizerProfileRecord> {
    verify_admin(&headers)?;
    let wallet = SuiService::normalize_address(&wallet)
        .ok_or_else(|| bad_request("Invalid organizer wallet address"))?;
    let organizer = state
        .db
        .get_organizer_profile(&wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Organizer profile not found"))?;

    Ok(Json(ApiResponse::ok(organizer)))
}

pub async fn list_admin_outcome_proposals(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AdminOutcomeProposalQuery>,
) -> AppResult<Vec<OutcomeProposalRecord>> {
    verify_admin(&headers)?;
    let proposals = state
        .db
        .list_admin_outcome_proposals(&query)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(proposals)))
}

pub async fn get_admin_outcome_proposal(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(proposal_id): Path<String>,
) -> AppResult<OutcomeProposalRecord> {
    verify_admin(&headers)?;
    let proposal_uuid =
        uuid::Uuid::parse_str(&proposal_id).map_err(|_| bad_request("Invalid proposal id"))?;
    let proposal = state
        .db
        .get_outcome_proposal(proposal_uuid)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Outcome proposal not found"))?;

    Ok(Json(ApiResponse::ok(proposal)))
}
