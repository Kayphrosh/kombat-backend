

use anyhow::Result;
use sqlx::{PgPool, postgres::PgPoolOptions};
use crate::models::{WagerRecord, WagerListQuery, UserRecord, UpdateProfileRequest, NotificationRecord, NonceRecord};
use serde_json::Value as JsonValue;

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
    pub initiator_name: Option<String>,
    pub initiator_avatar: Option<String>,
    pub challenger_name: Option<String>,
    pub challenger_avatar: Option<String>,
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

    pub async fn create_notification(&self, user_wallet: &str, kind: &str, payload: Option<JsonValue>) -> Result<()> {
        sqlx::query(
            "INSERT INTO notifications (user_wallet, kind, payload) VALUES ($1, $2, $3)"
        )
        .bind(user_wallet)
        .bind(kind)
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Nonce (auth) ──────────────────────────────────────────────────────

    pub async fn insert_nonce(&self, wallet: &str, nonce: &str, expires_at: chrono::DateTime<chrono::Utc>) -> Result<()> {
        sqlx::query(
            "INSERT INTO nonces (wallet, nonce, expires_at) VALUES ($1, $2, $3)"
        )
        .bind(wallet)
        .bind(nonce)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn consume_nonce(&self, wallet: &str, nonce: &str) -> Result<Option<NonceRecord>> {
        let row = sqlx::query_as::<_, NonceRecord>(
            r#"SELECT id, wallet, nonce, used, expires_at, created_at FROM nonces
               WHERE wallet = $1 AND nonce = $2 AND used = FALSE AND expires_at > NOW() LIMIT 1"#,
        )
        .bind(wallet)
        .bind(nonce)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(r) = &row {
            sqlx::query("UPDATE nonces SET used = TRUE WHERE id = $1")
                .bind(r.id)
                .execute(&self.pool)
                .await?;
        }

        Ok(row)
    }

    pub async fn get_latest_unused_nonce(&self, wallet: &str) -> Result<Option<NonceRecord>> {
        let row = sqlx::query_as::<_, NonceRecord>(
            r#"SELECT id, wallet, nonce, used, expires_at, created_at FROM nonces
               WHERE wallet = $1 AND used = FALSE AND expires_at > NOW() ORDER BY created_at DESC LIMIT 1"#,
        )
        .bind(wallet)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_notifications_for_user(&self, wallet: &str, limit: i64, offset: i64) -> Result<Vec<NotificationRecord>> {
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

    pub async fn mark_notification_read(&self, id: &str) -> Result<()> {
        let uuid = uuid::Uuid::parse_str(id)
            .map_err(|e| anyhow::anyhow!("Invalid notification ID: {}", e))?;
        sqlx::query(
            "UPDATE notifications SET is_read = TRUE WHERE id = $1"
        )
        .bind(uuid)
        .execute(&self.pool)
        .await?;
        Ok(())
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
                creator_declared_winner, challenger_declared_winner
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                $21, $22
            )
            ON CONFLICT (on_chain_address) DO UPDATE SET
                challenger       = EXCLUDED.challenger,
                status           = EXCLUDED.status,
                resolved_at      = EXCLUDED.resolved_at,
                winner           = EXCLUDED.winner,
                dispute_opened_at = EXCLUDED.dispute_opened_at,
                dispute_opener   = EXCLUDED.dispute_opener
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
                creator_declared_winner, challenger_declared_winner
              FROM wagers WHERE on_chain_address = $1"#,
        )
        .bind(address)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Get a wager with participant user info (display names, avatars)
    pub async fn get_wager_with_users(&self, address: &str) -> Result<Option<crate::models::WagerDetailResponse>> {
        let rows = sqlx::query_as::<_, WagerWithUsersRow>(
            r#"SELECT
                w.id, w.on_chain_address, w.wager_id, w.initiator, w.challenger,
                w.stake_usdc, w.description, w.status, w.resolution_source,
                w.resolver, w.expiry_ts, w.created_at, w.resolved_at, w.winner,
                w.protocol_fee_bps, w.oracle_feed, w.oracle_target,
                w.dispute_opened_at, w.dispute_opener, w.initiator_option,
                w.creator_declared_winner, w.challenger_declared_winner,
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
        let limit  = q.limit.unwrap_or(20).min(100);
        let offset = q.offset.unwrap_or(0);

        let mut qb = sqlx::QueryBuilder::new(
            r#"SELECT id, on_chain_address, wager_id, initiator, challenger,
               stake_usdc, description, status, resolution_source,
               resolver, expiry_ts, created_at, resolved_at, winner,
               protocol_fee_bps, oracle_feed, oracle_target,
               dispute_opened_at, dispute_opener, initiator_option,
               creator_declared_winner, challenger_declared_winner
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
                ).await
            }
            (None, Some(challenger), status) => {
                self.fetch_wagers_with_users(
                    "w.challenger = $1 AND ($2::text IS NULL OR w.status = $2)",
                    context_wallet,
                    Some(challenger),
                    status.as_deref(),
                    limit,
                    offset,
                ).await
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
        ).await
    }

    pub async fn update_wager_status(&self, address: &str, status: &str) -> Result<()> {
        sqlx::query(
            "UPDATE wagers SET status = $1 WHERE on_chain_address = $2"
        )
        .bind(status)
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

        let query = format!(
            "UPDATE wagers SET {} = $1 WHERE on_chain_address = $2",
            col
        );
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
                    "UPDATE wagers SET status = 'resolved', winner = $1, resolved_at = NOW() WHERE on_chain_address = $2"
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
        sqlx::query("DELETE FROM nonces WHERE wallet = $1")
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
    pub async fn record_wager_result(&self, winner_wallet: &str, loser_wallet: Option<&str>) -> Result<()> {
        sqlx::query(
            "UPDATE users SET wins = wins + 1, updated_at = NOW() WHERE wallet_address = $1"
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

        tracing::info!("Recorded wager result — winner: {}, loser: {:?}", winner_wallet, loser_wallet);
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

    pub async fn get_notification_settings(&self, wallet: &str) -> Result<crate::models::NotificationSettings> {
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
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT expo_token FROM push_tokens WHERE wallet_address = $1"
        )
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
    pub async fn upsert_match(&self, req: &crate::models::CreateMatchRequest) -> Result<crate::models::MatchRecord> {
        // Parse optional timestamps
        let scheduled_at = req.scheduled_at.as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        let begin_at = req.begin_at.as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        let end_at = req.end_at.as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let pandascore_status = req.pandascore_status.clone().unwrap_or_else(|| "not_started".to_string());
        
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
            self.upsert_match_opponent(row.id, opponent, i as i16).await?;
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

    /// Get a match by ID with pool statistics
    pub async fn get_match_with_odds(&self, match_id: &str) -> Result<Option<crate::models::MatchWithOdds>> {
        let match_uuid = uuid::Uuid::parse_str(match_id)
            .map_err(|e| anyhow::anyhow!("Invalid match ID: {}", e))?;

        let match_record = sqlx::query_as::<_, crate::models::MatchRecord>(
            "SELECT * FROM matches WHERE id = $1"
        )
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
    pub async fn get_match_by_pandascore_id(&self, pandascore_id: i64) -> Result<Option<crate::models::MatchRecord>> {
        let row = sqlx::query_as::<_, crate::models::MatchRecord>(
            "SELECT * FROM matches WHERE pandascore_id = $1"
        )
        .bind(pandascore_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Get opponents with pool statistics (single query with aggregates)
    async fn get_opponents_with_pools(&self, match_id: uuid::Uuid) -> Result<Vec<crate::models::OpponentWithPool>> {
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

        let result = rows.into_iter().map(|row| {
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
        }).collect();

        Ok(result)
    }

    /// List matches with filtering
    pub async fn list_matches(&self, query: &crate::models::MatchListQuery) -> Result<Vec<crate::models::MatchWithOdds>> {
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
            let escaped = search.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            qb.push(" AND name ILIKE ").push_bind(format!("%{}%", escaped));
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
        let match_record = sqlx::query_as::<_, crate::models::MatchRecord>(
            "SELECT * FROM matches WHERE id = $1"
        )
        .bind(match_uuid)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Match not found"))?;

        if match_record.status != "upcoming" && match_record.status != "live" {
            anyhow::bail!("Match is not accepting stakes (status: {})", match_record.status);
        }

        // Verify opponent belongs to this match
        let opponent = sqlx::query_as::<_, crate::models::MatchOpponentRecord>(
            "SELECT * FROM match_opponents WHERE id = $1 AND match_id = $2"
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
            user_wallet, amount_usdc, opponent.name, match_record.name
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
            Some("No one has staked on this side yet. Odds are very high but may change.".to_string())
        } else if total_pool - opponent_pool == 0 {
            Some("No opposition yet. Your stake may be refunded if no one bets on the other side.".to_string())
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
            "SELECT * FROM matches WHERE id = $1 FOR UPDATE"
        )
        .bind(match_uuid)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Match not found"))?;

        if match_record.status == "completed" {
            anyhow::bail!("Match already resolved");
        }

        // Get pool totals
        let total_pool: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE match_id = $1 AND status = 'active'"
        )
        .bind(match_uuid)
        .fetch_one(&mut *tx)
        .await?;

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
                "SELECT * FROM pool_stakes WHERE match_id = $1 AND status = 'active'"
            )
            .bind(match_uuid)
            .fetch_all(&mut *tx)
            .await?;

            let refunds: Vec<PayoutEntry> = refund_stakes.iter().map(|s| PayoutEntry {
                stake_id: s.id,
                user_wallet: s.user_wallet.clone(),
                amount_usdc: s.amount_usdc,
            }).collect();

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
            "SELECT * FROM pool_stakes WHERE opponent_id = $1 AND status = 'active'"
        )
        .bind(winner_uuid)
        .fetch_all(&mut *tx)
        .await?;

        let mut payouts = Vec::new();
        for stake in winner_stakes {
            let payout = ((stake.amount_usdc as i128 * total_pool as i128) / winner_pool as i128) as i64;
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

        sqlx::query("UPDATE match_opponents SET is_winner = FALSE WHERE match_id = $1 AND id != $2")
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
            match_id, winner_pool, loser_pool, total_pool
        );

        Ok(ResolveResult::Resolved(payouts))
    }

    /// Cancel a match and refund all stakes. Returns the list of refund entries
    /// for on-chain settlement.
    pub async fn cancel_match(&self, match_id: &str) -> Result<Vec<crate::models::PayoutEntry>> {
        let match_uuid = uuid::Uuid::parse_str(match_id)
            .map_err(|e| anyhow::anyhow!("Invalid match ID: {}", e))?;

        // Fetch active stakes before updating
        let stakes: Vec<crate::models::PoolStakeRecord> = sqlx::query_as(
            "SELECT * FROM pool_stakes WHERE match_id = $1 AND status = 'active'"
        )
        .bind(match_uuid)
        .fetch_all(&self.pool)
        .await?;

        let refunds: Vec<crate::models::PayoutEntry> = stakes.iter().map(|s| crate::models::PayoutEntry {
            stake_id: s.id,
            user_wallet: s.user_wallet.clone(),
            amount_usdc: s.amount_usdc,
        }).collect();

        sqlx::query("UPDATE pool_stakes SET status = 'refunded', resolved_at = NOW() WHERE match_id = $1 AND status = 'active'")
            .bind(match_uuid)
            .execute(&self.pool)
            .await?;

        sqlx::query("UPDATE matches SET status = 'cancelled', pandascore_status = 'canceled', updated_at = NOW() WHERE id = $1")
            .bind(match_uuid)
            .execute(&self.pool)
            .await?;

        tracing::info!("Match {} cancelled, {} stakes refunded", match_id, refunds.len());
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

        let result = rows.into_iter().map(|row| row.into_stake_with_match()).collect();
        Ok(result)
    }

    /// Get user's stake stats
    pub async fn get_user_stake_stats(&self, wallet: &str) -> Result<crate::models::UserStakeStats> {
        let active_stakes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM pool_stakes WHERE user_wallet = $1 AND status = 'active'"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let total_staked: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_usdc), 0)::BIGINT FROM pool_stakes WHERE user_wallet = $1"
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
            "SELECT COUNT(*)::BIGINT FROM pool_stakes WHERE user_wallet = $1 AND status = 'won'"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let loss_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM pool_stakes WHERE user_wallet = $1 AND status = 'lost'"
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
        };

        let challenger_option = wager.initiator_option.as_ref().map(|opt| {
            if opt.to_lowercase() == "yes" { "no".to_string() } else { "yes".to_string() }
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

        let rows = query
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
