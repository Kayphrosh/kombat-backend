use crate::models::{
    NotificationRecord, UpdateProfileRequest, UserRecord, WagerListQuery, WagerRecord,
};
use anyhow::Result;
use serde_json::Value as JsonValue;
use sqlx::{postgres::PgPoolOptions, PgPool};

#[derive(sqlx::FromRow)]
struct WagerWithUsersRow {
    pub id: uuid::Uuid,
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
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub winner: Option<String>,
    pub protocol_fee_bps: i16,
    pub oracle_feed: Option<String>,
    pub oracle_target: Option<i64>,
    pub dispute_opened_at: Option<chrono::DateTime<chrono::Utc>>,
    pub dispute_opener: Option<String>,
    pub initiator_option: Option<String>,
    pub creator_declared_winner: Option<String>,
    pub challenger_declared_winner: Option<String>,
    pub resolution_error: Option<String>,
    pub resolution_attempted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub initiator_name: Option<String>,
    pub initiator_avatar: Option<String>,
    pub challenger_name: Option<String>,
    pub challenger_avatar: Option<String>,
}

fn parse_optional_rfc3339(value: Option<&String>) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn slugify(value: &str) -> String {
    let mut slug = String::with_capacity(value.len());
    let mut last_dash = false;

    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }

    slug.trim_matches('-').to_string()
}

pub enum IdempotencyStatus {
    Started,
    Completed(crate::models::TxResponse),
    InProgress,
}

pub struct DbService {
    pool: PgPool,
}

impl DbService {
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    // ── Notifications ────────────────────────────────────────────────────

