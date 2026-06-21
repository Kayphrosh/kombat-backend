// app/src/models.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

// ─── API Request / Response Models ────────────────────────────────────────────

/// Index a P2P wager after it has been created on-chain by the client wallet.
/// The backend tracks state and the social layer (accept, declared winners,
/// disputes, win/loss); the on-chain wager object is created client-side.
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateWagerRequest {
    pub on_chain_address: String,
    pub wager_id: i64,
    pub initiator: String,
    pub stake_usdc: u64,
    pub description: String,
    pub expiry_ts: i64,
    pub resolution_source: String,
    pub resolver: String,
    pub challenger_address: Option<String>,
    pub initiator_option: Option<String>,
    pub protocol_fee_bps: Option<i16>,
    pub oracle_feed: Option<String>,
    pub oracle_target: Option<i64>,
    /// Optional wager terms/agreement to store durably on Walrus.
    pub terms: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWagerStatusRequest {
    pub status: String,
}

/// Build the on-chain transaction to create a P2P wager (before the wager
/// object exists). Mirrors the payment-intent PTB pattern.
#[derive(Debug, Deserialize)]
pub struct WagerCreatePtbRequest {
    pub initiator: String,
    pub stake_usdc: i64,
    pub description: String,
    pub expiry_ts: i64,
    pub resolver: String,
    pub network: Option<String>,
    pub challenger_address: Option<String>,
    pub initiator_option: Option<String>,
}

/// Declared winner for the resolve-PTB.
#[derive(Debug, Deserialize)]
pub struct WagerResolvePtbQuery {
    pub winner: String,
}

/// Generic wager PTB response (reuses the payment PTB building blocks).
#[derive(Debug, Serialize)]
pub struct WagerPtbResponse {
    pub wager_address: Option<String>,
    pub network: String,
    pub can_build: bool,
    pub reason: Option<String>,
    pub coin_type: Option<String>,
    pub package_id: Option<String>,
    pub expected_object_type: String,
    pub steps: Vec<PaymentPtbStep>,
    pub move_call: Option<PaymentMoveCall>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterPushTokenRequest {
    /// Accept both `expo_token` (original) and `token` (client shorthand).
    #[serde(alias = "token")]
    pub expo_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AcceptWagerRequest {
    pub challenger: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CancelWagerRequest {
    pub initiator: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeclineWagerRequest {
    pub challenger: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsentRequest {
    pub participant: String,
    pub declared_winner: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisputeSubmissionRequest {
    pub submitter: String,
    pub description: String,
    pub evidence_url: Option<String>,
    pub declared_winner: Option<String>,
    /// Optional structured evidence to store durably on Walrus. When present,
    /// the resulting aggregator URL is saved as `evidence_url`.
    pub evidence: Option<JsonValue>,
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
    pub stake_usdc: i64,
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
    pub resolution_error: Option<String>,
    pub resolution_attempted_at: Option<DateTime<Utc>>,
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
    pub opponent_wallet: Option<String>,
    pub opponent_name: Option<String>,
    pub opponent_avatar: Option<String>,
    pub expiry_ms: i64,
    pub expiry_unit: String,
    pub address_format: String,
    pub is_legacy: bool,
    pub resolution_error: Option<String>,
    pub resolution_attempted_at: Option<DateTime<Utc>>,
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

#[derive(Debug, Deserialize)]
pub struct UserSearchQuery {
    pub q: Option<String>,
    pub query: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub limit: Option<i64>,
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

// ─── Transaction Response ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
#[allow(dead_code)]
pub struct TxResponse {
    pub transaction_b64: String,
    pub description: String,
    pub address: Option<String>,
    pub on_chain_address: Option<String>,
    pub wager_address: Option<String>,
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
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }
}

impl ApiResponse<()> {
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
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

#[derive(Debug, Deserialize)]
pub struct MineWagersQuery {
    pub wallet: String,
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

#[derive(Debug, Serialize)]
pub struct HomeSummaryResponse {
    #[serde(flatten)]
    pub stats: UserStats,
    pub live_kombats: Vec<WagerDetailResponse>,
    pub history_kombats: Vec<WagerDetailResponse>,
    pub active_stakes: Vec<StakeWithMatch>,
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

// ═══════════════════════════════════════════════════════════════════════════════
// TOURNAMENT / MATCH BETTING (Pool Staking)
// ═══════════════════════════════════════════════════════════════════════════════

// ─── Match Record (from the configured esports data provider) ────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct MatchRecord {
    pub id: Uuid,
    pub pandascore_id: Option<i64>,
    pub slug: Option<String>,
    pub name: String,

    // Videogame info
    pub videogame_id: Option<i32>,
    pub videogame_name: Option<String>,
    pub videogame_slug: Option<String>,

    // League info
    pub league_id: Option<i32>,
    pub league_name: Option<String>,
    pub league_slug: Option<String>,
    pub league_image_url: Option<String>,

    // Series info
    pub series_id: Option<i32>,
    pub series_name: Option<String>,
    pub series_full_name: Option<String>,

    // Tournament info
    pub tournament_id: Option<i32>,
    pub tournament_name: Option<String>,
    pub tournament_slug: Option<String>,

    // Timing
    pub scheduled_at: Option<DateTime<Utc>>,
    pub begin_at: Option<DateTime<Utc>>,
    pub end_at: Option<DateTime<Utc>>,

    // Format
    pub match_type: Option<String>,
    pub number_of_games: Option<i32>,

    // Status
    pub pandascore_status: String,
    pub status: String,

    // Winner
    pub winner_id: Option<i32>,
    pub winner_type: Option<String>,
    pub forfeit: bool,

    // Streams
    pub streams_list: Option<JsonValue>,
    pub detailed_stats: bool,
    pub raw_data: Option<JsonValue>,

    // Sui pool object, populated after the Move pool is published/shared.
    pub sui_network: Option<String>,
    pub sui_pool_object_id: Option<String>,

    // Source and verification metadata.
    pub source: String,
    pub organizer_tournament_id: Option<Uuid>,
    pub organizer_wallet: Option<String>,
    pub result_status: String,
    pub rules_blob_id: Option<String>,
    pub bracket_blob_id: Option<String>,
    pub evidence_blob_id: Option<String>,
    pub evidence_summary: Option<String>,
    pub verification_status: String,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct OrganizerTournamentRecord {
    pub id: Uuid,
    pub organizer_wallet: String,
    pub name: String,
    pub videogame_name: Option<String>,
    pub videogame_slug: Option<String>,
    pub description: Option<String>,
    pub rules_blob_id: Option<String>,
    pub bracket_blob_id: Option<String>,
    pub evidence_blob_id: Option<String>,
    pub status: String,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub metadata: JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct OrganizerProfileRecord {
    pub id: Uuid,
    pub wallet_address: String,
    pub organization_name: String,
    pub contact_email: Option<String>,
    pub website_url: Option<String>,
    pub country: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub kyc_status: String,
    pub kyc_provider: Option<String>,
    pub kyc_reference_id: Option<String>,
    pub kyc_session_url: Option<String>,
    pub rejection_reason: Option<String>,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub metadata: JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── Match Opponent (Team or Player) ──────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct MatchOpponentRecord {
    pub id: Uuid,
    pub match_id: Uuid,
    pub pandascore_id: i32,
    pub opponent_type: String,
    pub name: String,
    pub acronym: Option<String>,
    pub image_url: Option<String>,
    pub location: Option<String>,
    pub position: i16,
    pub is_winner: Option<bool>,
    pub created_at: DateTime<Utc>,
}

// ─── Pool Stake Record ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct PoolStakeRecord {
    pub id: Uuid,
    pub match_id: Uuid,
    pub opponent_id: Uuid,
    pub user_wallet: String,
    pub amount_usdc: i64,
    pub odds_at_stake: Option<rust_decimal::Decimal>,
    pub status: String,
    pub payout_usdc: Option<i64>,
    pub stake_tx_hash: Option<String>,
    pub payout_tx_hash: Option<String>,
    pub stake_receipt_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

// ─── Match with Opponents and Pool Stats ──────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct MatchWithOdds {
    #[serde(flatten)]
    pub match_info: MatchRecord,
    pub pool_configured: bool,
    pub pool_object_id: Option<String>,
    pub sui_pool_object_id: Option<String>,
    pub opponents: Vec<OpponentWithPool>,
    pub total_pool_usdc: i64,
    pub total_stakers: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpponentWithPool {
    #[serde(flatten)]
    pub opponent: MatchOpponentRecord,
    pub pool_usdc: i64,
    pub pool_percentage: f64,
    pub odds: f64,
    pub staker_count: i64,
}

/// Flat row for JOIN query (used internally by db service)
#[derive(Debug, sqlx::FromRow)]
pub struct OpponentWithPoolRow {
    // MatchOpponentRecord fields
    pub id: Uuid,
    pub match_id: Uuid,
    pub pandascore_id: i32,
    pub opponent_type: String,
    pub name: String,
    pub acronym: Option<String>,
    pub image_url: Option<String>,
    pub location: Option<String>,
    pub position: i16,
    pub is_winner: Option<bool>,
    pub created_at: DateTime<Utc>,
    // Aggregate fields
    pub pool_usdc: i64,
    pub staker_count: i64,
    pub total_pool: i64,
}

impl OpponentWithPoolRow {
    pub fn into_opponent_record(self) -> MatchOpponentRecord {
        MatchOpponentRecord {
            id: self.id,
            match_id: self.match_id,
            pandascore_id: self.pandascore_id,
            opponent_type: self.opponent_type,
            name: self.name,
            acronym: self.acronym,
            image_url: self.image_url,
            location: self.location,
            position: self.position,
            is_winner: self.is_winner,
            created_at: self.created_at,
        }
    }
}

// ─── API Request Types ───────────────────────────────────────────────────────

/// Create/sync a match from provider-shaped data for admin backfills.
#[derive(Debug, Deserialize)]
pub struct CreateMatchRequest {
    #[serde(alias = "provider_id", alias = "external_id")]
    pub pandascore_id: i64,
    pub slug: Option<String>,
    pub name: String,

    // Videogame
    pub videogame_id: Option<i32>,
    pub videogame_name: Option<String>,
    pub videogame_slug: Option<String>,

    // League
    pub league_id: Option<i32>,
    pub league_name: Option<String>,
    pub league_slug: Option<String>,
    pub league_image_url: Option<String>,

    // Series
    pub series_id: Option<i32>,
    pub series_name: Option<String>,
    pub series_full_name: Option<String>,

    // Tournament
    pub tournament_id: Option<i32>,
    pub tournament_name: Option<String>,
    pub tournament_slug: Option<String>,

    // Timing
    pub scheduled_at: Option<String>,
    pub begin_at: Option<String>,
    pub end_at: Option<String>,

    // Format
    pub match_type: Option<String>,
    pub number_of_games: Option<i32>,

    // Status
    #[serde(alias = "provider_status")]
    pub pandascore_status: Option<String>,

    // Opponents (2 required for betting)
    pub opponents: Vec<CreateOpponentRequest>,

    // Streams
    pub streams_list: Option<JsonValue>,

    // Full raw data
    pub raw_data: Option<JsonValue>,

    // On-chain pool metadata, populated by the indexer after create_pool.
    pub sui_network: Option<String>,
    pub sui_pool_object_id: Option<String>,

    // Data provider. New provider syncs should set this to `grid`.
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateOpponentRequest {
    #[serde(alias = "provider_id", alias = "external_id")]
    pub pandascore_id: i32,
    pub opponent_type: String, // "Team" or "Player"
    pub name: String,
    pub acronym: Option<String>,
    pub image_url: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConfigureMatchPoolRequest {
    pub sui_network: Option<String>,
    pub sui_pool_object_id: String,
}

#[derive(Debug, Deserialize)]
pub struct BackfillMatchPoolsRequest {
    pub sui_network: Option<String>,
    pub match_ids: Option<Vec<Uuid>>,
    pub limit: Option<i64>,
    pub default_stake_window_hours: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct MatchPoolBackfillEntry {
    pub match_id: Uuid,
    pub match_name: String,
    pub status: String,
    pub created: bool,
    pub pool_object_id: Option<String>,
    pub tx_digest: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MatchPoolBackfillResponse {
    pub network: String,
    pub attempted: usize,
    pub created: usize,
    pub skipped: usize,
    pub failed: usize,
    pub entries: Vec<MatchPoolBackfillEntry>,
}

#[derive(Debug, Deserialize)]
pub struct SyncMatchStakesRequest {
    pub sui_network: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SyncMatchStakeEntry {
    pub tx_digest: String,
    pub receipt_id: String,
    pub owner: String,
    pub opponent_id: Option<Uuid>,
    pub outcome: u8,
    pub amount_usdc: i64,
    pub indexed: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SyncMatchStakesResponse {
    pub match_id: Uuid,
    pub network: String,
    pub pool_object_id: String,
    pub seen: usize,
    pub indexed: usize,
    pub skipped: usize,
    pub entries: Vec<SyncMatchStakeEntry>,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrganizerTournamentRequest {
    pub organizer_wallet: String,
    pub name: String,
    pub videogame_name: Option<String>,
    pub videogame_slug: Option<String>,
    pub description: Option<String>,
    pub rules_blob_id: Option<String>,
    pub bracket_blob_id: Option<String>,
    pub evidence_blob_id: Option<String>,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub metadata: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
pub struct OrganizerTournamentQuery {
    pub organizer_wallet: Option<String>,
    pub status: Option<String>,
    pub videogame: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrganizerMatchRequest {
    pub organizer_wallet: String,
    pub name: String,
    pub scheduled_at: Option<String>,
    pub begin_at: Option<String>,
    pub end_at: Option<String>,
    pub match_type: Option<String>,
    pub number_of_games: Option<i32>,
    pub rules_blob_id: Option<String>,
    pub bracket_blob_id: Option<String>,
    pub evidence_blob_id: Option<String>,
    pub streams_list: Option<JsonValue>,
    pub opponents: Vec<CreateOpponentRequest>,
    pub metadata: Option<JsonValue>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct OutcomeProposalRecord {
    pub id: Uuid,
    pub match_id: Uuid,
    pub proposed_winner_opponent_id: Option<Uuid>,
    pub proposed_winner_name: Option<String>,
    pub source: String,
    pub proposer_wallet: Option<String>,
    pub confidence: Option<rust_decimal::Decimal>,
    pub status: String,
    pub evidence_blob_id: Option<String>,
    pub evidence_url: Option<String>,
    pub evidence_summary: Option<String>,
    pub raw_data: JsonValue,
    pub created_at: DateTime<Utc>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub reviewer_wallet: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateOutcomeProposalRequest {
    pub proposer_wallet: Option<String>,
    pub proposed_winner_opponent_id: Option<String>,
    pub proposed_winner_name: Option<String>,
    pub source: Option<String>,
    pub confidence: Option<rust_decimal::Decimal>,
    pub evidence_blob_id: Option<String>,
    pub evidence_url: Option<String>,
    pub evidence_summary: Option<String>,
    pub raw_data: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewOutcomeProposalRequest {
    pub reviewer_wallet: Option<String>,
    pub decision: String,
}

#[derive(Debug, Deserialize)]
pub struct OrganizerApplyRequest {
    pub wallet_address: String,
    pub organization_name: String,
    pub contact_email: Option<String>,
    pub website_url: Option<String>,
    pub country: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
pub struct OrganizerKycSessionRequest {
    pub wallet_address: String,
    pub provider: Option<String>,
    pub return_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OrganizerKycSessionResponse {
    pub organizer: OrganizerProfileRecord,
    pub provider: String,
    pub reference_id: String,
    pub session_url: Option<String>,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct ReviewOrganizerRequest {
    pub status: String,
    pub kyc_status: Option<String>,
    pub kyc_provider: Option<String>,
    pub kyc_reference_id: Option<String>,
    pub kyc_session_url: Option<String>,
    pub rejection_reason: Option<String>,
    pub reviewed_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdminOrganizerQuery {
    pub status: Option<String>,
    pub kyc_status: Option<String>,
    pub country: Option<String>,
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AdminOutcomeProposalQuery {
    pub status: Option<String>,
    pub source: Option<String>,
    pub match_id: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ─── Walrus Artifacts / Agents ───────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WalrusArtifactRecord {
    pub id: Uuid,
    pub blob_id: String,
    pub object_id: Option<String>,
    pub artifact_type: String,
    pub owner_wallet: Option<String>,
    pub match_id: Option<Uuid>,
    pub outcome_proposal_id: Option<Uuid>,
    pub content_type: String,
    pub size_bytes: i64,
    pub aggregator_url: Option<String>,
    pub publisher_url: Option<String>,
    pub metadata: JsonValue,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWalrusArtifactRequest {
    pub artifact_type: String,
    pub owner_wallet: Option<String>,
    pub match_id: Option<String>,
    pub outcome_proposal_id: Option<String>,
    pub content_type: Option<String>,
    pub manifest: JsonValue,
    pub metadata: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
pub struct WalrusArtifactResponse {
    #[serde(flatten)]
    pub artifact: WalrusArtifactRecord,
    pub blob_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WalrusConfigResponse {
    pub enabled: bool,
    pub configured: bool,
    pub network: String,
    pub aggregator_url: Option<String>,
    pub max_upload_bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct WalrusBlobUrlResponse {
    pub blob_id: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct AgentRunRecord {
    pub id: Uuid,
    pub match_id: Option<Uuid>,
    pub agent_name: String,
    pub agent_id: Option<String>,
    pub status: String,
    pub watch_sources: JsonValue,
    pub evidence_blob_id: Option<String>,
    pub evidence_url: Option<String>,
    pub outcome_proposal_id: Option<Uuid>,
    pub proposed_winner_opponent_id: Option<Uuid>,
    pub proposed_winner_name: Option<String>,
    pub confidence: Option<rust_decimal::Decimal>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub verification_status: Option<String>,
    pub verification_note: Option<String>,
    pub raw_output: JsonValue,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AgentOutcomeProposalRequest {
    pub match_id: String,
    pub agent_name: Option<String>,
    /// Stable identifier for the submitting agent. When omitted, the id is
    /// resolved from the authenticating agent token.
    pub agent_id: Option<String>,
    pub watch_sources: Option<JsonValue>,
    pub proposed_winner_opponent_id: Option<String>,
    pub proposed_winner_name: Option<String>,
    pub confidence: Option<rust_decimal::Decimal>,
    #[allow(dead_code)]
    pub evidence_blob_id: Option<String>,
    pub evidence_url: Option<String>,
    pub evidence_summary: Option<String>,
    pub raw_output: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
pub struct AdminAgentRunQuery {
    pub status: Option<String>,
    pub agent_name: Option<String>,
    pub agent_id: Option<String>,
    pub match_id: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Place a stake on a match outcome
#[derive(Debug, Deserialize)]
pub struct PlaceStakeRequest {
    pub user_wallet: String,
    pub opponent_id: String, // UUID of the opponent to stake on
    pub amount_usdc: i64,    // micro-USDC (6 decimals)
}

/// Calculate potential payout (preview)
#[derive(Debug, Deserialize)]
pub struct CalculatePayoutRequest {
    pub opponent_id: String,
    pub amount_usdc: i64,
}

#[derive(Debug, Serialize)]
pub struct PayoutCalculation {
    pub stake_amount_usdc: i64,
    pub current_odds: f64,
    pub min_payout_usdc: i64,
    pub min_profit_usdc: i64,
    pub profit_percentage: f64,
    pub warning: Option<String>,
}

/// Resolve a match (admin or automated)
#[derive(Debug, Deserialize)]
pub struct ResolveMatchRequest {
    pub winner_opponent_id: String, // UUID of the winning opponent
    #[serde(alias = "pandascore_winner_id")]
    pub provider_winner_id: Option<i32>,
    pub forfeit: Option<bool>,
}

/// Individual payout/refund entry returned by resolve/cancel for on-chain settlement
#[derive(Debug, Clone)]
pub struct PayoutEntry {
    pub stake_id: Uuid,
    pub user_wallet: String,
    pub amount_usdc: i64, // payout or refund amount in micro-USDC
}

/// Result of resolve_match: what happened and what needs to be paid out
#[derive(Debug)]
pub enum ResolveResult {
    /// No stakes existed — nothing to pay out
    Empty,
    /// One-sided pool — everyone gets refunded
    Refunded(Vec<PayoutEntry>),
    /// Normal resolution — winners get payouts
    Resolved(Vec<PayoutEntry>),
}

// ─── Query Parameters ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MatchListQuery {
    pub status: Option<String>,    // defaults to current feed: upcoming + live
    pub videogame: Option<String>, // Filter by videogame slug
    pub league_id: Option<i32>,
    pub tournament_id: Option<i32>,
    pub tournament_slug: Option<String>,
    pub source: Option<String>, // defaults to grid; use all for legacy/admin views
    pub pool_configured: Option<bool>,
    pub search: Option<String>, // Search in name
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct GridSyncRequest {
    pub statuses: Option<Vec<String>>,
    pub videogame_slugs: Option<Vec<String>>,
    pub tournament_id: Option<String>,
    pub tournament_slug: Option<String>,
    pub graphql_query: Option<String>,
    pub max_pages: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct GridSyncResponse {
    pub provider: String,
    pub fetched: usize,
    pub synced: usize,
    pub synced_incomplete: usize,
    pub skipped: usize,
    pub resolved: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GridSourceResponse {
    pub provider: String,
    pub enabled: bool,
    pub configured: bool,
    pub base_url: String,
    pub matches_path: String,
    pub auth_header: String,
    pub api_style: String,
    pub default_statuses: Vec<String>,
    pub default_videogame_slugs: Vec<String>,
    pub default_per_page: u32,
    pub default_max_pages: u32,
}

#[derive(Debug, Serialize)]
pub struct GridProbeResponse {
    pub provider: String,
    pub url: String,
    pub http_status: u16,
    pub success: bool,
    pub item_count: usize,
    pub parsed_count: usize,
    pub body_preview: String,
}

#[derive(Debug, Deserialize)]
pub struct PandascoreSyncRequest {
    pub statuses: Option<Vec<String>>,
    pub videogame_slugs: Option<Vec<String>>,
    pub tournament_id: Option<String>,
    pub league_id: Option<String>,
    pub serie_id: Option<String>,
    pub sort: Option<String>,
    pub per_page: Option<u32>,
    pub max_pages: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct PandascoreSyncResponse {
    pub provider: String,
    pub fetched: usize,
    pub synced: usize,
    pub synced_incomplete: usize,
    pub skipped: usize,
    pub resolved: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PandascoreSourceResponse {
    pub provider: String,
    pub enabled: bool,
    pub configured: bool,
    pub base_url: String,
    pub default_statuses: Vec<String>,
    pub default_videogame_slugs: Vec<String>,
    pub default_per_page: u32,
    pub default_max_pages: u32,
}

#[derive(Debug, Serialize)]
pub struct PandascoreProbeResponse {
    pub provider: String,
    pub url: String,
    pub http_status: u16,
    pub success: bool,
    pub item_count: usize,
    pub parsed_count: usize,
    pub body_preview: String,
}

#[derive(Debug, Deserialize)]
pub struct StakeListQuery {
    pub status: Option<String>, // active, won, lost, refunded
    pub match_id: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct WalletDashboardQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ─── Wallet Dashboard ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WalletDashboardResponse {
    pub network: String,
    pub wallet: String,
    pub usdc_coin_type: Option<String>,
    pub available_balance_usdc: i64,
    pub locked_in_kombats_usdc: i64,
    pub total_balance_usdc: i64,
    pub transaction_history: Vec<WalletTransactionItem>,
    pub actions: WalletActionConfig,
}

#[derive(Debug, Serialize)]
pub struct WalletActionConfig {
    pub fund_wallet: WalletAction,
    pub withdraw: WalletAction,
}

#[derive(Debug, Serialize)]
pub struct WalletAction {
    pub enabled: bool,
    pub provider: String,
    pub requires_frontend_wallet: bool,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct WalletTransactionItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub amount_usdc: i64,
    pub direction: String,
    pub status: String,
    pub tx_hash: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

// ─── Transak On-Ramp Fallback ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TransakWidgetRequest {
    pub wallet_address: String,
    pub product: Option<String>,       // BUY
    pub fiat_currency: Option<String>, // default USD
    pub fiat_amount: Option<rust_decimal::Decimal>,
    pub crypto_currency_code: Option<String>, // default USDC
    pub crypto_amount: Option<rust_decimal::Decimal>,
    pub network: Option<String>, // default sui
    pub payment_method: Option<String>,
    pub email: Option<String>,
    pub partner_order_id: Option<String>,
    pub partner_customer_id: Option<String>,
    pub redirect_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TransakWidgetResponse {
    pub provider: String,
    pub product: String,
    pub wallet_address: String,
    pub widget_url: String,
}

#[derive(Debug, Deserialize)]
pub struct TransakQuoteRequest {
    pub product: Option<String>,       // BUY
    pub fiat_currency: Option<String>, // default USD
    pub fiat_amount: Option<rust_decimal::Decimal>,
    pub crypto_currency_code: Option<String>, // default USDC
    pub crypto_amount: Option<rust_decimal::Decimal>,
    pub network: Option<String>, // default sui
    pub payment_method: Option<String>,
    pub country_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TransakQuoteResponse {
    pub raw: serde_json::Value,
}

// ─── Generic Ramp Provider Layer ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RampProviderQuery {
    pub country: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RampProvidersResponse {
    pub primary_provider: String,
    pub default_network: String,
    pub default_crypto_currency: String,
    pub default_fiat_currency: String,
    pub partner_fee_bps: u16,
    pub country: Option<String>,
    pub providers: Vec<crate::services::ramp::RampProvider>,
}

#[derive(Debug, Deserialize)]
pub struct RampSessionRequest {
    pub wallet_address: String,
    pub product: Option<String>,       // BUY
    pub fiat_currency: Option<String>, // default USD
    pub fiat_amount: Option<rust_decimal::Decimal>,
    pub crypto_currency_code: Option<String>, // default USDC
    pub crypto_amount: Option<rust_decimal::Decimal>,
    pub network: Option<String>, // default sui
}

#[derive(Debug, Serialize)]
pub struct RampSessionResponse {
    pub provider: String,
    pub product: String,
    pub wallet_address: String,
    pub launch_method: String,
    pub client_action: String,
    pub network: String,
    pub crypto_currency_code: String,
    pub fiat_currency: String,
    pub fiat_amount: Option<rust_decimal::Decimal>,
    pub crypto_amount: Option<rust_decimal::Decimal>,
    pub note: String,
}

// ─── Programmable Payment Intents ────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct PaymentIntentRecord {
    pub id: Uuid,
    pub user_wallet: String,
    pub kind: String,
    pub status: String,
    pub network: String,
    pub match_id: Uuid,
    pub opponent_id: Uuid,
    pub amount_usdc: i64,
    pub reserve_balance_usdc: i64,
    pub settlement_rule: String,
    pub current_balance_usdc: Option<i64>,
    pub funding_shortfall_usdc: i64,
    pub stake_tx_hash: Option<String>,
    pub stake_receipt_id: Option<String>,
    pub metadata: JsonValue,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePaymentIntentRequest {
    pub wallet_address: String,
    pub match_id: String,
    pub opponent_id: String,
    pub amount_usdc: i64,
    pub network: Option<String>,
    pub reserve_balance_usdc: Option<i64>,
    pub settlement_rule: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PaymentIntentFunding {
    pub current_balance_usdc: i64,
    pub required_balance_usdc: i64,
    pub funding_shortfall_usdc: i64,
    pub onramp_required: bool,
}

#[derive(Debug, Serialize)]
pub struct PaymentIntentRule {
    pub rule_type: String,
    pub amount_usdc: i64,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct PaymentIntentResponse {
    pub intent: PaymentIntentRecord,
    pub funding: PaymentIntentFunding,
    pub rules: Vec<PaymentIntentRule>,
    pub match_name: String,
    pub opponent_name: String,
    pub pool_configured: bool,
    pub pool_object_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PaymentIntentOnrampResponse {
    pub intent: PaymentIntentResponse,
    pub onramp_required: bool,
    pub ramp_session: Option<RampSessionResponse>,
}

#[derive(Debug, Serialize)]
pub struct PaymentPtbStep {
    pub kind: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct PaymentPtbArgument {
    pub name: String,
    pub kind: String,
    pub value: Option<JsonValue>,
    pub source: String,
}

#[derive(Debug, Serialize)]
pub struct PaymentMoveCall {
    pub target: String,
    pub package_id: String,
    pub module: String,
    pub function: String,
    pub type_arguments: Vec<String>,
    pub arguments: Vec<PaymentPtbArgument>,
}

#[derive(Debug, Serialize)]
pub struct PaymentIntentPtbResponse {
    pub intent_id: Uuid,
    pub network: String,
    pub can_build: bool,
    pub reason: Option<String>,
    pub coin_type: Option<String>,
    pub pool_configured: bool,
    pub pool_object_id: Option<String>,
    pub amount_usdc: i64,
    pub reserve_balance_usdc: i64,
    pub expected_receipt_type: String,
    pub steps: Vec<PaymentPtbStep>,
    pub move_call: Option<PaymentMoveCall>,
}

// ─── Stake Receipt Secondary Market ──────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ReceiptMarketListingRecord {
    pub id: Uuid,
    pub network: String,
    pub seller_wallet: String,
    pub buyer_wallet: Option<String>,
    pub receipt_id: String,
    pub listing_object_id: Option<String>,
    pub match_id: Uuid,
    pub opponent_id: Uuid,
    pub ask_amount_usdc: i64,
    pub status: String,
    pub listing_tx_hash: Option<String>,
    pub sale_tx_hash: Option<String>,
    pub metadata: JsonValue,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReceiptListingRequest {
    pub wallet_address: String,
    pub receipt_id: String,
    pub match_id: String,
    pub opponent_id: String,
    pub ask_amount_usdc: i64,
    pub network: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ReceiptListingQuery {
    pub match_id: Option<String>,
    pub seller_wallet: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ActivateReceiptListingRequest {
    pub wallet_address: String,
    pub listing_object_id: String,
    pub listing_tx_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MarkReceiptListingSoldRequest {
    pub buyer_wallet: String,
    pub sale_tx_hash: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReceiptListingResponse {
    pub listing: ReceiptMarketListingRecord,
    pub id: Uuid,
    pub network: String,
    pub wallet_address: String,
    pub seller_wallet: String,
    pub buyer_wallet: Option<String>,
    pub receipt_id: String,
    pub listing_object_id: Option<String>,
    pub match_id: Uuid,
    pub opponent_id: Uuid,
    pub ask_amount_usdc: i64,
    pub status: String,
    pub listing_tx_hash: Option<String>,
    pub sale_tx_hash: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub match_name: String,
    pub opponent_name: String,
}

#[derive(Debug, Serialize)]
pub struct ReceiptMarketPtbResponse {
    pub listing_id: Uuid,
    pub network: String,
    pub can_build: bool,
    pub reason: Option<String>,
    pub coin_type: Option<String>,
    pub expected_receipt_type: String,
    pub steps: Vec<PaymentPtbStep>,
    pub move_call: Option<PaymentMoveCall>,
}

// ─── User Stake Summary ──────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct UserStakeStats {
    pub active_stakes: i64,
    pub total_staked_usdc: i64,
    pub total_won_usdc: i64,
    pub total_lost_usdc: i64,
    pub win_count: i64,
    pub loss_count: i64,
    // Client (StakeHistoryScreen) expects these names. Kept alongside the
    // originals so existing consumers are unaffected.
    pub total_active: i64,
    pub total_won: i64,
    pub total_lost: i64,
    pub total_refunded: i64,
}

// ─── Stake with Match Info (for user's stake list) ────────────────────────────

#[derive(Debug, Serialize)]
pub struct StakeWithMatch {
    #[serde(flatten)]
    pub stake: PoolStakeRecord,
    pub match_name: String,
    pub match_status: String,
    pub opponent_name: String,
    pub opponent_image_url: Option<String>,
    pub videogame_name: Option<String>,
    pub league_name: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    /// Nested match summary with BOTH opponents (ordered by position) so the FE
    /// can render the pool card without an extra per-stake match fetch.
    #[serde(rename = "match")]
    pub match_summary: Option<StakeMatchSummary>,
}

#[derive(Debug, Serialize)]
pub struct StakeMatchSummary {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub videogame_name: Option<String>,
    pub league_name: Option<String>,
    pub opponents: Vec<StakeMatchOpponent>,
}

#[derive(Debug, Serialize, Clone)]
pub struct StakeMatchOpponent {
    pub id: Uuid,
    pub name: String,
    pub acronym: Option<String>,
    pub image_url: Option<String>,
    pub position: i16,
}

/// Flat row for JOIN query (used internally by db service)
#[derive(Debug, sqlx::FromRow)]
pub struct StakeWithMatchRow {
    // PoolStakeRecord fields
    pub id: Uuid,
    pub match_id: Uuid,
    pub opponent_id: Uuid,
    pub user_wallet: String,
    pub amount_usdc: i64,
    pub odds_at_stake: Option<rust_decimal::Decimal>,
    pub status: String,
    pub payout_usdc: Option<i64>,
    pub stake_tx_hash: Option<String>,
    pub payout_tx_hash: Option<String>,
    pub stake_receipt_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    // Joined fields
    pub match_name: String,
    pub match_status: String,
    pub opponent_name: String,
    pub opponent_image_url: Option<String>,
    pub videogame_name: Option<String>,
    pub league_name: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
}

impl StakeWithMatchRow {
    pub fn into_stake_with_match(self) -> StakeWithMatch {
        StakeWithMatch {
            stake: PoolStakeRecord {
                id: self.id,
                match_id: self.match_id,
                opponent_id: self.opponent_id,
                user_wallet: self.user_wallet,
                amount_usdc: self.amount_usdc,
                odds_at_stake: self.odds_at_stake,
                status: self.status,
                payout_usdc: self.payout_usdc,
                stake_tx_hash: self.stake_tx_hash,
                payout_tx_hash: self.payout_tx_hash,
                stake_receipt_id: self.stake_receipt_id,
                created_at: self.created_at,
                resolved_at: self.resolved_at,
            },
            match_name: self.match_name,
            match_status: self.match_status,
            opponent_name: self.opponent_name,
            opponent_image_url: self.opponent_image_url,
            videogame_name: self.videogame_name,
            league_name: self.league_name,
            scheduled_at: self.scheduled_at,
            // Populated by the query layer after fetching both opponents.
            match_summary: None,
        }
    }
}
