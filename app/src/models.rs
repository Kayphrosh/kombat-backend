// app/src/models.rs
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;
use chrono::{DateTime, Utc};

// ─── API Request / Response Models ────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateWagerRequest {
    pub initiator: String,
    pub stake_lamports: u64,
    pub description: String,
    pub expiry_ts: i64,
    pub resolution_source: String,
    pub resolver: String,
    pub challenger_address: Option<String>,
    pub initiator_option: Option<String>,
    pub oracle_feed: Option<String>,
    pub oracle_target: Option<i64>,
    pub oracle_initiator_wins_above: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterPushTokenRequest {
    pub expo_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AcceptWagerRequest {
    pub challenger: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResolveWagerRequest {
    pub winner: String,
    pub caller: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsentRequest {
    pub participant: String,
    pub declared_winner: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisputeRequest {
    pub opener: String,
    pub description: Option<String>,
    pub evidence_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisputeSubmissionRequest {
    pub submitter: String,
    pub description: String,
    pub evidence_url: Option<String>,
    pub declared_winner: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct DisputeSubmissionRecord {
    pub id: uuid::Uuid,
    pub wager_address: String,
    pub submitter: String,
    pub description: String,
    pub evidence_url: Option<String>,
    pub declared_winner: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// ─── Wager Record ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WagerRecord {
    pub id: Uuid,
    pub on_chain_address: String,
    pub wager_id: i64,
    pub initiator: String,
    pub challenger: Option<String>,
    pub stake_lamports: i64,
    pub description: String,
    pub status: String,
    pub resolution_source: String,
    pub resolver: String,
    pub expiry_ts: i64,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub winner: Option<String>,
    pub protocol_fee_bps: i16,
    pub oracle_feed: Option<String>,
    pub oracle_target: Option<i64>,
    pub dispute_opened_at: Option<DateTime<Utc>>,
    pub dispute_opener: Option<String>,
    pub initiator_option: Option<String>,
    pub creator_declared_winner: Option<String>,
    pub challenger_declared_winner: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WagerDetailResponse {
    #[serde(flatten)]
    pub wager: WagerRecord,
    pub initiator_name: Option<String>,
    pub initiator_avatar: Option<String>,
    pub challenger_name: Option<String>,
    pub challenger_avatar: Option<String>,
    pub challenger_option: Option<String>,
}

// ─── User Profile ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct UserRecord {
    pub id: Uuid,
    pub wallet_address: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub wins: i32,
    pub losses: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateProfileRequest {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

// ─── Notifications ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct NotificationRecord {
    pub id: uuid::Uuid,
    pub user_wallet: String,
    pub kind: String,
    pub payload: Option<JsonValue>,
    pub is_read: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NotificationListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ─── Nonce (Solana auth) ───────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct NonceRecord {
    pub id: uuid::Uuid,
    pub wallet: String,
    pub nonce: String,
    pub used: bool,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ─── Transaction Response ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct TxResponse {
    pub transaction_b64: String,
    pub description: String,
}

// ─── Standard API Response wrapper ───────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self { success: true, data: Some(data), error: None }
    }
}

impl ApiResponse<()> {
    pub fn err(msg: impl Into<String>) -> Self {
        Self { success: false, data: None, error: Some(msg.into()) }
    }
}

// ─── List / Filter Query Params ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WagerListQuery {
    pub initiator: Option<String>,
    pub challenger: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ─── User Stats ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct UserStats {
    pub live_count: i64,
    pub completed_count: i64,
    pub total_stake: i64,
    pub total_won: i64,
}

// ─── Notification Settings ───────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct NotificationSettings {
    pub user_wallet: String,
    pub challenges: bool,
    pub funds: bool,
    pub disputes: bool,
    pub marketing: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateNotificationSettings {
    pub challenges: Option<bool>,
    pub funds: Option<bool>,
    pub disputes: Option<bool>,
    pub marketing: Option<bool>,
}

// ─── Dynamic SDK Auth ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DynamicAuthRequest {
    #[serde(alias = "dynamicToken")]
    pub dynamic_token: String,
}

#[derive(Debug, Serialize)]
pub struct DynamicAuthResponse {
    pub user: UserRecord,
    #[serde(rename = "accessToken")]
    pub access_token: String,
}

// ─── Upload Response ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadResponse {
    pub url: String,
}