    pub async fn create_notification(
        &self,
        user_wallet: &str,
        kind: &str,
        payload: Option<JsonValue>,
    ) -> Result<NotificationRecord> {
        let row = sqlx::query_as::<_, NotificationRecord>(
            r#"INSERT INTO notifications (user_wallet, kind, payload)
               VALUES ($1, $2, $3)
               RETURNING id, user_wallet, kind, payload, is_read, created_at"#,
        )
        .bind(user_wallet)
        .bind(kind)
        .bind(payload)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_notifications_for_user(
        &self,
        wallet: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<NotificationRecord>> {
        let rows = sqlx::query_as::<_, NotificationRecord>(
            "SELECT id, user_wallet, kind, payload, is_read, created_at FROM notifications WHERE user_wallet = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(wallet)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn mark_notification_read_for_user(&self, id: &str, wallet: &str) -> Result<bool> {
        let uuid = uuid::Uuid::parse_str(id)
            .map_err(|e| anyhow::anyhow!("Invalid notification ID: {}", e))?;
        let result = sqlx::query(
            "UPDATE notifications SET is_read = TRUE WHERE id = $1 AND user_wallet = $2",
        )
        .bind(uuid)
        .bind(wallet)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ── Idempotency ─────────────────────────────────────────────────────────

    pub async fn begin_idempotent_request(
        &self,
        scope: &str,
        wallet: &str,
        request_hash: &str,
    ) -> Result<IdempotencyStatus> {
        sqlx::query(
            r#"DELETE FROM idempotency_keys
               WHERE scope = $1 AND wallet = $2 AND request_hash = $3
                 AND created_at < NOW() - INTERVAL '15 minutes'"#,
        )
        .bind(scope)
        .bind(wallet)
        .bind(request_hash)
        .execute(&self.pool)
        .await?;

        let inserted: Option<uuid::Uuid> = sqlx::query_scalar(
            r#"INSERT INTO idempotency_keys (scope, wallet, request_hash)
               VALUES ($1, $2, $3)
               ON CONFLICT (scope, wallet, request_hash) DO NOTHING
               RETURNING id"#,
        )
        .bind(scope)
        .bind(wallet)
        .bind(request_hash)
        .fetch_optional(&self.pool)
        .await?;

        if inserted.is_some() {
            return Ok(IdempotencyStatus::Started);
        }

        let row: Option<(Option<JsonValue>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            r#"SELECT response_json, created_at
               FROM idempotency_keys
               WHERE scope = $1 AND wallet = $2 AND request_hash = $3"#,
        )
        .bind(scope)
        .bind(wallet)
        .bind(request_hash)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((Some(response_json), _)) => {
                let response: crate::models::TxResponse = serde_json::from_value(response_json)?;
                Ok(IdempotencyStatus::Completed(response))
            }
            Some((None, created_at)) => {
                if created_at < chrono::Utc::now() - chrono::Duration::minutes(2) {
                    sqlx::query(
                        r#"DELETE FROM idempotency_keys
                           WHERE scope = $1 AND wallet = $2 AND request_hash = $3"#,
                    )
                    .bind(scope)
                    .bind(wallet)
                    .bind(request_hash)
                    .execute(&self.pool)
                    .await?;

                    let inserted: Option<uuid::Uuid> = sqlx::query_scalar(
                        r#"INSERT INTO idempotency_keys (scope, wallet, request_hash)
                           VALUES ($1, $2, $3)
                           ON CONFLICT (scope, wallet, request_hash) DO NOTHING
                           RETURNING id"#,
                    )
                    .bind(scope)
                    .bind(wallet)
                    .bind(request_hash)
                    .fetch_optional(&self.pool)
                    .await?;

                    if inserted.is_some() {
                        Ok(IdempotencyStatus::Started)
                    } else {
                        Ok(IdempotencyStatus::InProgress)
                    }
                } else {
                    Ok(IdempotencyStatus::InProgress)
                }
            }
            None => Ok(IdempotencyStatus::Started),
        }
    }

    pub async fn complete_idempotent_request(
        &self,
        scope: &str,
        wallet: &str,
        request_hash: &str,
        response: &crate::models::TxResponse,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE idempotency_keys
               SET response_json = $4
               WHERE scope = $1 AND wallet = $2 AND request_hash = $3"#,
        )
        .bind(scope)
        .bind(wallet)
        .bind(request_hash)
        .bind(serde_json::to_value(response)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn fail_idempotent_request(
        &self,
        scope: &str,
        wallet: &str,
        request_hash: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"DELETE FROM idempotency_keys
               WHERE scope = $1 AND wallet = $2 AND request_hash = $3"#,
        )
        .bind(scope)
        .bind(wallet)
        .bind(request_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Wager CRUD ────────────────────────────────────────────────────────────

    pub async fn upsert_wager(&self, w: &WagerRecord) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO wagers (
                id, on_chain_address, wager_id, initiator, challenger,
                stake_usdc, description, status, resolution_source,
                resolver, expiry_ts, created_at, resolved_at, winner,
                protocol_fee_bps, oracle_feed, oracle_target,
                dispute_opened_at, dispute_opener, initiator_option,
                creator_declared_winner, challenger_declared_winner,
                resolution_error, resolution_attempted_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                $21, $22, $23, $24
            )
            ON CONFLICT (on_chain_address) DO UPDATE SET
                challenger       = EXCLUDED.challenger,
                status           = EXCLUDED.status,
                resolved_at      = EXCLUDED.resolved_at,
                winner           = EXCLUDED.winner,
                dispute_opened_at = EXCLUDED.dispute_opened_at,
                dispute_opener   = EXCLUDED.dispute_opener,
                resolution_error = EXCLUDED.resolution_error,
                resolution_attempted_at = EXCLUDED.resolution_attempted_at
            "#,
        )
        .bind(&w.id)
        .bind(&w.on_chain_address)
        .bind(w.wager_id)
        .bind(&w.initiator)
        .bind(&w.challenger)
        .bind(w.stake_usdc)
        .bind(&w.description)
        .bind(&w.status)
        .bind(&w.resolution_source)
        .bind(&w.resolver)
        .bind(w.expiry_ts)
        .bind(w.created_at)
        .bind(w.resolved_at)
        .bind(&w.winner)
        .bind(w.protocol_fee_bps as i16)
        .bind(&w.oracle_feed)
        .bind(w.oracle_target)
        .bind(w.dispute_opened_at)
        .bind(&w.dispute_opener)
        .bind(&w.initiator_option)
        .bind(&w.creator_declared_winner)
        .bind(&w.challenger_declared_winner)
        .bind(&w.resolution_error)
        .bind(w.resolution_attempted_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_wager_by_address(&self, address: &str) -> Result<Option<WagerRecord>> {
        let row = sqlx::query_as::<_, WagerRecord>(
            r#"SELECT
                id, on_chain_address, wager_id, initiator, challenger,
                stake_usdc, description, status, resolution_source,
                resolver, expiry_ts, created_at, resolved_at, winner,
                protocol_fee_bps, oracle_feed, oracle_target,
                dispute_opened_at, dispute_opener, initiator_option,
                creator_declared_winner, challenger_declared_winner,
                resolution_error, resolution_attempted_at
              FROM wagers WHERE on_chain_address = $1"#,
        )
        .bind(address)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Get a wager with participant user info (display names, avatars)
    pub async fn get_wager_with_users(
        &self,
        address: &str,
    ) -> Result<Option<crate::models::WagerDetailResponse>> {
        let rows = sqlx::query_as::<_, WagerWithUsersRow>(
            r#"SELECT
                w.id, w.on_chain_address, w.wager_id, w.initiator, w.challenger,
                w.stake_usdc, w.description, w.status, w.resolution_source,
                w.resolver, w.expiry_ts, w.created_at, w.resolved_at, w.winner,
                w.protocol_fee_bps, w.oracle_feed, w.oracle_target,
                w.dispute_opened_at, w.dispute_opener, w.initiator_option,
                w.creator_declared_winner, w.challenger_declared_winner,
                w.resolution_error, w.resolution_attempted_at,
                ui.display_name AS initiator_name,
                ui.avatar_url AS initiator_avatar,
                uc.display_name AS challenger_name,
                uc.avatar_url AS challenger_avatar
               FROM wagers w
               LEFT JOIN users ui ON ui.wallet_address = w.initiator
               LEFT JOIN users uc ON uc.wallet_address = w.challenger
               WHERE w.on_chain_address = $1"#,
        )
        .bind(address)
        .fetch_optional(&self.pool)
        .await?;

        Ok(rows.map(|row| Self::enrich_wager_row(row, None)))
    }

    pub async fn list_wagers(&self, q: &WagerListQuery) -> Result<Vec<WagerRecord>> {
        let limit = q.limit.unwrap_or(20).min(100);
        let offset = q.offset.unwrap_or(0);

        let mut qb = sqlx::QueryBuilder::new(
            r#"SELECT id, on_chain_address, wager_id, initiator, challenger,
               stake_usdc, description, status, resolution_source,
               resolver, expiry_ts, created_at, resolved_at, winner,
               protocol_fee_bps, oracle_feed, oracle_target,
               dispute_opened_at, dispute_opener, initiator_option,
               creator_declared_winner, challenger_declared_winner,
               resolution_error, resolution_attempted_at
               FROM wagers WHERE 1=1"#,
        );

        if let Some(ref ini) = q.initiator {
            qb.push(" AND initiator = ").push_bind(ini.clone());
        }
        if let Some(ref ch) = q.challenger {
            qb.push(" AND challenger = ").push_bind(ch.clone());
        }
        if let Some(ref st) = q.status {
            qb.push(" AND status = ").push_bind(st.clone());
        }

        qb.push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        let rows = qb
            .build_query_as::<WagerRecord>()
            .fetch_all(&self.pool)
            .await?;

        Ok(rows)
    }

    pub async fn list_wagers_enriched(
        &self,
        q: &WagerListQuery,
        context_wallet: Option<&str>,
    ) -> Result<Vec<crate::models::WagerDetailResponse>> {
        let limit = q.limit.unwrap_or(20).min(100);
        let offset = q.offset.unwrap_or(0);

        match (&q.initiator, &q.challenger, &q.status) {
            (Some(initiator), None, status) => {
                self.fetch_wagers_with_users(
                    "w.initiator = $1 AND ($2::text IS NULL OR w.status = $2)",
                    context_wallet,
                    Some(initiator),
                    status.as_deref(),
                    limit,
                    offset,
                )
                .await
            }
            (None, Some(challenger), status) => {
                self.fetch_wagers_with_users(
                    "w.challenger = $1 AND ($2::text IS NULL OR w.status = $2)",
                    context_wallet,
                    Some(challenger),
                    status.as_deref(),
                    limit,
                    offset,
                )
                .await
            }
            (Some(initiator), Some(challenger), status) => {
                let rows = sqlx::query_as::<_, WagerWithUsersRow>(
                    r#"SELECT
                        w.id, w.on_chain_address, w.wager_id, w.initiator, w.challenger,
                        w.stake_usdc, w.description, w.status, w.resolution_source,
                        w.resolver, w.expiry_ts, w.created_at, w.resolved_at, w.winner,
                        w.protocol_fee_bps, w.oracle_feed, w.oracle_target,
                        w.dispute_opened_at, w.dispute_opener, w.initiator_option,
                        w.creator_declared_winner, w.challenger_declared_winner,
                        w.resolution_error, w.resolution_attempted_at,
                        ui.display_name AS initiator_name,
                        ui.avatar_url AS initiator_avatar,
                        uc.display_name AS challenger_name,
                        uc.avatar_url AS challenger_avatar
                       FROM wagers w
                       LEFT JOIN users ui ON ui.wallet_address = w.initiator
                       LEFT JOIN users uc ON uc.wallet_address = w.challenger
                       WHERE w.initiator = $1
                         AND w.challenger = $2
                         AND ($3::text IS NULL OR w.status = $3)
                       ORDER BY w.created_at DESC
                       LIMIT $4 OFFSET $5"#,
                )
                .bind(initiator)
                .bind(challenger)
                .bind(status.as_deref())
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?;

                Ok(rows
                    .into_iter()
                    .map(|row| Self::enrich_wager_row(row, context_wallet))
                    .collect())
            }
            (None, None, status) => {
                let rows = sqlx::query_as::<_, WagerWithUsersRow>(
                    r#"SELECT
                        w.id, w.on_chain_address, w.wager_id, w.initiator, w.challenger,
                        w.stake_usdc, w.description, w.status, w.resolution_source,
                        w.resolver, w.expiry_ts, w.created_at, w.resolved_at, w.winner,
                        w.protocol_fee_bps, w.oracle_feed, w.oracle_target,
                        w.dispute_opened_at, w.dispute_opener, w.initiator_option,
                        w.creator_declared_winner, w.challenger_declared_winner,
                        w.resolution_error, w.resolution_attempted_at,
                        ui.display_name AS initiator_name,
                        ui.avatar_url AS initiator_avatar,
                        uc.display_name AS challenger_name,
                        uc.avatar_url AS challenger_avatar
                       FROM wagers w
                       LEFT JOIN users ui ON ui.wallet_address = w.initiator
                       LEFT JOIN users uc ON uc.wallet_address = w.challenger
                       WHERE ($1::text IS NULL OR w.status = $1)
                       ORDER BY w.created_at DESC
                       LIMIT $2 OFFSET $3"#,
                )
                .bind(status.as_deref())
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?;

                Ok(rows
                    .into_iter()
                    .map(|row| Self::enrich_wager_row(row, context_wallet))
                    .collect())
            }
        }
    }

    pub async fn list_my_wagers(
        &self,
        wallet: &str,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<crate::models::WagerDetailResponse>> {
        self.fetch_wagers_with_users(
            "(w.initiator = $1 OR w.challenger = $1) AND ($2::text IS NULL OR w.status = $2)",
            Some(wallet),
            Some(wallet),
            None,
            limit.unwrap_or(100).min(200),
            offset.unwrap_or(0),
        )
        .await
    }

    pub async fn update_wager_status(&self, address: &str, status: &str) -> Result<()> {
        sqlx::query("UPDATE wagers SET status = $1 WHERE on_chain_address = $2")
            .bind(status)
            .bind(address)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_wager_resolution_attempt(
        &self,
        address: &str,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE wagers SET resolution_error = $1, resolution_attempted_at = NOW() WHERE on_chain_address = $2",
        )
        .bind(error)
        .bind(address)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Store which winner a participant declared and auto-resolve if both agree.
    pub async fn set_declared_winner(
        &self,
        address: &str,
        is_initiator: bool,
        declared_winner: &str,
    ) -> Result<Option<String>> {
        // Update the appropriate column
        let col = if is_initiator {
            "creator_declared_winner"
        } else {
            "challenger_declared_winner"
        };

        let query = format!("UPDATE wagers SET {} = $1 WHERE on_chain_address = $2", col);
        sqlx::query(&query)
            .bind(declared_winner)
            .bind(address)
            .execute(&self.pool)
            .await?;

        // Check if both sides now agree on the same winner
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT creator_declared_winner, challenger_declared_winner FROM wagers WHERE on_chain_address = $1"
        )
        .bind(address)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((Some(creator_pick), Some(challenger_pick))) = row {
            if creator_pick == challenger_pick {
                // Both agree — mark as resolved
                sqlx::query(
                    "UPDATE wagers SET status = 'resolved', winner = $1, resolved_at = NOW(), resolution_error = NULL WHERE on_chain_address = $2"
                )
                .bind(&creator_pick)
                .bind(address)
                .execute(&self.pool)
                .await?;

                // Update win/loss stats for both participants
                if let Some(wager) = self.get_wager_by_address(address).await? {
                    let loser = if wager.initiator == creator_pick {
                        wager.challenger.as_deref()
                    } else {
                        Some(wager.initiator.as_str())
                    };
                    self.record_wager_result(&creator_pick, loser).await?;
                }

                return Ok(Some(creator_pick));
            }
        }

        Ok(None)
    }

    // ── User Profile ──────────────────────────────────────────────────────────

    pub async fn get_user(&self, wallet: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRecord>(
            r#"SELECT id, wallet_address, email, display_name, avatar_url,
                      wins, losses, created_at, updated_at
               FROM users WHERE wallet_address = $1"#,
        )
        .bind(wallet)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn search_users(&self, query: &str, limit: i64) -> Result<Vec<UserRecord>> {
        let pattern = format!("%{}%", query);
        let rows = sqlx::query_as::<_, UserRecord>(
            r#"SELECT id, wallet_address, email, display_name, avatar_url,
                      wins, losses, created_at, updated_at
               FROM users
               WHERE display_name ILIKE $1 OR wallet_address ILIKE $1
               ORDER BY display_name ASC NULLS LAST
               LIMIT $2"#,
        )
        .bind(pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn upsert_user(
        &self,
        wallet: &str,
        req: &UpdateProfileRequest,
    ) -> Result<UserRecord> {
        let row = sqlx::query_as::<_, UserRecord>(
            r#"INSERT INTO users (wallet_address, email, display_name, avatar_url)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (wallet_address) DO UPDATE SET
                   email        = COALESCE(EXCLUDED.email,        users.email),
                   display_name = COALESCE(EXCLUDED.display_name, users.display_name),
                   avatar_url   = COALESCE(EXCLUDED.avatar_url,   users.avatar_url),
                   updated_at   = NOW()
               RETURNING id, wallet_address, email, display_name, avatar_url,
                         wins, losses, created_at, updated_at"#,
        )
        .bind(wallet)
        .bind(&req.email)
        .bind(&req.display_name)
        .bind(&req.avatar_url)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    // ── Delete User ──────────────────────────────────────────────────────────

    pub async fn delete_user(&self, wallet: &str) -> Result<()> {
        // Delete related data first
        sqlx::query("DELETE FROM notifications WHERE user_wallet = $1")
            .bind(wallet)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM notification_settings WHERE user_wallet = $1")
            .bind(wallet)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM users WHERE wallet_address = $1")
            .bind(wallet)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Win/Loss Recording ───────────────────────────────────────────────

    /// Increment the winner's `wins` and the loser's `losses` in the users table.
    /// If either wallet does not have a user row yet the update is silently skipped
    /// (the user simply hasn't created a profile).
    pub async fn record_wager_result(
        &self,
        winner_wallet: &str,
        loser_wallet: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE users SET wins = wins + 1, updated_at = NOW() WHERE wallet_address = $1",
        )
        .bind(winner_wallet)
        .execute(&self.pool)
        .await?;

        if let Some(loser) = loser_wallet {
            sqlx::query(
                "UPDATE users SET losses = losses + 1, updated_at = NOW() WHERE wallet_address = $1"
            )
            .bind(loser)
            .execute(&self.pool)
            .await?;
        }

        tracing::info!(
            "Recorded wager result — winner: {}, loser: {:?}",
            winner_wallet,
            loser_wallet
        );
        Ok(())
    }

    // ── Dispute Submissions ──────────────────────────────────────────────────

    /// Insert or update a dispute submission for a participant.
    /// Each (wager_address, submitter) pair can have exactly one submission.
    pub async fn upsert_dispute_submission(
        &self,
        wager_address: &str,
        submitter: &str,
        description: &str,
        evidence_url: Option<&str>,
        declared_winner: Option<&str>,
    ) -> Result<crate::models::DisputeSubmissionRecord> {
        let row = sqlx::query_as::<_, crate::models::DisputeSubmissionRecord>(
            r#"INSERT INTO dispute_submissions (wager_address, submitter, description, evidence_url, declared_winner)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (wager_address, submitter) DO UPDATE SET
                   description     = EXCLUDED.description,
                   evidence_url    = EXCLUDED.evidence_url,
                   declared_winner = EXCLUDED.declared_winner,
                   updated_at      = NOW()
               RETURNING id, wager_address, submitter, description, evidence_url, declared_winner, created_at, updated_at"#,
        )
        .bind(wager_address)
        .bind(submitter)
        .bind(description)
        .bind(evidence_url)
        .bind(declared_winner)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Fetch all dispute submissions for a wager (up to 2 — one per participant).
    pub async fn get_dispute_submissions(
        &self,
        wager_address: &str,
    ) -> Result<Vec<crate::models::DisputeSubmissionRecord>> {
        let rows = sqlx::query_as::<_, crate::models::DisputeSubmissionRecord>(
            r#"SELECT id, wager_address, submitter, description, evidence_url, declared_winner, created_at, updated_at
               FROM dispute_submissions
               WHERE wager_address = $1
               ORDER BY created_at ASC"#,
        )
        .bind(wager_address)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── User Stats ──────────────────────────────────────────────────────────

    pub async fn get_user_stats(&self, wallet: &str) -> Result<crate::models::UserStats> {
        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM wagers WHERE status = 'active' AND (initiator = $1 OR challenger = $1)"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let completed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM wagers WHERE status = 'resolved' AND (initiator = $1 OR challenger = $1)"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let total_stake: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(stake_usdc), 0)::BIGINT FROM wagers WHERE (initiator = $1 OR challenger = $1)"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let total_won: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(stake_usdc * 2), 0)::BIGINT FROM wagers WHERE status = 'resolved' AND winner = $1"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        Ok(crate::models::UserStats {
            live_count: live,
            completed_count: completed,
            total_stake,
            total_won,
        })
    }

    // ── Notification Settings ────────────────────────────────────────────────

    pub async fn get_notification_settings(
        &self,
        wallet: &str,
    ) -> Result<crate::models::NotificationSettings> {
        let row = sqlx::query_as::<_, crate::models::NotificationSettings>(
            "SELECT user_wallet, challenges, funds, disputes, marketing FROM notification_settings WHERE user_wallet = $1"
        )
        .bind(wallet)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.unwrap_or(crate::models::NotificationSettings {
            user_wallet: wallet.to_string(),
            challenges: true,
            funds: true,
            disputes: true,
            marketing: false,
        }))
    }

    pub async fn upsert_notification_settings(
        &self,
        wallet: &str,
        settings: &crate::models::UpdateNotificationSettings,
    ) -> Result<crate::models::NotificationSettings> {
        let row = sqlx::query_as::<_, crate::models::NotificationSettings>(
            r#"INSERT INTO notification_settings (user_wallet, challenges, funds, disputes, marketing)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (user_wallet) DO UPDATE SET
                   challenges = COALESCE($2, notification_settings.challenges),
                   funds      = COALESCE($3, notification_settings.funds),
                   disputes   = COALESCE($4, notification_settings.disputes),
                   marketing  = COALESCE($5, notification_settings.marketing)
               RETURNING user_wallet, challenges, funds, disputes, marketing"#
        )
        .bind(wallet)
        .bind(settings.challenges)
        .bind(settings.funds)
        .bind(settings.disputes)
        .bind(settings.marketing)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    // ── Push Tokens ──────────────────────────────────────────────────────────

    pub async fn upsert_push_token(&self, wallet: &str, token: &str) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO push_tokens (wallet_address, expo_token)
               VALUES ($1, $2)
               ON CONFLICT (wallet_address, expo_token) DO NOTHING"#,
        )
        .bind(wallet)
        .bind(token)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_push_tokens(&self, wallet: &str) -> Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT expo_token FROM push_tokens WHERE wallet_address = $1")
                .bind(wallet)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // TOURNAMENT / MATCH BETTING (Pool Staking)
    // ═══════════════════════════════════════════════════════════════════════════

    // ── Match CRUD ──────────────────────────────────────────────────────────────

    /// Create or update a match from PandaScore data
    pub async fn upsert_match(
        &self,
        req: &crate::models::CreateMatchRequest,
    ) -> Result<crate::models::MatchRecord> {
        // Parse optional timestamps
        let scheduled_at = req
            .scheduled_at
            .as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        let begin_at = req
            .begin_at
            .as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        let end_at = req
            .end_at
            .as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let pandascore_status = req
            .pandascore_status
            .clone()
            .unwrap_or_else(|| "not_started".to_string());

        // Determine our internal status based on PandaScore status
        let status = match pandascore_status.as_str() {
            "not_started" => "upcoming",
            "running" => "live",
            "finished" => "completed",
            "canceled" => "cancelled",
            "postponed" => "upcoming", // Keep as upcoming but track postponed
            _ => "upcoming",
        };

        let row = sqlx::query_as::<_, crate::models::MatchRecord>(
            r#"INSERT INTO matches (
                pandascore_id, slug, name,
                videogame_id, videogame_name, videogame_slug,
                league_id, league_name, league_slug, league_image_url,
                series_id, series_name, series_full_name,
                tournament_id, tournament_name, tournament_slug,
                scheduled_at, begin_at, end_at,
                match_type, number_of_games,
                pandascore_status, status,
                streams_list, raw_data
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19,
                $20, $21, $22, $23, $24, $25
            )
            ON CONFLICT (pandascore_id) DO UPDATE SET
                name = EXCLUDED.name,
                scheduled_at = EXCLUDED.scheduled_at,
                begin_at = EXCLUDED.begin_at,
                end_at = EXCLUDED.end_at,
                pandascore_status = EXCLUDED.pandascore_status,
                status = CASE 
                    WHEN matches.status = 'completed' THEN matches.status
                    WHEN matches.status = 'cancelled' THEN matches.status
                    ELSE EXCLUDED.status
                END,
                streams_list = EXCLUDED.streams_list,
                raw_data = EXCLUDED.raw_data,
                updated_at = NOW()
            RETURNING *"#,
        )
        .bind(req.pandascore_id)
        .bind(&req.slug)
        .bind(&req.name)
        .bind(req.videogame_id)
        .bind(&req.videogame_name)
        .bind(&req.videogame_slug)
        .bind(req.league_id)
        .bind(&req.league_name)
        .bind(&req.league_slug)
        .bind(&req.league_image_url)
        .bind(req.series_id)
        .bind(&req.series_name)
        .bind(&req.series_full_name)
        .bind(req.tournament_id)
        .bind(&req.tournament_name)
        .bind(&req.tournament_slug)
        .bind(scheduled_at)
        .bind(begin_at)
        .bind(end_at)
        .bind(&req.match_type)
        .bind(req.number_of_games)
        .bind(&pandascore_status)
        .bind(status)
        .bind(&req.streams_list)
        .bind(&req.raw_data)
        .fetch_one(&self.pool)
        .await?;

        // Upsert opponents
        for (i, opponent) in req.opponents.iter().enumerate() {
            self.upsert_match_opponent(row.id, opponent, i as i16)
                .await?;
        }

        Ok(row)
    }

    /// Upsert a match opponent
    async fn upsert_match_opponent(
        &self,
        match_id: uuid::Uuid,
        opponent: &crate::models::CreateOpponentRequest,
        position: i16,
    ) -> Result<crate::models::MatchOpponentRecord> {
        let row = sqlx::query_as::<_, crate::models::MatchOpponentRecord>(
            r#"INSERT INTO match_opponents (
                match_id, pandascore_id, opponent_type, name, acronym, image_url, location, position
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (match_id, position) DO UPDATE SET
                pandascore_id = EXCLUDED.pandascore_id,
                name = EXCLUDED.name,
                acronym = EXCLUDED.acronym,
                image_url = EXCLUDED.image_url,
                location = EXCLUDED.location
            RETURNING *"#,
        )
        .bind(match_id)
        .bind(opponent.pandascore_id)
        .bind(&opponent.opponent_type)
        .bind(&opponent.name)
        .bind(&opponent.acronym)
        .bind(&opponent.image_url)
        .bind(&opponent.location)
        .bind(position)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn create_organizer_tournament(
        &self,
        req: &crate::models::CreateOrganizerTournamentRequest,
    ) -> Result<crate::models::OrganizerTournamentRecord> {
        let starts_at = parse_optional_rfc3339(req.starts_at.as_ref());
        let ends_at = parse_optional_rfc3339(req.ends_at.as_ref());
        let metadata = req
            .metadata
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));

        let row = sqlx::query_as::<_, crate::models::OrganizerTournamentRecord>(
            r#"INSERT INTO organizer_tournaments (
                organizer_wallet, name, videogame_name, videogame_slug, description,
                rules_blob_id, bracket_blob_id, evidence_blob_id,
                status, starts_at, ends_at, metadata
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8,
                'draft', $9, $10, $11
            )
            RETURNING *"#,
        )
        .bind(&req.organizer_wallet)
        .bind(&req.name)
        .bind(&req.videogame_name)
        .bind(&req.videogame_slug)
        .bind(&req.description)
        .bind(&req.rules_blob_id)
        .bind(&req.bracket_blob_id)
        .bind(&req.evidence_blob_id)
        .bind(starts_at)
        .bind(ends_at)
        .bind(metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_organizer_tournaments(
        &self,
        query: &crate::models::OrganizerTournamentQuery,
    ) -> Result<Vec<crate::models::OrganizerTournamentRecord>> {
        let limit = query.limit.unwrap_or(20).clamp(1, 100);
        let offset = query.offset.unwrap_or(0).max(0);
        let mut qb = sqlx::QueryBuilder::new("SELECT * FROM organizer_tournaments WHERE 1=1");

        if let Some(ref wallet) = query.organizer_wallet {
            qb.push(" AND organizer_wallet = ")
                .push_bind(wallet.clone());
        }
        if let Some(ref status) = query.status {
            qb.push(" AND status = ").push_bind(status.clone());
        }
        if let Some(ref videogame) = query.videogame {
            qb.push(" AND videogame_slug = ")
                .push_bind(videogame.clone());
        }

        qb.push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        Ok(qb
            .build_query_as::<crate::models::OrganizerTournamentRecord>()
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn get_organizer_tournament(
        &self,
        id: uuid::Uuid,
    ) -> Result<Option<crate::models::OrganizerTournamentRecord>> {
        Ok(
            sqlx::query_as::<_, crate::models::OrganizerTournamentRecord>(
                "SELECT * FROM organizer_tournaments WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?,
        )
    }

    pub async fn upsert_organizer_profile(
        &self,
        req: &crate::models::OrganizerApplyRequest,
    ) -> Result<crate::models::OrganizerProfileRecord> {
        let metadata = req
            .metadata
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));

        let row = sqlx::query_as::<_, crate::models::OrganizerProfileRecord>(
            r#"INSERT INTO organizer_profiles (
                wallet_address, organization_name, contact_email, website_url,
                country, description, status, metadata
            ) VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7)
            ON CONFLICT (wallet_address) DO UPDATE SET
                organization_name = EXCLUDED.organization_name,
                contact_email = EXCLUDED.contact_email,
                website_url = EXCLUDED.website_url,
                country = EXCLUDED.country,
                description = EXCLUDED.description,
                status = CASE
                    WHEN organizer_profiles.status = 'approved' THEN organizer_profiles.status
                    ELSE 'pending'
                END,
                metadata = EXCLUDED.metadata,
                rejection_reason = NULL,
                updated_at = NOW()
            RETURNING *"#,
        )
        .bind(&req.wallet_address)
        .bind(&req.organization_name)
        .bind(&req.contact_email)
        .bind(&req.website_url)
        .bind(&req.country)
        .bind(&req.description)
        .bind(metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_organizer_profile(
        &self,
        wallet: &str,
    ) -> Result<Option<crate::models::OrganizerProfileRecord>> {
        Ok(sqlx::query_as::<_, crate::models::OrganizerProfileRecord>(
            "SELECT * FROM organizer_profiles WHERE wallet_address = $1",
        )
        .bind(wallet)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn list_admin_organizer_profiles(
        &self,
        query: &crate::models::AdminOrganizerQuery,
    ) -> Result<Vec<crate::models::OrganizerProfileRecord>> {
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        let offset = query.offset.unwrap_or(0).max(0);
        let mut qb = sqlx::QueryBuilder::new("SELECT * FROM organizer_profiles WHERE 1=1");

        if let Some(ref status) = query.status {
            qb.push(" AND status = ").push_bind(status.clone());
        }
        if let Some(ref kyc_status) = query.kyc_status {
            qb.push(" AND kyc_status = ").push_bind(kyc_status.clone());
        }
        if let Some(ref country) = query.country {
            qb.push(" AND country = ").push_bind(country.clone());
        }
        if let Some(ref search) = query.search {
            let escaped = search
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            qb.push(" AND (organization_name ILIKE ")
                .push_bind(format!("%{}%", escaped))
                .push(" OR wallet_address ILIKE ")
                .push_bind(format!("%{}%", escaped))
                .push(" OR contact_email ILIKE ")
                .push_bind(format!("%{}%", escaped))
                .push(")");
        }

        qb.push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        Ok(qb
            .build_query_as::<crate::models::OrganizerProfileRecord>()
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn create_organizer_kyc_session(
        &self,
        wallet: &str,
        provider: &str,
        reference_id: &str,
        session_url: Option<&str>,
    ) -> Result<crate::models::OrganizerProfileRecord> {
        let row = sqlx::query_as::<_, crate::models::OrganizerProfileRecord>(
            r#"UPDATE organizer_profiles
               SET kyc_status = 'pending',
                   kyc_provider = $2,
                   kyc_reference_id = $3,
                   kyc_session_url = $4,
                   updated_at = NOW()
               WHERE wallet_address = $1
               RETURNING *"#,
        )
        .bind(wallet)
        .bind(provider)
        .bind(reference_id)
        .bind(session_url)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn review_organizer_profile(
        &self,
        wallet: &str,
        req: &crate::models::ReviewOrganizerRequest,
    ) -> Result<crate::models::OrganizerProfileRecord> {
        if !matches!(
            req.status.as_str(),
            "pending" | "approved" | "rejected" | "suspended"
        ) {
            anyhow::bail!("Unsupported organizer status: {}", req.status);
        }
        if let Some(ref kyc_status) = req.kyc_status {
            if !matches!(
                kyc_status.as_str(),
                "not_started" | "pending" | "verified" | "rejected"
            ) {
                anyhow::bail!("Unsupported KYC status: {}", kyc_status);
            }
        }

        let row = sqlx::query_as::<_, crate::models::OrganizerProfileRecord>(
            r#"UPDATE organizer_profiles
               SET status = $2,
                   kyc_status = COALESCE($3, kyc_status),
                   kyc_provider = COALESCE($4, kyc_provider),
                   kyc_reference_id = COALESCE($5, kyc_reference_id),
                   kyc_session_url = COALESCE($6, kyc_session_url),
                   rejection_reason = $7,
                   reviewed_by = $8,
                   reviewed_at = NOW(),
                   updated_at = NOW()
               WHERE wallet_address = $1
               RETURNING *"#,
        )
        .bind(wallet)
        .bind(&req.status)
        .bind(&req.kyc_status)
        .bind(&req.kyc_provider)
        .bind(&req.kyc_reference_id)
        .bind(&req.kyc_session_url)
        .bind(&req.rejection_reason)
        .bind(&req.reviewed_by)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn organizer_can_create_markets(&self, wallet: &str) -> Result<bool> {
        let allowed: Option<bool> = sqlx::query_scalar(
            r#"SELECT (status = 'approved' AND kyc_status = 'verified')
               FROM organizer_profiles
               WHERE wallet_address = $1"#,
        )
        .bind(wallet)
        .fetch_optional(&self.pool)
        .await?;

        Ok(allowed.unwrap_or(false))
    }

    pub async fn create_organizer_match(
        &self,
        tournament: &crate::models::OrganizerTournamentRecord,
        req: &crate::models::CreateOrganizerMatchRequest,
    ) -> Result<crate::models::MatchRecord> {
        let scheduled_at = parse_optional_rfc3339(req.scheduled_at.as_ref());
        let begin_at = parse_optional_rfc3339(req.begin_at.as_ref());
        let end_at = parse_optional_rfc3339(req.end_at.as_ref());
        let raw_data = req
            .metadata
            .clone()
            .map(|metadata| serde_json::json!({ "organizer_metadata": metadata }));

        let row = sqlx::query_as::<_, crate::models::MatchRecord>(
            r#"INSERT INTO matches (
                slug, name,
                videogame_name, videogame_slug,
                tournament_name, tournament_slug,
                scheduled_at, begin_at, end_at,
                match_type, number_of_games,
                pandascore_status, status,
                streams_list, raw_data,
                source, organizer_tournament_id, organizer_wallet,
                result_status, rules_blob_id, bracket_blob_id, evidence_blob_id,
                verification_status
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9,
                $10, $11, 'not_started', 'upcoming',
                $12, $13, 'organizer', $14, $15,
                'pending', $16, $17, $18, 'organizer_attested'
            )
            RETURNING *"#,
        )
        .bind(slugify(&req.name))
        .bind(&req.name)
        .bind(&tournament.videogame_name)
        .bind(&tournament.videogame_slug)
        .bind(&tournament.name)
        .bind(slugify(&tournament.name))
        .bind(scheduled_at)
        .bind(begin_at)
        .bind(end_at)
        .bind(&req.match_type)
        .bind(req.number_of_games)
        .bind(&req.streams_list)
        .bind(raw_data)
        .bind(tournament.id)
        .bind(&req.organizer_wallet)
        .bind(
            req.rules_blob_id
                .as_ref()
                .or(tournament.rules_blob_id.as_ref()),
        )
        .bind(
            req.bracket_blob_id
                .as_ref()
                .or(tournament.bracket_blob_id.as_ref()),
        )
        .bind(
            req.evidence_blob_id
                .as_ref()
                .or(tournament.evidence_blob_id.as_ref()),
        )
        .fetch_one(&self.pool)
        .await?;

        for (index, opponent) in req.opponents.iter().enumerate() {
            let mut opponent = crate::models::CreateOpponentRequest {
                pandascore_id: opponent.pandascore_id,
                opponent_type: opponent.opponent_type.clone(),
                name: opponent.name.clone(),
                acronym: opponent.acronym.clone(),
                image_url: opponent.image_url.clone(),
                location: opponent.location.clone(),
            };
            if opponent.pandascore_id == 0 {
                opponent.pandascore_id = -((index as i32) + 1);
            }
            self.upsert_match_opponent(row.id, &opponent, index as i16)
                .await?;
        }

        Ok(row)
    }

    pub async fn create_outcome_proposal(
        &self,
        match_id: uuid::Uuid,
        req: &crate::models::CreateOutcomeProposalRequest,
    ) -> Result<crate::models::OutcomeProposalRecord> {
        let proposed_winner_uuid = match req.proposed_winner_opponent_id.as_ref() {
            Some(id) => Some(
                uuid::Uuid::parse_str(id)
                    .map_err(|e| anyhow::anyhow!("Invalid proposed_winner_opponent_id: {}", e))?,
            ),
            None => None,
        };
        let raw_data = req
            .raw_data
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        let source = req
            .source
            .clone()
            .unwrap_or_else(|| "organizer".to_string());

        let row = sqlx::query_as::<_, crate::models::OutcomeProposalRecord>(
            r#"INSERT INTO outcome_proposals (
                match_id, proposed_winner_opponent_id, proposed_winner_name,
                source, proposer_wallet, confidence, evidence_blob_id,
                evidence_url, evidence_summary, raw_data
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING *"#,
        )
        .bind(match_id)
        .bind(proposed_winner_uuid)
        .bind(&req.proposed_winner_name)
        .bind(source)
        .bind(&req.proposer_wallet)
        .bind(req.confidence)
        .bind(&req.evidence_blob_id)
        .bind(&req.evidence_url)
        .bind(&req.evidence_summary)
        .bind(raw_data)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query(
            r#"UPDATE matches
               SET result_status = 'proposed',
                   evidence_blob_id = COALESCE($2, evidence_blob_id),
                   evidence_summary = COALESCE($3, evidence_summary),
                   verification_status = $4,
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(match_id)
        .bind(&req.evidence_blob_id)
        .bind(&req.evidence_summary)
        .bind(format!("{}_proposed", row.source))
        .execute(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_outcome_proposals(
        &self,
        match_id: uuid::Uuid,
    ) -> Result<Vec<crate::models::OutcomeProposalRecord>> {
        Ok(sqlx::query_as::<_, crate::models::OutcomeProposalRecord>(
            "SELECT * FROM outcome_proposals WHERE match_id = $1 ORDER BY created_at DESC",
        )
        .bind(match_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn list_admin_outcome_proposals(
        &self,
        query: &crate::models::AdminOutcomeProposalQuery,
    ) -> Result<Vec<crate::models::OutcomeProposalRecord>> {
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        let offset = query.offset.unwrap_or(0).max(0);
        let mut qb = sqlx::QueryBuilder::new("SELECT * FROM outcome_proposals WHERE 1=1");

        if let Some(ref status) = query.status {
            qb.push(" AND status = ").push_bind(status.clone());
        }
        if let Some(ref source) = query.source {
            qb.push(" AND source = ").push_bind(source.clone());
        }
        if let Some(ref match_id) = query.match_id {
            let match_uuid = uuid::Uuid::parse_str(match_id)
                .map_err(|e| anyhow::anyhow!("Invalid match_id: {}", e))?;
            qb.push(" AND match_id = ").push_bind(match_uuid);
        }

        qb.push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        Ok(qb
            .build_query_as::<crate::models::OutcomeProposalRecord>()
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn get_outcome_proposal(
        &self,
        proposal_id: uuid::Uuid,
    ) -> Result<Option<crate::models::OutcomeProposalRecord>> {
        Ok(sqlx::query_as::<_, crate::models::OutcomeProposalRecord>(
            "SELECT * FROM outcome_proposals WHERE id = $1",
        )
        .bind(proposal_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn review_outcome_proposal(
        &self,
        proposal_id: uuid::Uuid,
        decision: &str,
        reviewer_wallet: Option<&str>,
    ) -> Result<crate::models::OutcomeProposalRecord> {
        let status = match decision {
            "approve" => "approved",
            "reject" => "rejected",
            "dispute" => "disputed",
            other => anyhow::bail!("Unsupported decision: {}", other),
        };

        let row = sqlx::query_as::<_, crate::models::OutcomeProposalRecord>(
            r#"UPDATE outcome_proposals
               SET status = $2,
                   reviewed_at = NOW(),
                   reviewer_wallet = $3
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(proposal_id)
        .bind(status)
        .bind(reviewer_wallet)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query(
            r#"UPDATE matches
               SET result_status = $2,
                   verification_status = $3,
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(row.match_id)
        .bind(status)
        .bind(format!("{}_{}", row.source, status))
        .execute(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn create_walrus_artifact(
        &self,
        req: &crate::models::CreateWalrusArtifactRequest,
        stored: &crate::services::walrus::WalrusStoredBlob,
    ) -> Result<crate::models::WalrusArtifactRecord> {
        let match_id = match req.match_id.as_ref() {
            Some(id) => Some(
                uuid::Uuid::parse_str(id)
                    .map_err(|e| anyhow::anyhow!("Invalid match_id: {}", e))?,
            ),
            None => None,
        };
        let outcome_proposal_id = match req.outcome_proposal_id.as_ref() {
            Some(id) => Some(
                uuid::Uuid::parse_str(id)
                    .map_err(|e| anyhow::anyhow!("Invalid outcome_proposal_id: {}", e))?,
            ),
            None => None,
        };
        let metadata = req
            .metadata
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));

        let row = sqlx::query_as::<_, crate::models::WalrusArtifactRecord>(
            r#"INSERT INTO walrus_artifacts (
                blob_id, object_id, artifact_type, owner_wallet, match_id,
                outcome_proposal_id, content_type, size_bytes, aggregator_url,
                publisher_url, metadata
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            RETURNING *"#,
        )
        .bind(&stored.blob_id)
        .bind(&stored.object_id)
        .bind(&req.artifact_type)
        .bind(&req.owner_wallet)
        .bind(match_id)
        .bind(outcome_proposal_id)
        .bind(req.content_type.as_deref().unwrap_or("application/json"))
        .bind(stored.size_bytes as i64)
        .bind(&stored.aggregator_url)
        .bind(std::env::var("WALRUS_PUBLISHER_URL").ok())
        .bind(metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// List Walrus artifacts linked to a wager via `metadata->>'wager_address'`.
    pub async fn list_walrus_artifacts_for_wager(
        &self,
        wager_address: &str,
    ) -> Result<Vec<crate::models::WalrusArtifactRecord>> {
        Ok(sqlx::query_as::<_, crate::models::WalrusArtifactRecord>(
            r#"SELECT * FROM walrus_artifacts
               WHERE metadata->>'wager_address' = $1
               ORDER BY created_at DESC"#,
        )
        .bind(wager_address)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_walrus_artifact(
        &self,
        artifact_id: uuid::Uuid,
    ) -> Result<Option<crate::models::WalrusArtifactRecord>> {
        Ok(sqlx::query_as::<_, crate::models::WalrusArtifactRecord>(
            "SELECT * FROM walrus_artifacts WHERE id = $1",
        )
        .bind(artifact_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_agent_run_for_proposal(
        &self,
        match_id: uuid::Uuid,
        req: &crate::models::AgentOutcomeProposalRequest,
        proposal_id: uuid::Uuid,
        agent_id: Option<&str>,
        evidence_blob_id: Option<&str>,
        evidence_url: Option<&str>,
        verification_status: Option<&str>,
        verification_note: Option<&str>,
    ) -> Result<crate::models::AgentRunRecord> {
        let proposed_winner_uuid = match req.proposed_winner_opponent_id.as_ref() {
            Some(id) => Some(
                uuid::Uuid::parse_str(id)
                    .map_err(|e| anyhow::anyhow!("Invalid proposed_winner_opponent_id: {}", e))?,
            ),
            None => None,
        };
        let agent_name = req
            .agent_name
            .clone()
            .unwrap_or_else(|| "kombat-outcome-agent".to_string());
        let watch_sources = req
            .watch_sources
            .clone()
            .unwrap_or_else(|| serde_json::json!([]));
        let raw_output = req
            .raw_output
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));

        let row = sqlx::query_as::<_, crate::models::AgentRunRecord>(
            r#"INSERT INTO agent_runs (
                match_id, agent_name, agent_id, status, watch_sources, evidence_blob_id,
                evidence_url, outcome_proposal_id, proposed_winner_opponent_id,
                proposed_winner_name, confidence, summary, verification_status,
                verification_note, raw_output, started_at, completed_at
            ) VALUES (
                $1, $2, $3, 'completed', $4, $5, $6, $7, $8,
                $9, $10, $11, $12, $13, $14, NOW(), NOW()
            )
            RETURNING *"#,
        )
        .bind(match_id)
        .bind(agent_name)
        .bind(agent_id)
        .bind(watch_sources)
        .bind(evidence_blob_id)
        .bind(evidence_url)
        .bind(proposal_id)
        .bind(proposed_winner_uuid)
        .bind(&req.proposed_winner_name)
        .bind(req.confidence)
        .bind(&req.evidence_summary)
        .bind(verification_status)
        .bind(verification_note)
        .bind(raw_output)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Apply the result of agent outcome verification to a proposal and its
    /// match. `auto_verified` mirrors an approval; `pending_review` keeps the
    /// proposal awaiting a human decision.
    pub async fn apply_agent_verification(
        &self,
        proposal_id: uuid::Uuid,
        match_id: uuid::Uuid,
        source: &str,
        status: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE outcome_proposals
               SET status = $2,
                   reviewed_at = CASE WHEN $2 = 'auto_verified' THEN NOW() ELSE reviewed_at END,
                   reviewer_wallet = CASE WHEN $2 = 'auto_verified' THEN 'agent' ELSE reviewer_wallet END
               WHERE id = $1"#,
        )
        .bind(proposal_id)
        .bind(status)
        .execute(&self.pool)
        .await?;

        let result_status = if status == "auto_verified" {
            "approved"
        } else {
            "proposed"
        };
        sqlx::query(
            r#"UPDATE matches
               SET result_status = $2,
                   verification_status = $3,
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(match_id)
        .bind(result_status)
        .bind(format!("{}_{}", source, status))
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_admin_agent_runs(
        &self,
        query: &crate::models::AdminAgentRunQuery,
    ) -> Result<Vec<crate::models::AgentRunRecord>> {
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        let offset = query.offset.unwrap_or(0).max(0);
        let mut qb = sqlx::QueryBuilder::new("SELECT * FROM agent_runs WHERE 1=1");

        if let Some(ref status) = query.status {
            qb.push(" AND status = ").push_bind(status.clone());
        }
        if let Some(ref agent_name) = query.agent_name {
            qb.push(" AND agent_name = ").push_bind(agent_name.clone());
        }
        if let Some(ref agent_id) = query.agent_id {
            qb.push(" AND agent_id = ").push_bind(agent_id.clone());
        }
        if let Some(ref match_id) = query.match_id {
            let match_uuid = uuid::Uuid::parse_str(match_id)
                .map_err(|e| anyhow::anyhow!("Invalid match_id: {}", e))?;
            qb.push(" AND match_id = ").push_bind(match_uuid);
        }

        qb.push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        Ok(qb
            .build_query_as::<crate::models::AgentRunRecord>()
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn get_agent_run(
        &self,
        run_id: uuid::Uuid,
    ) -> Result<Option<crate::models::AgentRunRecord>> {
        Ok(sqlx::query_as::<_, crate::models::AgentRunRecord>(
            "SELECT * FROM agent_runs WHERE id = $1",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Get a match by ID with pool statistics
    pub async fn get_match_with_odds(
        &self,
        match_id: &str,
    ) -> Result<Option<crate::models::MatchWithOdds>> {
        let match_uuid = uuid::Uuid::parse_str(match_id)
            .map_err(|e| anyhow::anyhow!("Invalid match ID: {}", e))?;

        let match_record =
            sqlx::query_as::<_, crate::models::MatchRecord>("SELECT * FROM matches WHERE id = $1")
                .bind(match_uuid)
                .fetch_optional(&self.pool)
                .await?;

        let match_record = match match_record {
            Some(m) => m,
            None => return Ok(None),
        };

        // Get opponents with pool stats
        let opponents = self.get_opponents_with_pools(match_uuid).await?;

        let total_pool_usdc: i64 = opponents.iter().map(|o| o.pool_usdc).sum();
        let total_stakers: i64 = opponents.iter().map(|o| o.staker_count).sum();

        Ok(Some(crate::models::MatchWithOdds {
            match_info: match_record,
            opponents,
            total_pool_usdc,
            total_stakers,
        }))
    }

    /// Get a match by PandaScore ID
    pub async fn get_match_by_pandascore_id(
        &self,
        pandascore_id: i64,
    ) -> Result<Option<crate::models::MatchRecord>> {
        let row = sqlx::query_as::<_, crate::models::MatchRecord>(
            "SELECT * FROM matches WHERE pandascore_id = $1",
        )
        .bind(pandascore_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Matches that are past their scheduled time, still open, have a
    /// PandaScore id, and haven't had an outcome proposal submitted yet.
    /// `lookback_hours` caps how far back we look to avoid re-checking old
    /// settled matches on restart.
    pub async fn get_pollable_matches(
        &self,
        lookback_hours: i64,
    ) -> anyhow::Result<Vec<crate::models::MatchRecord>> {
        Ok(sqlx::query_as::<_, crate::models::MatchRecord>(
            r#"SELECT m.*
               FROM matches m
               WHERE m.pandascore_id IS NOT NULL
                 AND m.pandascore_status NOT IN ('finished', 'canceled')
                 AND m.result_status = 'pending'
                 AND m.scheduled_at IS NOT NULL
                 AND m.scheduled_at < NOW()
                 AND m.scheduled_at > NOW() - ($1 * interval '1 hour')
                 AND NOT EXISTS (
                     SELECT 1 FROM outcome_proposals op
                     WHERE op.match_id = m.id
                 )
               ORDER BY m.scheduled_at ASC
               LIMIT 50"#,
        )
        .bind(lookback_hours)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_match_opponents(
        &self,
        match_id: uuid::Uuid,
    ) -> Result<Vec<crate::models::MatchOpponentRecord>> {
        Ok(sqlx::query_as::<_, crate::models::MatchOpponentRecord>(
            "SELECT * FROM match_opponents WHERE match_id = $1 ORDER BY position ASC",
        )
        .bind(match_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Get opponents with pool statistics (single query with aggregates)
    async fn get_opponents_with_pools(
        &self,
        match_id: uuid::Uuid,
    ) -> Result<Vec<crate::models::OpponentWithPool>> {
        // Single query: get opponents with their pool stats via LEFT JOIN aggregate
        let rows: Vec<crate::models::OpponentWithPoolRow> = sqlx::query_as(
            r#"SELECT
                mo.*,
                COALESCE(s.pool_usdc, 0)::BIGINT AS pool_usdc,
                COALESCE(s.staker_count, 0)::BIGINT AS staker_count,
                COALESCE(tp.total_pool, 0)::BIGINT AS total_pool
            FROM match_opponents mo
            LEFT JOIN (
                SELECT opponent_id,
                       SUM(amount_usdc) AS pool_usdc,
                       COUNT(DISTINCT user_wallet) AS staker_count
                FROM pool_stakes
                WHERE status = 'active'
                GROUP BY opponent_id
            ) s ON s.opponent_id = mo.id
            CROSS JOIN (
                SELECT COALESCE(SUM(amount_usdc), 0) AS total_pool
                FROM pool_stakes
                WHERE match_id = $1 AND status = 'active'
            ) tp
            WHERE mo.match_id = $1
            ORDER BY mo.position"#,
        )
        .bind(match_id)
        .fetch_all(&self.pool)
        .await?;

        let result = rows
            .into_iter()
            .map(|row| {
                let pool_usdc = row.pool_usdc;
                let staker_count = row.staker_count;
                let total_pool = row.total_pool;

                let pool_percentage = if total_pool > 0 {
                    (pool_usdc as f64 / total_pool as f64) * 100.0
                } else {
                    50.0
                };

                let odds = if pool_usdc > 0 {
                    (total_pool as f64 / pool_usdc as f64).min(9999.0)
                } else if total_pool > 0 {
                    9999.0
                } else {
                    1.0
                };

                crate::models::OpponentWithPool {
                    opponent: row.into_opponent_record(),
                    pool_usdc,
                    pool_percentage,
                    odds,
                    staker_count,
                }
            })
            .collect();

        Ok(result)
    }

    /// List matches with filtering
    pub async fn list_matches(
        &self,
        query: &crate::models::MatchListQuery,
    ) -> Result<Vec<crate::models::MatchWithOdds>> {
        let limit = query.limit.unwrap_or(20).min(100);
        let offset = query.offset.unwrap_or(0);

        let mut qb = sqlx::QueryBuilder::new("SELECT * FROM matches WHERE 1=1");

        if let Some(ref status) = query.status {
            qb.push(" AND status = ").push_bind(status.clone());
        }

        if let Some(ref vg) = query.videogame {
            qb.push(" AND videogame_slug = ").push_bind(vg.clone());
        }

        if let Some(league_id) = query.league_id {
            qb.push(" AND league_id = ").push_bind(league_id);
        }

        if let Some(ref search) = query.search {
            let escaped = search
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            qb.push(" AND name ILIKE ")
                .push_bind(format!("%{}%", escaped));
        }

        qb.push(" ORDER BY scheduled_at ASC NULLS LAST LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        let matches = qb
            .build_query_as::<crate::models::MatchRecord>()
            .fetch_all(&self.pool)
            .await?;

        let mut result = Vec::with_capacity(matches.len());
        for m in matches {
            let opponents = self.get_opponents_with_pools(m.id).await?;
            let total_pool_usdc: i64 = opponents.iter().map(|o| o.pool_usdc).sum();
            let total_stakers: i64 = opponents.iter().map(|o| o.staker_count).sum();

            result.push(crate::models::MatchWithOdds {
                match_info: m,
                opponents,
                total_pool_usdc,
                total_stakers,
            });
        }

        Ok(result)
    }

    // ── Payment Intents ───────────────────────────────────────────────────────

    pub async fn create_payment_intent(
        &self,
        user_wallet: &str,
        network: &str,
        match_id: uuid::Uuid,
        opponent_id: uuid::Uuid,
        amount_usdc: i64,
        reserve_balance_usdc: i64,
        settlement_rule: &str,
        current_balance_usdc: i64,
        funding_shortfall_usdc: i64,
    ) -> Result<crate::models::PaymentIntentRecord> {
        let funding_shortfall_usdc = funding_shortfall_usdc.max(0);
        let status = if funding_shortfall_usdc > 0 {
            "requires_funding"
        } else {
            "ready_to_stake"
        };

        let row = sqlx::query_as::<_, crate::models::PaymentIntentRecord>(
            r#"INSERT INTO payment_intents (
                user_wallet, kind, status, network, match_id, opponent_id,
                amount_usdc, reserve_balance_usdc, settlement_rule,
                current_balance_usdc, funding_shortfall_usdc
            ) VALUES (
                $1, 'STAKE_TOURNAMENT', $2, $3, $4, $5,
                $6, $7, $8, $9, $10
            )
            RETURNING *"#,
        )
        .bind(user_wallet)
        .bind(status)
        .bind(network)
        .bind(match_id)
        .bind(opponent_id)
        .bind(amount_usdc)
        .bind(reserve_balance_usdc)
        .bind(settlement_rule)
        .bind(current_balance_usdc)
        .bind(funding_shortfall_usdc)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_payment_intent(
        &self,
        intent_id: uuid::Uuid,
    ) -> Result<Option<crate::models::PaymentIntentRecord>> {
        let row = sqlx::query_as::<_, crate::models::PaymentIntentRecord>(
            "SELECT * FROM payment_intents WHERE id = $1",
        )
        .bind(intent_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn update_payment_intent_funding(
        &self,
        intent_id: uuid::Uuid,
        current_balance_usdc: i64,
        funding_shortfall_usdc: i64,
    ) -> Result<crate::models::PaymentIntentRecord> {
        let funding_shortfall_usdc = funding_shortfall_usdc.max(0);
        let status = if funding_shortfall_usdc > 0 {
            "requires_funding"
        } else {
            "ready_to_stake"
        };

        let row = sqlx::query_as::<_, crate::models::PaymentIntentRecord>(
            r#"UPDATE payment_intents
               SET current_balance_usdc = $2,
                   funding_shortfall_usdc = $3,
                   status = CASE
                       WHEN status IN ('submitted', 'completed', 'cancelled', 'expired') THEN status
                       ELSE $4
                   END,
                   updated_at = NOW()
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(intent_id)
        .bind(current_balance_usdc)
        .bind(funding_shortfall_usdc)
        .bind(status)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    // ── Receipt Market Listings ──────────────────────────────────────────────

    pub async fn create_receipt_listing(
        &self,
        network: &str,
        seller_wallet: &str,
        receipt_id: &str,
        match_id: uuid::Uuid,
        opponent_id: uuid::Uuid,
        ask_amount_usdc: i64,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<crate::models::ReceiptMarketListingRecord> {
        let row = sqlx::query_as::<_, crate::models::ReceiptMarketListingRecord>(
            r#"INSERT INTO receipt_market_listings (
                network, seller_wallet, receipt_id, match_id, opponent_id,
                ask_amount_usdc, status, expires_at
            ) VALUES ($1, $2, $3, $4, $5, $6, 'draft', $7)
            RETURNING *"#,
        )
        .bind(network)
        .bind(seller_wallet)
        .bind(receipt_id)
        .bind(match_id)
        .bind(opponent_id)
        .bind(ask_amount_usdc)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_receipt_listing(
        &self,
        listing_id: uuid::Uuid,
    ) -> Result<Option<crate::models::ReceiptMarketListingRecord>> {
        let row = sqlx::query_as::<_, crate::models::ReceiptMarketListingRecord>(
            "SELECT * FROM receipt_market_listings WHERE id = $1",
        )
        .bind(listing_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_receipt_listings(
        &self,
        query: &crate::models::ReceiptListingQuery,
    ) -> Result<Vec<crate::models::ReceiptMarketListingRecord>> {
        let limit = query.limit.unwrap_or(20).clamp(1, 100);
        let offset = query.offset.unwrap_or(0).max(0);
        let mut qb = sqlx::QueryBuilder::new("SELECT * FROM receipt_market_listings WHERE 1=1");

        if let Some(ref match_id) = query.match_id {
            let match_uuid = uuid::Uuid::parse_str(match_id)
                .map_err(|e| anyhow::anyhow!("Invalid match_id: {}", e))?;
            qb.push(" AND match_id = ").push_bind(match_uuid);
        }

        if let Some(ref seller_wallet) = query.seller_wallet {
            qb.push(" AND seller_wallet = ")
                .push_bind(seller_wallet.clone());
        }

        if let Some(ref status) = query.status {
            qb.push(" AND status = ").push_bind(status.clone());
        } else {
            qb.push(" AND status IN ('draft', 'active')");
        }

        qb.push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        let rows = qb
            .build_query_as::<crate::models::ReceiptMarketListingRecord>()
            .fetch_all(&self.pool)
            .await?;

        Ok(rows)
    }

    pub async fn activate_receipt_listing(
        &self,
        listing_id: uuid::Uuid,
        listing_object_id: &str,
        listing_tx_hash: Option<&str>,
    ) -> Result<crate::models::ReceiptMarketListingRecord> {
        let row = sqlx::query_as::<_, crate::models::ReceiptMarketListingRecord>(
            r#"UPDATE receipt_market_listings
               SET listing_object_id = $2,
                   listing_tx_hash = $3,
                   status = 'active',
                   updated_at = NOW()
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(listing_id)
        .bind(listing_object_id)
        .bind(listing_tx_hash)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn mark_receipt_listing_sold(
        &self,
        listing_id: uuid::Uuid,
        buyer_wallet: &str,
        sale_tx_hash: Option<&str>,
    ) -> Result<crate::models::ReceiptMarketListingRecord> {
        let row = sqlx::query_as::<_, crate::models::ReceiptMarketListingRecord>(
            r#"UPDATE receipt_market_listings
               SET buyer_wallet = $2,
                   sale_tx_hash = $3,
                   status = 'sold',
                   updated_at = NOW()
               WHERE id = $1
                 AND status = 'active'
               RETURNING *"#,
        )
        .bind(listing_id)
        .bind(buyer_wallet)
        .bind(sale_tx_hash)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    // ── Pool Stakes ────────────────────────────────────────────────────────────

    /// Place a stake on a match outcome
    pub async fn place_stake(
        &self,
        match_id: &str,
        opponent_id: &str,
        user_wallet: &str,
        amount_usdc: i64,
    ) -> Result<crate::models::PoolStakeRecord> {
        let match_uuid = uuid::Uuid::parse_str(match_id)
            .map_err(|e| anyhow::anyhow!("Invalid match ID: {}", e))?;
        let opponent_uuid = uuid::Uuid::parse_str(opponent_id)
            .map_err(|e| anyhow::anyhow!("Invalid opponent ID: {}", e))?;

        // Verify match exists and is accepting stakes
        let match_record =
            sqlx::query_as::<_, crate::models::MatchRecord>("SELECT * FROM matches WHERE id = $1")
                .bind(match_uuid)
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Match not found"))?;

        if match_record.status != "upcoming" && match_record.status != "live" {
            anyhow::bail!(
                "Match is not accepting stakes (status: {})",
                match_record.status
            );
        }

        // Verify opponent belongs to this match
        let opponent = sqlx::query_as::<_, crate::models::MatchOpponentRecord>(
            "SELECT * FROM match_opponents WHERE id = $1 AND match_id = $2",
        )
        .bind(opponent_uuid)
        .bind(match_uuid)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Opponent not found for this match"))?;

        // Calculate current odds with two simple queries instead of full opponent stats
        let total_pool: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE match_id = $1 AND status = 'active'"
        )
        .bind(match_uuid)
        .fetch_one(&self.pool)
        .await?;

        let opponent_pool: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE opponent_id = $1 AND status = 'active'"
        )
        .bind(opponent_uuid)
        .fetch_one(&self.pool)
        .await?;

        let current_odds = if opponent_pool > 0 {
            (total_pool as f64 / opponent_pool as f64).min(9999.0)
        } else if total_pool > 0 {
            9999.0
        } else {
            1.0
        };

        // Insert stake
        let row = sqlx::query_as::<_, crate::models::PoolStakeRecord>(
            r#"INSERT INTO pool_stakes (
                match_id, opponent_id, user_wallet, amount_usdc, odds_at_stake, status
            ) VALUES ($1, $2, $3, $4, $5, 'active')
            RETURNING *"#,
        )
        .bind(match_uuid)
        .bind(opponent_uuid)
        .bind(user_wallet)
        .bind(amount_usdc)
        .bind(rust_decimal::Decimal::from_f64_retain(current_odds))
        .fetch_one(&self.pool)
        .await?;

        tracing::info!(
            "Stake placed: {} staked {} on {} for match {}",
            user_wallet,
            amount_usdc,
            opponent.name,
            match_record.name
        );

        Ok(row)
    }

    /// Calculate potential payout for a stake
    pub async fn calculate_payout(
        &self,
        match_id: &str,
        opponent_id: &str,
        amount_usdc: i64,
    ) -> Result<crate::models::PayoutCalculation> {
        let match_uuid = uuid::Uuid::parse_str(match_id)
            .map_err(|e| anyhow::anyhow!("Invalid match ID: {}", e))?;
        let opponent_uuid = uuid::Uuid::parse_str(opponent_id)
            .map_err(|e| anyhow::anyhow!("Invalid opponent ID: {}", e))?;

        // Get current pool totals
        let total_pool: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE match_id = $1 AND status = 'active'"
        )
        .bind(match_uuid)
        .fetch_one(&self.pool)
        .await?;

        let opponent_pool: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE opponent_id = $1 AND status = 'active'"
        )
        .bind(opponent_uuid)
        .fetch_one(&self.pool)
        .await?;

        // Calculate new totals after this stake
        let new_total = total_pool + amount_usdc;
        let new_opponent_pool = opponent_pool + amount_usdc;

        // Calculate odds after stake
        let current_odds = if new_opponent_pool > 0 {
            new_total as f64 / new_opponent_pool as f64
        } else {
            1.0
        };

        let min_payout = (amount_usdc as f64 * current_odds) as i64;
        let min_profit = min_payout - amount_usdc;
        let profit_percentage = if amount_usdc > 0 {
            (min_profit as f64 / amount_usdc as f64) * 100.0
        } else {
            0.0
        };

        // Check for warning conditions
        let warning = if total_pool == 0 {
            Some("You're the first staker! Odds will change as others stake.".to_string())
        } else if opponent_pool == 0 {
            Some(
                "No one has staked on this side yet. Odds are very high but may change."
                    .to_string(),
            )
        } else if total_pool - opponent_pool == 0 {
            Some(
                "No opposition yet. Your stake may be refunded if no one bets on the other side."
                    .to_string(),
            )
        } else {
            None
        };

        Ok(crate::models::PayoutCalculation {
            stake_amount_usdc: amount_usdc,
            current_odds,
            min_payout_usdc: min_payout,
            min_profit_usdc: min_profit,
            profit_percentage,
            warning,
        })
    }

    /// Resolve a match and process payouts (wrapped in a transaction)
    pub async fn resolve_match(
        &self,
        match_id: &str,
        winner_opponent_id: &str,
        pandascore_winner_id: Option<i32>,
        forfeit: bool,
    ) -> Result<crate::models::ResolveResult> {
        use crate::models::{PayoutEntry, ResolveResult};

        let match_uuid = uuid::Uuid::parse_str(match_id)
            .map_err(|e| anyhow::anyhow!("Invalid match ID: {}", e))?;
        let winner_uuid = uuid::Uuid::parse_str(winner_opponent_id)
            .map_err(|e| anyhow::anyhow!("Invalid winner ID: {}", e))?;

        let mut tx = self.pool.begin().await?;

        // Get match
        let match_record = sqlx::query_as::<_, crate::models::MatchRecord>(
            "SELECT * FROM matches WHERE id = $1 FOR UPDATE",
        )
        .bind(match_uuid)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Match not found"))?;

        // Get pool totals
        let total_pool: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE match_id = $1 AND status = 'active'"
        )
        .bind(match_uuid)
        .fetch_one(&mut *tx)
        .await?;

        if match_record.status == "completed" && total_pool == 0 {
            anyhow::bail!("Match already resolved");
        }

        let winner_pool: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE opponent_id = $1 AND status = 'active'"
        )
        .bind(winner_uuid)
        .fetch_one(&mut *tx)
        .await?;

        let loser_pool = total_pool - winner_pool;

        // Check edge cases
        if total_pool == 0 {
            // No stakes at all - just mark complete
            sqlx::query("UPDATE matches SET status = 'completed', winner_id = $2, forfeit = $3, updated_at = NOW() WHERE id = $1")
                .bind(match_uuid)
                .bind(pandascore_winner_id)
                .bind(forfeit)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(ResolveResult::Empty);
        }

        if loser_pool == 0 || winner_pool == 0 {
            // One-sided pool - refund everyone
            let refund_stakes: Vec<crate::models::PoolStakeRecord> = sqlx::query_as(
                "SELECT * FROM pool_stakes WHERE match_id = $1 AND status = 'active'",
            )
            .bind(match_uuid)
            .fetch_all(&mut *tx)
            .await?;

            let refunds: Vec<PayoutEntry> = refund_stakes
                .iter()
                .map(|s| PayoutEntry {
                    stake_id: s.id,
                    user_wallet: s.user_wallet.clone(),
                    amount_usdc: s.amount_usdc,
                })
                .collect();

            sqlx::query("UPDATE pool_stakes SET status = 'refunded', resolved_at = NOW() WHERE match_id = $1 AND status = 'active'")
                .bind(match_uuid)
                .execute(&mut *tx)
                .await?;

            sqlx::query("UPDATE matches SET status = 'refunded', winner_id = $2, forfeit = $3, updated_at = NOW() WHERE id = $1")
                .bind(match_uuid)
                .bind(pandascore_winner_id)
                .bind(forfeit)
                .execute(&mut *tx)
                .await?;

            tx.commit().await?;
            tracing::info!("Match {} resolved with refunds (one-sided pool)", match_id);
            return Ok(ResolveResult::Refunded(refunds));
        }

        // Calculate payouts for winners using integer arithmetic
        // Each winner gets: (their_stake * total_pool) / winner_pool
        // Use i128 to prevent overflow on multiplication
        let winner_stakes: Vec<crate::models::PoolStakeRecord> = sqlx::query_as(
            "SELECT * FROM pool_stakes WHERE opponent_id = $1 AND status = 'active'",
        )
        .bind(winner_uuid)
        .fetch_all(&mut *tx)
        .await?;

        let mut payouts = Vec::new();
        for stake in winner_stakes {
            let payout =
                ((stake.amount_usdc as i128 * total_pool as i128) / winner_pool as i128) as i64;
            sqlx::query("UPDATE pool_stakes SET status = 'won', payout_usdc = $2, resolved_at = NOW() WHERE id = $1")
                .bind(stake.id)
                .bind(payout)
                .execute(&mut *tx)
                .await?;
            payouts.push(PayoutEntry {
                stake_id: stake.id,
                user_wallet: stake.user_wallet.clone(),
                amount_usdc: payout,
            });
        }

        // Mark losing stakes
        sqlx::query("UPDATE pool_stakes SET status = 'lost', payout_usdc = 0, resolved_at = NOW() WHERE match_id = $1 AND opponent_id != $2 AND status = 'active'")
            .bind(match_uuid)
            .bind(winner_uuid)
            .execute(&mut *tx)
            .await?;

        // Update winner opponent
        sqlx::query("UPDATE match_opponents SET is_winner = TRUE WHERE id = $1")
            .bind(winner_uuid)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "UPDATE match_opponents SET is_winner = FALSE WHERE match_id = $1 AND id != $2",
        )
        .bind(match_uuid)
        .bind(winner_uuid)
        .execute(&mut *tx)
        .await?;

        // Update match
        sqlx::query("UPDATE matches SET status = 'completed', winner_id = $2, forfeit = $3, pandascore_status = 'finished', updated_at = NOW() WHERE id = $1")
            .bind(match_uuid)
            .bind(pandascore_winner_id)
            .bind(forfeit)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        tracing::info!(
            "Match {} resolved: winner pool {}, loser pool {}, total {}",
            match_id,
            winner_pool,
            loser_pool,
            total_pool
        );

        Ok(ResolveResult::Resolved(payouts))
    }

    /// Cancel a match and refund all stakes. Returns the list of refund entries
    /// for on-chain settlement.
    pub async fn cancel_match(&self, match_id: &str) -> Result<Vec<crate::models::PayoutEntry>> {
        let match_uuid = uuid::Uuid::parse_str(match_id)
            .map_err(|e| anyhow::anyhow!("Invalid match ID: {}", e))?;

        // Fetch active stakes before updating
        let stakes: Vec<crate::models::PoolStakeRecord> =
            sqlx::query_as("SELECT * FROM pool_stakes WHERE match_id = $1 AND status = 'active'")
                .bind(match_uuid)
                .fetch_all(&self.pool)
                .await?;

        let refunds: Vec<crate::models::PayoutEntry> = stakes
            .iter()
            .map(|s| crate::models::PayoutEntry {
                stake_id: s.id,
                user_wallet: s.user_wallet.clone(),
                amount_usdc: s.amount_usdc,
            })
            .collect();

        sqlx::query("UPDATE pool_stakes SET status = 'refunded', resolved_at = NOW() WHERE match_id = $1 AND status = 'active'")
            .bind(match_uuid)
            .execute(&self.pool)
            .await?;

        sqlx::query("UPDATE matches SET status = 'cancelled', pandascore_status = 'canceled', updated_at = NOW() WHERE id = $1")
            .bind(match_uuid)
            .execute(&self.pool)
            .await?;

        tracing::info!(
            "Match {} cancelled, {} stakes refunded",
            match_id,
            refunds.len()
        );
        Ok(refunds)
    }

    /// Record on-chain payout/refund tx hash for a stake
    pub async fn set_payout_tx_hash(&self, stake_id: uuid::Uuid, tx_hash: &str) -> Result<()> {
        sqlx::query("UPDATE pool_stakes SET payout_tx_hash = $2 WHERE id = $1")
            .bind(stake_id)
            .bind(tx_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_stakes_by_match_status(
        &self,
        match_id: &str,
        status: &str,
    ) -> Result<Vec<crate::models::PoolStakeRecord>> {
        let match_uuid = uuid::Uuid::parse_str(match_id)
            .map_err(|e| anyhow::anyhow!("Invalid match ID: {}", e))?;
        let rows = sqlx::query_as::<_, crate::models::PoolStakeRecord>(
            "SELECT * FROM pool_stakes WHERE match_id = $1 AND status = $2",
        )
        .bind(match_uuid)
        .bind(status)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Get user's stakes (single JOIN query)
    pub async fn get_user_stakes(
        &self,
        wallet: &str,
        query: &crate::models::StakeListQuery,
    ) -> Result<Vec<crate::models::StakeWithMatch>> {
        let limit = query.limit.unwrap_or(20).min(100);
        let offset = query.offset.unwrap_or(0);

        let rows: Vec<crate::models::StakeWithMatchRow> = sqlx::query_as(
            r#"SELECT
                ps.*,
                m.name AS match_name,
                m.status AS match_status,
                mo.name AS opponent_name,
                mo.image_url AS opponent_image_url,
                m.videogame_name,
                m.scheduled_at
            FROM pool_stakes ps
            JOIN matches m ON m.id = ps.match_id
            JOIN match_opponents mo ON mo.id = ps.opponent_id
            WHERE ps.user_wallet = $1
            ORDER BY ps.created_at DESC
            LIMIT $2 OFFSET $3"#,
        )
        .bind(wallet)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let result = rows
            .into_iter()
            .map(|row| row.into_stake_with_match())
            .collect();
        Ok(result)
    }

    /// Get user's stake stats
    pub async fn get_user_stake_stats(
        &self,
        wallet: &str,
    ) -> Result<crate::models::UserStakeStats> {
        let active_stakes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM pool_stakes WHERE user_wallet = $1 AND status = 'active'",
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let total_staked: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE user_wallet = $1",
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let total_won: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(payout_usdc), 0)::BIGINT FROM pool_stakes WHERE user_wallet = $1 AND status = 'won'"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let total_lost: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE user_wallet = $1 AND status = 'lost'"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let win_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM pool_stakes WHERE user_wallet = $1 AND status = 'won'",
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let loss_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM pool_stakes WHERE user_wallet = $1 AND status = 'lost'",
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        Ok(crate::models::UserStakeStats {
            active_stakes,
            total_staked_usdc: total_staked,
            total_won_usdc: total_won,
            total_lost_usdc: total_lost,
            win_count,
            loss_count,
        })
    }

    pub async fn get_locked_in_kombats_usdc(&self, wallet: &str) -> Result<i64> {
        let locked: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE user_wallet = $1 AND status = 'active'",
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        Ok(locked)
    }

    pub async fn list_wallet_transactions(
        &self,
        wallet: &str,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<crate::models::WalletTransactionItem>> {
        let limit = limit.unwrap_or(20).min(100);
        let offset = offset.unwrap_or(0);

        let transactions: Vec<crate::models::WalletTransactionItem> = sqlx::query_as(
            r#"
            WITH stake_events AS (
                SELECT
                    ps.id::TEXT || ':stake' AS id,
                    CASE
                        WHEN ps.status = 'active' THEN 'stake_locked'
                        ELSE 'stake_placed'
                    END AS kind,
                    'Stake locked · ' || m.name AS title,
                    mo.name AS subtitle,
                    -ps.amount_usdc AS amount_usdc,
                    'out' AS direction,
                    ps.status AS status,
                    ps.stake_tx_hash AS tx_hash,
                    ps.created_at AS occurred_at
                FROM pool_stakes ps
                JOIN matches m ON m.id = ps.match_id
                JOIN match_opponents mo ON mo.id = ps.opponent_id
                WHERE ps.user_wallet = $1
            ),
            resolution_events AS (
                SELECT
                    ps.id::TEXT || ':resolution' AS id,
                    CASE
                        WHEN ps.status = 'won' THEN 'won'
                        WHEN ps.status = 'lost' THEN 'lost'
                        WHEN ps.status = 'refunded' THEN 'refunded'
                        ELSE ps.status
                    END AS kind,
                    CASE
                        WHEN ps.status = 'won' THEN 'Won · ' || m.name
                        WHEN ps.status = 'lost' THEN 'Lost · ' || m.name
                        WHEN ps.status = 'refunded' THEN 'Refunded · ' || m.name
                        ELSE initcap(ps.status) || ' · ' || m.name
                    END AS title,
                    mo.name AS subtitle,
                    CASE
                        WHEN ps.status = 'won' THEN COALESCE(ps.payout_usdc, 0)
                        WHEN ps.status = 'refunded' THEN ps.amount_usdc
                        WHEN ps.status = 'lost' THEN -ps.amount_usdc
                        ELSE 0
                    END AS amount_usdc,
                    CASE
                        WHEN ps.status IN ('won', 'refunded') THEN 'in'
                        WHEN ps.status = 'lost' THEN 'out'
                        ELSE 'neutral'
                    END AS direction,
                    ps.status AS status,
                    COALESCE(ps.payout_tx_hash, ps.stake_tx_hash) AS tx_hash,
                    COALESCE(ps.resolved_at, ps.created_at) AS occurred_at
                FROM pool_stakes ps
                JOIN matches m ON m.id = ps.match_id
                JOIN match_opponents mo ON mo.id = ps.opponent_id
                WHERE ps.user_wallet = $1
                  AND ps.status IN ('won', 'lost', 'refunded')
            )
            SELECT * FROM (
                SELECT * FROM stake_events
                UNION ALL
                SELECT * FROM resolution_events
            ) events
            ORDER BY occurred_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(wallet)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(transactions)
    }

    /// Update match status (for syncing with PandaScore)
    pub async fn update_match_status(&self, match_id: &str, pandascore_status: &str) -> Result<()> {
        let match_uuid = uuid::Uuid::parse_str(match_id)
            .map_err(|e| anyhow::anyhow!("Invalid match ID: {}", e))?;

        let status = match pandascore_status {
            "not_started" => "upcoming",
            "running" => "live",
            "finished" => "completed",
            "canceled" => "cancelled",
            "postponed" => "upcoming",
            _ => "upcoming",
        };

        sqlx::query("UPDATE matches SET pandascore_status = $2, status = $3, updated_at = NOW() WHERE id = $1")
            .bind(match_uuid)
            .bind(pandascore_status)
            .bind(status)
            .execute(&self.pool)
            .await?;

        Ok(())
    }
    fn enrich_wager_row(
        row: WagerWithUsersRow,
        context_wallet: Option<&str>,
    ) -> crate::models::WagerDetailResponse {
        let expiry_unit = if row.expiry_ts < 1_000_000_000_000 {
            "seconds"
        } else {
            "milliseconds"
        };
        let expiry_ms = if expiry_unit == "seconds" {
            row.expiry_ts.saturating_mul(1000)
        } else {
            row.expiry_ts
        };
        let address_format = if row.on_chain_address.starts_with("0x") {
            "sui"
        } else {
            "legacy"
        };
        let is_legacy = expiry_unit == "seconds" || address_format != "sui";

        let wager = WagerRecord {
            id: row.id,
            on_chain_address: row.on_chain_address,
            wager_id: row.wager_id,
            initiator: row.initiator,
            challenger: row.challenger,
            stake_usdc: row.stake_usdc,
            description: row.description,
            status: row.status,
            resolution_source: row.resolution_source,
            resolver: row.resolver,
            expiry_ts: row.expiry_ts,
            created_at: row.created_at,
            resolved_at: row.resolved_at,
            winner: row.winner,
            protocol_fee_bps: row.protocol_fee_bps,
            oracle_feed: row.oracle_feed,
            oracle_target: row.oracle_target,
            dispute_opened_at: row.dispute_opened_at,
            dispute_opener: row.dispute_opener,
            initiator_option: row.initiator_option,
            creator_declared_winner: row.creator_declared_winner,
            challenger_declared_winner: row.challenger_declared_winner,
            resolution_error: row.resolution_error.clone(),
            resolution_attempted_at: row.resolution_attempted_at,
        };

        let challenger_option = wager.initiator_option.as_ref().map(|opt| {
            if opt.to_lowercase() == "yes" {
                "no".to_string()
            } else {
                "yes".to_string()
            }
        });

        let (opponent_wallet, opponent_name, opponent_avatar) = match context_wallet {
            Some(wallet) if wallet == wager.initiator => (
                wager.challenger.clone(),
                row.challenger_name.clone(),
                row.challenger_avatar.clone(),
            ),
            Some(wallet) if wager.challenger.as_deref() == Some(wallet) => (
                Some(wager.initiator.clone()),
                row.initiator_name.clone(),
                row.initiator_avatar.clone(),
            ),
            _ => (None, None, None),
        };
        let resolution_error = wager.resolution_error.clone();
        let resolution_attempted_at = wager.resolution_attempted_at;

        crate::models::WagerDetailResponse {
            wager,
            initiator_name: row.initiator_name,
            initiator_avatar: row.initiator_avatar,
            challenger_name: row.challenger_name,
            challenger_avatar: row.challenger_avatar,
            challenger_option,
            opponent_wallet,
            opponent_name,
            opponent_avatar,
            expiry_ms,
            expiry_unit: expiry_unit.to_string(),
            address_format: address_format.to_string(),
            is_legacy,
            resolution_error,
            resolution_attempted_at,
        }
    }

    async fn fetch_wagers_with_users(
        &self,
        filter_sql: &str,
        context_wallet: Option<&str>,
        bind_wallet: Option<&str>,
        status: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<crate::models::WagerDetailResponse>> {
        let sql = format!(
            r#"SELECT
                w.id, w.on_chain_address, w.wager_id, w.initiator, w.challenger,
                w.stake_usdc, w.description, w.status, w.resolution_source,
                w.resolver, w.expiry_ts, w.created_at, w.resolved_at, w.winner,
                w.protocol_fee_bps, w.oracle_feed, w.oracle_target,
                w.dispute_opened_at, w.dispute_opener, w.initiator_option,
                w.creator_declared_winner, w.challenger_declared_winner,
                w.resolution_error, w.resolution_attempted_at,
                ui.display_name AS initiator_name,
                ui.avatar_url AS initiator_avatar,
                uc.display_name AS challenger_name,
                uc.avatar_url AS challenger_avatar
               FROM wagers w
               LEFT JOIN users ui ON ui.wallet_address = w.initiator
               LEFT JOIN users uc ON uc.wallet_address = w.challenger
               WHERE {filter_sql}
               ORDER BY w.created_at DESC
               LIMIT $3 OFFSET $4"#,
        );

        let mut query = sqlx::query_as::<_, WagerWithUsersRow>(&sql);

        if let Some(wallet) = bind_wallet {
            query = query.bind(wallet);
        } else {
            query = query.bind(Option::<String>::None);
        }

        if let Some(status) = status {
            query = query.bind(status);
        } else {
            query = query.bind(Option::<String>::None);
        }

        let rows = query.bind(limit).bind(offset).fetch_all(&self.pool).await?;

        Ok(rows
            .into_iter()
            .map(|row| Self::enrich_wager_row(row, context_wallet))
            .collect())
    }
}
