// app/src/models.rs
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;
use chrono::{DateTime, Utc};

// ─── API Request / Response Models ────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateWagerRequest {
    pub initiator: String,
    pub stake_usdc: u64,
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
pub struct FundWagerRequest {
    pub initiator: String,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
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

#[derive(Debug, Serialize, Deserialize)]
pub struct HomeSummaryResponse {
    #[serde(flatten)]
    pub stats: UserStats,
    pub live_kombats: Vec<WagerDetailResponse>,
    pub history_kombats: Vec<WagerDetailResponse>,
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

// ─── Match Record (from PandaScore) ───────────────────────────────────────────

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
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

// ─── Match with Opponents and Pool Stats ──────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct MatchWithOdds {
    #[serde(flatten)]
    pub match_info: MatchRecord,
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

/// Create/sync a match from PandaScore data (frontend pushes this)
#[derive(Debug, Deserialize)]
pub struct CreateMatchRequest {
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
    pub pandascore_status: Option<String>,
    
    // Opponents (2 required for betting)
    pub opponents: Vec<CreateOpponentRequest>,
    
    // Streams
    pub streams_list: Option<JsonValue>,
    
    // Full raw data
    pub raw_data: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
pub struct CreateOpponentRequest {
    pub pandascore_id: i32,
    pub opponent_type: String, // "Team" or "Player"
    pub name: String,
    pub acronym: Option<String>,
    pub image_url: Option<String>,
    pub location: Option<String>,
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
    pub pandascore_winner_id: Option<i32>,
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
    pub status: Option<String>,       // upcoming, live, completed, cancelled
    pub videogame: Option<String>,    // Filter by videogame slug
    pub league_id: Option<i32>,
    pub search: Option<String>,       // Search in name
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct StakeListQuery {
    pub status: Option<String>,       // active, won, lost, refunded
    pub match_id: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
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
    pub scheduled_at: Option<DateTime<Utc>>,
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
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    // Joined fields
    pub match_name: String,
    pub match_status: String,
    pub opponent_name: String,
    pub opponent_image_url: Option<String>,
    pub videogame_name: Option<String>,
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
                created_at: self.created_at,
                resolved_at: self.resolved_at,
            },
            match_name: self.match_name,
            match_status: self.match_status,
            opponent_name: self.opponent_name,
            opponent_image_url: self.opponent_image_url,
            videogame_name: self.videogame_name,
            scheduled_at: self.scheduled_at,
        }
    }
}
