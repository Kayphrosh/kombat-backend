// app/src/services/db.rs
//! PostgreSQL persistence layer using sqlx.
//! Indexes on-chain wager state for fast queryability.
//! All queries use runtime-checked sqlx (no compile-time DB required).

use anyhow::Result;
use sqlx::{PgPool, postgres::PgPoolOptions};
use crate::models::{WagerRecord, WagerListQuery, UserRecord, UpdateProfileRequest, NotificationRecord, NonceRecord};
use serde_json::Value as JsonValue;

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
        let wager = self.get_wager_by_address(address).await?;
        let wager = match wager {
            Some(w) => w,
            None => return Ok(None),
        };

        // Look up initiator user info
        let initiator_user = self.get_user(&wager.initiator).await?;
        let (initiator_name, initiator_avatar) = match initiator_user {
            Some(u) => (u.display_name, u.avatar_url),
            None => (None, None),
        };

        // Look up challenger user info (if present)
        let (challenger_name, challenger_avatar) = if let Some(ref ch) = wager.challenger {
            match self.get_user(ch).await? {
                Some(u) => (u.display_name, u.avatar_url),
                None => (None, None),
            }
        } else {
            (None, None)
        };

        // Compute challenger option (opposite of initiator)
        let challenger_option = wager.initiator_option.as_ref().map(|opt| {
            if opt.to_lowercase() == "yes" { "no".to_string() } else { "yes".to_string() }
        });

        Ok(Some(crate::models::WagerDetailResponse {
            wager,
            initiator_name,
            initiator_avatar,
            challenger_name,
            challenger_avatar,
            challenger_option,
        }))
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
}